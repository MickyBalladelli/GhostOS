#!/usr/bin/env python3
"""Validate roadmap structure, links, and evidence mappings."""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parent.parent
INVENTORY_PATH = ROOT / "docs" / "test-inventory.toml"
COVERAGE_PATH = ROOT / "docs" / "test-coverage.toml"
NUMBERED_HEADING = re.compile(
    r"^(?P<marks>#{1,6})[ \t]+(?P<id>\d+(?:\.\d+)*)(?:\.[ \t]*|[ \t]+)(?P<title>.*?)[ \t]*$"
)
ANY_HEADING = re.compile(r"^#{1,6}[ \t]+.+")
MARKDOWN_LINK = re.compile(r"(?<!!)\[[^\]]+\]\(([^)\n]+)\)")
SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")


def roadmap_paths() -> list[Path]:
    ignored = {".git", "build", "target"}
    return sorted(
        path
        for path in ROOT.rglob("TODO*.md")
        if not any(part in ignored for part in path.parts)
    )


def numbered_headings(path: Path) -> list[dict[str, object]]:
    lines = path.read_text().splitlines()
    headings: list[dict[str, object]] = []
    for index, line in enumerate(lines):
        match = NUMBERED_HEADING.match(line)
        if match:
            headings.append(
                {
                    "line": index,
                    "level": len(match.group("marks")),
                    "id": match.group("id"),
                    "title": (match.group("title") or "").strip(),
                }
            )
    return headings


def validate_numbered_headings(path: Path, errors: list[str]) -> None:
    lines = path.read_text().splitlines()
    headings = numbered_headings(path)
    seen: dict[str, int] = {}

    for heading in headings:
        heading_id = str(heading["id"])
        line_number = int(heading["line"]) + 1
        previous_line = seen.get(heading_id)
        if previous_line is not None:
            errors.append(
                f"{path.relative_to(ROOT)}:{line_number}: duplicate roadmap ID "
                f"{heading_id} (first used at line {previous_line})"
            )
        else:
            seen[heading_id] = line_number

        if not str(heading["title"]):
            errors.append(f"{path.relative_to(ROOT)}:{line_number}: numbered heading has no title")

        start = int(heading["line"]) + 1
        end = len(lines)
        for candidate in range(start, len(lines)):
            if ANY_HEADING.match(lines[candidate]):
                end = candidate
                break
        body_lines = [
            line.strip()
            for line in lines[start:end]
            if line.strip()
            and line.strip() != "---"
            and not line.strip().startswith("<!--")
            and not ANY_HEADING.match(line)
        ]
        if not body_lines:
            errors.append(
                f"{path.relative_to(ROOT)}:{line_number}: numbered feature {heading_id} has an empty body"
            )


def heading_slugs(path: Path) -> set[str]:
    slugs: set[str] = set()
    counts: dict[str, int] = {}
    for line in path.read_text().splitlines():
        match = re.match(r"^#{1,6}[ \t]+(.+?)[ \t]*#?[ \t]*$", line)
        if not match:
            continue
        title = re.sub(r"<[^>]*>", "", match.group(1).strip().lower())
        slug = re.sub(r"[^\w\s-]", "", title)
        slug = re.sub(r"\s+", "-", slug)
        occurrence = counts.get(slug, 0)
        counts[slug] = occurrence + 1
        slugs.add(slug if occurrence == 0 else f"{slug}-{occurrence}")
    return slugs


def link_target(raw_target: str) -> str:
    target = raw_target.strip()
    if target.startswith("<") and ">" in target:
        return target[1 : target.index(">")]
    return target.split()[0]


