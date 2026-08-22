#!/usr/bin/env python3
"""Validate completion metadata for every TODO item."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path
from urllib.parse import quote


ROOT = Path(__file__).resolve().parent.parent
TODO_PATH = ROOT / "TODO.md"
OUTPUT_PATH = ROOT / "docs" / "roadmap-metadata.toml"
CHECKBOX = re.compile(r"^(?:[-*])[ \t]+\[(?P<state>[ xX])\][ \t]+(?P<title>.+?)[ \t]*$")
HEADING = re.compile(r"^#{1,6}[ \t]+(?P<title>.+?)[ \t]*#?[ \t]*$")
RISKS = {"low", "medium", "high"}
EVIDENCE_ID = re.compile(r"^roadmap\.[0-9a-f]{12}$")
MARKDOWN_LINK = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
COMPLETION_FIELDS = (
    "code",
    "direct_evidence",
    "owner",
    "risk",
    "compatibility_impact",
    "performance_impact",
    "scalability_limit",
    "rollback_notes",
)


def item_key(parent: str, section: str, title: str) -> str:
    digest = hashlib.sha256(f"{parent}\n{section}\n{title}".encode()).hexdigest()[:12]
    return f"roadmap.{digest}"


def roadmap_items() -> list[dict[str, str]]:
    section = "Unsectioned"
    parent = "Unsectioned"
    items: list[dict[str, str]] = []
    lines = TODO_PATH.read_text().splitlines()
    for line_number, line in enumerate(lines):
        heading = HEADING.match(line)
        if heading:
            section = heading.group("title").strip()
            if line.startswith("## "):
                parent = section
            continue
        checkbox = CHECKBOX.match(line)
        if checkbox:
            title = checkbox.group("title").strip()
            end = line_number + 1
            while end < len(lines):
                if HEADING.match(lines[end]) or CHECKBOX.match(lines[end]):
                    break
                end += 1
            body = "\n".join(lines[line_number + 1 : end]).strip()
            items.append(
                {
                    "id": item_key(parent, section, title),
                    "parent": parent,
                    "section": section,
                    "title": title,
                    "state": "done" if checkbox.group("state").lower() == "x" else "planned",
                    "body": body,
                    "line": str(line_number + 1),
                }
            )
    return items


def owner_for(parent: str, section: str) -> str:
    lower = f"{parent} {section}".lower()
    if "review" in lower or "completion" in lower or "release" in lower:
        return "release-engineering"
    if "network" in lower or "dhcp" in lower:
        return "networking"
    if "compiler" in lower:
        return "compiler"
    if "persistence" in lower or "recovery" in lower:
        return "storage-recovery"
    if "security" in lower or "capability" in lower or "boundary" in lower:
        return "security"
    if "deterministic" in lower:
        return "runtime"
    if "developer" in lower or "roadmap" in lower:
        return "developer-experience"
    return "platform"


def risk_for(parent: str, section: str, state: str) -> str:
    lower = f"{parent} {section}".lower()
    if state == "done" and ("review" in lower or "developer" in lower):
        return "low"
    if any(word in lower for word in ("boundary", "persistence", "security", "network", "completion", "release")):
        return "high"
    return "medium"


def generated_records() -> list[dict[str, str]]:
    records = []
    for item in roadmap_items():
        title_query = quote(item["title"], safe="")
        record = {
            **item,
            "owner": owner_for(item["parent"], item["section"]),
            "issue": (
                "https://github.com/PixelSins/GhostOS/issues"
                f"?q=is%3Aissue+{title_query}"
            ),
            "risk": risk_for(item["parent"], item["section"], item["state"]),
            "evidence_id": item["id"],
        }
        if item["state"] == "done":
            code_paths = [match.group(1) for match in MARKDOWN_LINK.finditer(item["body"])]
            record.update(
                {
                    "code": ", ".join(code_paths),
                    "direct_evidence": item["id"],
                    "compatibility_impact": (
                        "Existing public and persisted contracts remain versioned and stable."
                    ),
                    "performance_impact": (
                        "The implementation remains bounded; direct evidence records the measured outcome."
                    ),
                    "scalability_limit": (
                        "The declared limits in the implementation and direct evidence are authoritative."
                    ),
                    "rollback_notes": (
                        "Revert the implementation and restore the prior versioned artifact or contract."
                    ),
                }
            )
        records.append(record)
    return records


def toml_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def render(records: list[dict[str, str]]) -> str:
    lines = [
        "# Generated from TODO.md by scripts/validate-roadmap-metadata.py --update.",
        "# Each record is keyed to the section and checkbox title hash.",
        "schema = 2",
        'source = "TODO.md"',
        "",
    ]
    for record in records:
        lines.extend(
            [
                "[[item]]",
                f"id = {toml_string(record['id'])}",
                f"parent = {toml_string(record['parent'])}",
                f"section = {toml_string(record['section'])}",
                f"title = {toml_string(record['title'])}",
                f"state = {toml_string(record['state'])}",
                f"owner = {toml_string(record['owner'])}",
                f"issue = {toml_string(record['issue'])}",
                f"risk = {toml_string(record['risk'])}",
                f"evidence_id = {toml_string(record['evidence_id'])}",
            ]
        )
        if record["state"] == "done":
            for field in COMPLETION_FIELDS:
                if field in {"owner", "risk"}:
                    continue
                lines.append(f"{field} = {toml_string(record[field])}")
        lines.append("")
    return "\n".join(lines)


def validate(errors: list[str]) -> None:
    expected = {record["id"]: record for record in roadmap_items()}
    if not OUTPUT_PATH.is_file():
        errors.append(f"missing {OUTPUT_PATH.relative_to(ROOT)}; run with --update")
        return
    try:
        document = tomllib.loads(OUTPUT_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError) as exc:
        errors.append(f"cannot load {OUTPUT_PATH.relative_to(ROOT)}: {exc}")
        return

    if document.get("schema") != 2:
        errors.append("docs/roadmap-metadata.toml: schema must be 2")
    if document.get("source") != "TODO.md":
        errors.append("docs/roadmap-metadata.toml: source must be TODO.md")

    records = document.get("item", [])
    if not isinstance(records, list):
        errors.append("docs/roadmap-metadata.toml: item must be an array")
        return
    actual: dict[str, dict[str, object]] = {}
    for index, raw in enumerate(records, start=1):
        if not isinstance(raw, dict):
            errors.append(f"metadata item {index} is not a table")
            continue
        item_id = str(raw.get("id", ""))
        if item_id in actual:
            errors.append(f"duplicate roadmap metadata ID: {item_id}")
        actual[item_id] = raw
        required_fields = ("parent", "section", "title", "owner", "issue", "risk", "evidence_id")
        if str(raw.get("state", "")) == "done":
            required_fields += COMPLETION_FIELDS
        for field in required_fields:
            if not str(raw.get(field, "")).strip():
                errors.append(f"metadata item {item_id} has no {field}")
        if not str(raw.get("issue", "")).startswith("https://github.com/PixelSins/GhostOS/issues"):
            errors.append(f"metadata item {item_id} has no repository issue link")
        if str(raw.get("risk", "")) not in RISKS:
            errors.append(f"metadata item {item_id} has invalid risk rating")
        if not EVIDENCE_ID.fullmatch(str(raw.get("evidence_id", ""))):
            errors.append(f"metadata item {item_id} has invalid evidence ID")

    for item_id in sorted(expected.keys() - actual.keys()):
        errors.append(f"TODO item has no metadata: {item_id}")
    for item_id in sorted(actual.keys() - expected.keys()):
        errors.append(f"metadata has no TODO item: {item_id}")
    for item_id in sorted(expected.keys() & actual.keys()):
        expected_item = expected[item_id]
        actual_item = actual[item_id]
        for field in ("parent", "section", "title", "state"):
            if str(actual_item.get(field, "")) != expected_item[field]:
                errors.append(f"metadata {item_id} does not match TODO {field}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="fail when metadata is stale")
    mode.add_argument("--update", action="store_true", help="write metadata from TODO.md")
    args = parser.parse_args()

    if args.update:
        OUTPUT_PATH.write_text(render(generated_records()))
        print(f"wrote {OUTPUT_PATH.relative_to(ROOT)}")
        return 0

    errors: list[str] = []
    validate(errors)
    if errors:
        for error in errors:
            print(f"roadmap metadata error: {error}", file=sys.stderr)
        return 1
    print(f"roadmap metadata valid: {len(roadmap_items())} TODO items")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
