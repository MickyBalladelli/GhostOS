#!/usr/bin/env python3
"""Validate evidence-backed claims made by a GhostOS release."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import pathlib
import re
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAIM_TERMS = {
    "scalable": re.compile(r"\bscalable\b", re.IGNORECASE),
    "durable": re.compile(r"\bdurable\b", re.IGNORECASE),
    "secure": re.compile(r"\bsecure\b", re.IGNORECASE),
    "real-time": re.compile(r"\breal[ -]time\b", re.IGNORECASE),
}
OPERATORS = {"<", "<=", ">", ">=", "=="}
SHA256 = re.compile(r"^[0-9a-f]{64}$")
MAX_CLAIMS = 64


def read_json(path: pathlib.Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_text())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{path}: invalid JSON ({error})") from error
    if not isinstance(value, dict):
        raise ValueError(f"{path}: JSON root must be an object")
    return value


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if result.returncode != 0 or not result.stdout.strip():
        raise ValueError("cannot determine current Git revision")
    return result.stdout.strip()


def required_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{field} must be a non-empty string")
    return value.strip()


def numeric(value: object, field: str) -> int | float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{field} must be numeric")
    if not math.isfinite(float(value)) or value < 0:
        raise ValueError(f"{field} must be finite and non-negative")
    return value


def evidence_path(evidence_dir: pathlib.Path, raw_path: object) -> pathlib.Path:
    relative = pathlib.PurePosixPath(required_text(raw_path, "artifact.path"))
    if relative.is_absolute() or ".." in relative.parts:
        raise ValueError("artifact.path must stay inside the evidence directory")
    candidate = evidence_dir / pathlib.Path(*relative.parts)
    if candidate.is_symlink():
        raise ValueError(f"retained artifact is missing or is a symlink: {relative}")
    path = candidate.resolve()
    try:
        path.relative_to(evidence_dir.resolve())
    except ValueError as error:
        raise ValueError("artifact.path must stay inside the evidence directory") from error
    if not path.is_file():
        raise ValueError(f"retained artifact is missing or is a symlink: {relative}")
    return path


def unreleased_section(path: pathlib.Path) -> str:
    text = path.read_text()
    match = re.search(r"^## \[Unreleased\]\s*$", text, re.MULTILINE)
    if match is None:
        raise ValueError(f"{path}: missing ## [Unreleased] section")
    remainder = text[match.end() :]
    next_heading = re.search(r"^## (?!#)", remainder, re.MULTILINE)
    return remainder[: next_heading.start()] if next_heading else remainder


def mentioned_terms(text: str) -> set[str]:
    return {term for term, pattern in CLAIM_TERMS.items() if pattern.search(text)}


def validate_claim(claim: object, index: int, evidence_dir: pathlib.Path) -> str:
    if not isinstance(claim, dict):
        raise ValueError(f"claim {index} must be an object")
    claim_id = required_text(claim.get("id"), f"claim {index}.id")
    term = required_text(claim.get("term"), f"claim {claim_id}.term").lower()
    if term not in CLAIM_TERMS:
        raise ValueError(f"claim {claim_id}.term is not a controlled release term")
    statement = required_text(claim.get("statement"), f"claim {claim_id}.statement")
    if not CLAIM_TERMS[term].search(statement):
        raise ValueError(f"claim {claim_id}.statement does not contain {term!r}")
    required_text(claim.get("workload"), f"claim {claim_id}.workload")

    threshold = claim.get("measured_threshold")
    if not isinstance(threshold, dict):
        raise ValueError(f"claim {claim_id} needs measured_threshold")
    required_text(threshold.get("metric"), f"claim {claim_id}.measured_threshold.metric")
    operator = required_text(
        threshold.get("operator"), f"claim {claim_id}.measured_threshold.operator"
    )
    if operator not in OPERATORS:
        raise ValueError(f"claim {claim_id} has an unsupported threshold operator")
    threshold_value = numeric(
        threshold.get("value"), f"claim {claim_id}.measured_threshold.value"
    )
    observed = numeric(
        threshold.get("observed"), f"claim {claim_id}.measured_threshold.observed"
    )
    passed = {
        "<": observed < threshold_value,
        "<=": observed <= threshold_value,
        ">": observed > threshold_value,
        ">=": observed >= threshold_value,
        "==": observed == threshold_value,
    }[operator]
    if not passed:
        raise ValueError(f"claim {claim_id} measured threshold is not met")
    required_text(threshold.get("unit"), f"claim {claim_id}.measured_threshold.unit")

    host = claim.get("host_configuration")
    if not isinstance(host, dict):
        raise ValueError(f"claim {claim_id} needs host_configuration")
    for field in ("system", "release", "architecture", "configuration"):
        required_text(host.get(field), f"claim {claim_id}.host_configuration.{field}")

    artifact = claim.get("artifact")
    if not isinstance(artifact, dict):
        raise ValueError(f"claim {claim_id} needs a retained artifact")
    path = evidence_path(evidence_dir, artifact.get("path"))
    digest = required_text(artifact.get("sha256"), f"claim {claim_id}.artifact.sha256").lower()
    if not SHA256.fullmatch(digest):
        raise ValueError(f"claim {claim_id}.artifact.sha256 is not a SHA-256 digest")
    if sha256(path) != digest:
        raise ValueError(f"claim {claim_id} retained artifact digest changed")
    required_text(artifact.get("description"), f"claim {claim_id}.artifact.description")
    return term


def validate(claims_path: pathlib.Path, evidence_dir: pathlib.Path, release_notes: pathlib.Path) -> int:
    claims = read_json(claims_path)
    if claims.get("schema") != 1 or claims.get("kind") != "ghostos-release-claims":
        raise ValueError("release claims have an unsupported schema")
    revision = git_revision()
    if claims.get("revision") != revision:
        raise ValueError("release claims revision does not match HEAD")
    if not evidence_dir.is_dir():
        raise ValueError(f"evidence directory does not exist: {evidence_dir}")
    entries = claims.get("claims")
    if not isinstance(entries, list) or len(entries) > MAX_CLAIMS:
        raise ValueError(f"release claims must contain at most {MAX_CLAIMS} claim records")

    ids: set[str] = set()
    terms: set[str] = set()
    for index, claim in enumerate(entries, start=1):
        term = validate_claim(claim, index, evidence_dir)
        claim_id = required_text(claim.get("id") if isinstance(claim, dict) else None, f"claim {index}.id")
        if claim_id in ids:
            raise ValueError(f"duplicate release claim ID: {claim_id}")
        ids.add(claim_id)
        terms.add(term)

    required_terms = mentioned_terms(unreleased_section(release_notes))
    missing = sorted(required_terms - terms)
    if missing:
        raise ValueError(
            "release notes use controlled claim terms without evidence-backed records: "
            + ", ".join(missing)
        )
    return len(entries)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claims", required=True, type=pathlib.Path)
    parser.add_argument("--evidence-dir", required=True, type=pathlib.Path)
    parser.add_argument("--release-notes", type=pathlib.Path, default=ROOT / "CHANGELOG.md")
    args = parser.parse_args()
    try:
        count = validate(
            args.claims.expanduser().resolve(),
            args.evidence_dir.expanduser().resolve(),
            args.release_notes.expanduser().resolve(),
        )
    except (OSError, ValueError) as error:
        print(f"release claims error: {error}", file=sys.stderr)
        return 1
    print(f"release claims verified: {count} claim records")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