def validate_links(path: Path, errors: list[str]) -> None:
    lines = path.read_text().splitlines()
    in_fence = False
    cached_slugs: dict[Path, set[str]] = {}

    for line_number, line in enumerate(lines, start=1):
        if line.lstrip().startswith(("```", "~~~")):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for match in MARKDOWN_LINK.finditer(line):
            target = link_target(match.group(1))
            if not target:
                continue
            if SCHEME.match(target) or target.startswith("//"):
                continue

            path_part, separator, fragment = target.partition("#")
            source_path = path if not path_part else path.parent / unquote(path_part)
            try:
                resolved = source_path.resolve()
                resolved.relative_to(ROOT.resolve())
            except ValueError:
                errors.append(
                    f"{path.relative_to(ROOT)}:{line_number}: link escapes repository: {target}"
                )
                continue

            if not resolved.exists():
                errors.append(
                    f"{path.relative_to(ROOT)}:{line_number}: stale link target: {target}"
                )
                continue
            if separator and fragment:
                if resolved not in cached_slugs:
                    cached_slugs[resolved] = heading_slugs(resolved)
                if unquote(fragment).lower() not in cached_slugs[resolved]:
                    errors.append(
                        f"{path.relative_to(ROOT)}:{line_number}: stale link anchor: {target}"
                    )


def validate_root_inventory(errors: list[str]) -> None:
    try:
        inventory = tomllib.loads(INVENTORY_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError) as exc:
        errors.append(f"cannot load {INVENTORY_PATH.relative_to(ROOT)}: {exc}")
        return
    source = inventory.get("source")
    if not isinstance(source, str) or not source.strip():
        errors.append("docs/test-inventory.toml: source must name the roadmap file")
        return
    todo_path = ROOT / source
    if not todo_path.is_file():
        errors.append(f"docs/test-inventory.toml: source file is missing: {source}")
        return

    root_features = {
        f"{int(heading['id']):02d}": heading
        for heading in numbered_headings(todo_path)
        if int(heading["level"]) == 2
        and re.fullmatch(r"\d+", str(heading["id"]))
        and int(str(heading["id"])) <= 58
    }
    inventory_features = inventory.get("feature", [])
    inventory_by_id: dict[str, dict[str, object]] = {}

    if not isinstance(inventory_features, list):
        errors.append("docs/test-inventory.toml: feature entries are not an array")
        return

    for index, feature in enumerate(inventory_features, start=1):
        if not isinstance(feature, dict):
            errors.append(f"docs/test-inventory.toml: feature entry {index} is not a table")
            continue
        raw_id = str(feature.get("id", ""))
        if not re.fullmatch(r"\d+", raw_id):
            errors.append(f"docs/test-inventory.toml: feature entry {index} has invalid ID {raw_id!r}")
            continue
        feature_id = f"{int(raw_id):02d}"
        if feature_id in inventory_by_id:
            errors.append(f"docs/test-inventory.toml: duplicate feature ID {feature_id}")
        else:
            inventory_by_id[feature_id] = feature
        if not str(feature.get("todo_heading", "")).strip():
            errors.append(f"docs/test-inventory.toml: feature {feature_id} has no todo_heading")

    for feature_id in sorted(root_features.keys() - inventory_by_id.keys()):
        errors.append(f"{source}: feature heading {feature_id} is missing from the inventory")
    for feature_id in sorted(inventory_by_id.keys() - root_features.keys()):
        errors.append(f"docs/test-inventory.toml: feature {feature_id} has no {source} heading")


def known_evidence_ids(inventory: dict[str, object], errors: list[str]) -> set[str]:
    evidence_ids: set[str] = set()
    for feature in inventory.get("feature", []):
        if not isinstance(feature, dict):
            continue
        raw_id = str(feature.get("id", ""))
        if re.fullmatch(r"\d+", raw_id):
            feature_id = f"{int(raw_id):02d}"
            evidence_ids.add(f"feature.{feature_id}")
        for tier, values in feature.items():
            if tier in {"id", "todo_heading", "owner"} or not isinstance(values, list):
                continue
            evidence_ids.update(str(value) for value in values if str(value).strip())

    try:
        coverage = tomllib.loads(COVERAGE_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError) as exc:
        errors.append(f"cannot load {COVERAGE_PATH.relative_to(ROOT)}: {exc}")
        return evidence_ids

    for section_name in ("vm_device", "boot_path"):
        entries = coverage.get(section_name, [])
        if not isinstance(entries, list):
            errors.append(f"docs/test-coverage.toml: {section_name} entries are not an array")
            continue
        for entry in entries:
            if isinstance(entry, dict) and str(entry.get("test_id", "")).strip():
                evidence_ids.add(str(entry["test_id"]))
    return evidence_ids


