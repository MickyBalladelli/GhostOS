#!/usr/bin/env python3
"""Validate the SynOS guest-compatibility changelog contract."""

from __future__ import annotations

import argparse
import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parent.parent
CATEGORIES = (
    "Guest-visible",
    "Snapshot and migration",
    "Disk formats",
    "Tooling and documentation",
)
REQUIRED_FIELDS = ("Impact:", "Compatibility:", "Evidence:")


def unreleased_section(text: str) -> str:
    match = re.search(r"^## \[Unreleased\]\s*$", text, re.MULTILINE)
    if match is None:
        raise ValueError("CHANGELOG.md must contain a ## [Unreleased] section")
    remainder = text[match.end() :]
    next_heading = re.search(r"^## (?!#)", remainder, re.MULTILINE)
    return remainder[: next_heading.start()] if next_heading else remainder


def category_sections(section: str) -> dict[str, str]:
    matches = list(re.finditer(r"^### (.+?)\s*$", section, re.MULTILINE))
    found: dict[str, str] = {}
    for index, match in enumerate(matches):
        name = match.group(1).strip()
        end = matches[index + 1].start() if index + 1 < len(matches) else len(section)
        if name in CATEGORIES:
            found[name] = section[match.end() : end]
    missing = [name for name in CATEGORIES if name not in found]
    if missing:
        raise ValueError(f"Unreleased section is missing category headings: {', '.join(missing)}")
    return found


def entries(category: str, body: str) -> list[tuple[int, str]]:
    lines = body.splitlines()
    result: list[tuple[int, str]] = []
    for index, line in enumerate(lines):
        if line.startswith("- "):
            result.append((index, line[2:].strip()))
    if not result:
        raise ValueError(f"{category} must contain a bullet or `None yet.`")
    return result


def validate_entry(category: str, body: str, index: int, value: str) -> None:
    if re.search(r"\bnone yet\b", value, re.IGNORECASE):
        return
    lines = body.splitlines()
    next_entry = next(
        (position for position in range(index + 1, len(lines)) if lines[position].startswith("- ")),
        len(lines),
    )
    details = "\n".join(lines[index:next_entry])
    missing = [field for field in REQUIRED_FIELDS if field not in details]
    if category in {"Snapshot and migration", "Disk formats"} and "Migration:" not in details:
        missing.append("Migration:")
    if missing:
        raise ValueError(f"{category} entry `{value}` is missing {', '.join(missing)}")


def validate(path: pathlib.Path) -> None:
    text = path.read_text()
    section = unreleased_section(text)
    sections = category_sections(section)
    for category, body in sections.items():
        for index, value in entries(category, body):
            validate_entry(category, body, index, value)


def validate_changed_files(path: pathlib.Path, changed_files: list[str]) -> None:
    if not changed_files:
        return
    section = unreleased_section(path.read_text())
    sections = category_sections(section)
    required: set[str] = set()
    for value in changed_files:
        normalized = value.replace("\\", "/")
        if normalized.startswith("virtual_machine/src/snapshot") or normalized.startswith(
            "virtual_machine/src/migration"
        ):
            required.add("Snapshot and migration")
        elif normalized.startswith("virtual_machine/src/devices/storage/"):
            required.add("Disk formats")
        elif normalized.startswith(
            (
                "virtual_machine/src/cpu/",
                "virtual_machine/src/boot/",
                "virtual_machine/src/firmware/",
                "virtual_machine/src/devices/",
                "virtual_machine/src/terminal",
                "virtual_machine/src/input",
                "virtual_machine/src/memory/",
                "virtual_machine/src/lib.rs",
                "virtual_machine/src/main.rs",
            )
        ):
            required.add("Guest-visible")
    for category in required:
        if not any(not re.search(r"\bnone yet\b", value, re.IGNORECASE) for _, value in entries(category, sections[category])):
            raise ValueError(f"changed files require a real [Unreleased] entry under {category}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", nargs="?", type=pathlib.Path, default=ROOT / "CHANGELOG.md")
    parser.add_argument(
        "--changed-file",
        action="append",
        default=[],
        help="changed path to classify and require a matching Unreleased entry",
    )
    args = parser.parse_args()
    path = args.path.expanduser().resolve()
    try:
        if not path.is_file():
            raise ValueError(f"changelog does not exist: {path}")
        validate(path)
        validate_changed_files(path, args.changed_file)
    except (OSError, ValueError) as error:
        print(f"changelog validation failed: {error}", file=sys.stderr)
        return 1
    print(f"changelog contract passed: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