def validate_roadmap_mappings(paths: list[Path], errors: list[str]) -> None:
    try:
        inventory = tomllib.loads(INVENTORY_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError) as exc:
        errors.append(f"cannot load {INVENTORY_PATH.relative_to(ROOT)}: {exc}")
        return

    config = inventory.get("roadmap_validation")
    if not isinstance(config, dict):
        errors.append("docs/test-inventory.toml: missing [roadmap_validation] table")
        return

    expected_paths = config.get("paths")
    if not isinstance(expected_paths, list) or not expected_paths or not all(
        isinstance(path, str) and path.strip() for path in expected_paths
    ):
        errors.append("docs/test-inventory.toml: roadmap_validation.paths must be a non-empty string array")
        return

    discovered = {path.relative_to(ROOT).as_posix() for path in paths}
    expected = {str(path) for path in expected_paths}
    for path in sorted(discovered - expected):
        errors.append(f"docs/test-inventory.toml: roadmap file {path} is missing from roadmap_validation.paths")
    for path in sorted(expected - discovered):
        errors.append(f"docs/test-inventory.toml: roadmap_validation.paths lists missing file {path}")

    mappings = inventory.get("roadmap")
    if not isinstance(mappings, list):
        errors.append("docs/test-inventory.toml: roadmap evidence mappings are missing")
        return

    mapping_by_path: dict[str, dict[str, object]] = {}
    for index, mapping in enumerate(mappings, start=1):
        if not isinstance(mapping, dict):
            errors.append(f"docs/test-inventory.toml: roadmap mapping {index} is not a table")
            continue
        path = str(mapping.get("path", "")).strip()
        if not path:
            errors.append(f"docs/test-inventory.toml: roadmap mapping {index} has no path")
            continue
        if path in mapping_by_path:
            errors.append(f"docs/test-inventory.toml: duplicate roadmap mapping for {path}")
        else:
            mapping_by_path[path] = mapping

        if not str(mapping.get("scope", "")).strip():
            errors.append(f"docs/test-inventory.toml: roadmap mapping {path} has no scope")
        evidence = mapping.get("evidence_ids")
        if not isinstance(evidence, list) or not evidence:
            errors.append(f"docs/test-inventory.toml: roadmap {path} has no evidence mappings")
            continue
        if not all(isinstance(value, str) and value.strip() for value in evidence):
            errors.append(f"docs/test-inventory.toml: roadmap {path} has an empty evidence ID")

    known_ids = known_evidence_ids(inventory, errors)
    for path in sorted(discovered):
        mapping = mapping_by_path.get(path)
        if mapping is None:
            errors.append(f"docs/test-inventory.toml: roadmap {path} has no evidence mapping")
            continue
        evidence = mapping.get("evidence_ids", [])
        if not isinstance(evidence, list):
            continue
        for evidence_id in evidence:
            if isinstance(evidence_id, str) and evidence_id.strip() and evidence_id not in known_ids:
                errors.append(
                    f"docs/test-inventory.toml: roadmap {path} references unknown evidence ID {evidence_id}"
                )

    for path in sorted(set(mapping_by_path) - discovered):
        errors.append(f"docs/test-inventory.toml: evidence mapping has no roadmap file {path}")


def main() -> int:
    errors: list[str] = []
    paths = roadmap_paths()
    for path in paths:
        validate_numbered_headings(path, errors)
        validate_links(path, errors)
    validate_root_inventory(errors)
    validate_roadmap_mappings(paths, errors)

    if errors:
        for error in errors:
            print(f"roadmap error: {error}", file=sys.stderr)
        return 1

    print(f"roadmaps valid: {len(paths)} files, root inventory IDs 01..58")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
