#!/usr/bin/env python3
"""Validate the compatibility proof required for a SynOS release."""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
from typing import Any


SCHEMA = "synos-upgrade-compatibility"
SCHEMA_VERSION = 1
MODES = {"read-write", "read-convert"}


def require_object(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def require_text(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{name} must be a non-empty string")
    return value.strip()


def require_positive_integer(value: Any, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise ValueError(f"{name} must be a positive integer")
    return value


def validate_contract(value: Any, name: str, peer_version: int) -> dict[str, Any]:
    contract = require_object(value, name)
    minimum = require_positive_integer(
        contract.get("minimum_peer_version"), f"{name}.minimum_peer_version"
    )
    maximum = require_positive_integer(
        contract.get("maximum_peer_version"), f"{name}.maximum_peer_version"
    )
    if minimum > maximum:
        raise ValueError(f"{name} has a minimum above its maximum")
    if not minimum <= peer_version <= maximum:
        raise ValueError(
            f"{name} does not accept peer version {peer_version} "
            f"(supported range {minimum}..{maximum})"
        )
    mode = require_text(contract.get("mode"), f"{name}.mode")
    if mode not in MODES:
        allowed = ", ".join(sorted(MODES))
        raise ValueError(f"{name}.mode must be one of: {allowed}")
    return contract


def evidence_paths(value: Any, name: str) -> list[str]:
    if not isinstance(value, list) or not value:
        raise ValueError(f"{name} must contain at least one evidence path")
    paths: list[str] = []
    for index, raw_path in enumerate(value):
        path = require_text(raw_path, f"{name}[{index}]")
        candidate = pathlib.PurePosixPath(path)
        if candidate.is_absolute() or ".." in candidate.parts:
            raise ValueError(f"{name}[{index}] must stay inside the evidence directory")
        paths.append(candidate.as_posix())
    return paths


def validate_evidence(path: pathlib.PurePosixPath, evidence_root: pathlib.Path) -> None:
    resolved = (evidence_root / pathlib.Path(path)).resolve()
    try:
        resolved.relative_to(evidence_root)
    except ValueError as error:
        raise ValueError(f"evidence path escapes the evidence directory: {path}") from error
    if not resolved.is_file():
        raise ValueError(f"compatibility evidence does not exist: {path}")
    try:
        record = json.loads(resolved.read_text())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"compatibility evidence is not valid JSON: {path}") from error
    if not isinstance(record, dict):
        raise ValueError(f"compatibility evidence must contain an object: {path}")
    state = record.get("state", record.get("result_state"))
    if state == "pass":
        state = "passed"
    if state != "passed":
        raise ValueError(f"compatibility evidence is not passed: {path}")


def validate_manifest(
    manifest_path: pathlib.Path,
    evidence_root: pathlib.Path,
    expected_revision: str | None = None,
    expected_version: str | None = None,
) -> tuple[int, int]:
    try:
        manifest = json.loads(manifest_path.read_text())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read compatibility manifest: {manifest_path}") from error

    document = require_object(manifest, "manifest")
    if document.get("schema") != SCHEMA:
        raise ValueError(f"manifest.schema must be {SCHEMA!r}")
    if document.get("schema_version") != SCHEMA_VERSION:
        raise ValueError(
            f"unsupported compatibility manifest schema version: {document.get('schema_version')!r}"
        )

    release = require_object(document.get("release"), "release")
    previous = require_object(document.get("previous"), "previous")
    release_version = require_text(release.get("version"), "release.version")
    previous_version = require_text(previous.get("version"), "previous.version")
    release_revision = require_text(release.get("revision"), "release.revision")
    previous_revision = require_text(previous.get("revision"), "previous.revision")
    if release_version == previous_version:
        raise ValueError("release.version must differ from previous.version")
    if release_revision == previous_revision:
        raise ValueError("release.revision must differ from previous.revision")
    if expected_revision is not None and release_revision != expected_revision:
        raise ValueError(
            f"release.revision {release_revision!r} does not match expected revision {expected_revision!r}"
        )
    if expected_version is not None and release_version != expected_version:
        raise ValueError(
            f"release.version {release_version!r} does not match expected version {expected_version!r}"
        )

    rollback = require_object(document.get("rollback"), "rollback")
    if rollback.get("supported") is not True:
        raise ValueError("rollback.supported must be true")
    if require_text(rollback.get("target_revision"), "rollback.target_revision") != previous_revision:
        raise ValueError("rollback.target_revision must match previous.revision")

    raw_artifacts = document.get("artifacts")
    if not isinstance(raw_artifacts, list) or not raw_artifacts:
        raise ValueError("manifest.artifacts must contain at least one artifact")

    names: set[str] = set()
    evidence_count = 0
    for index, raw_artifact in enumerate(raw_artifacts):
        artifact = require_object(raw_artifact, f"artifacts[{index}]")
        name = require_text(artifact.get("name"), f"artifacts[{index}].name")
        if name in names:
            raise ValueError(f"duplicate compatibility artifact: {name}")
        names.add(name)
        old_version = require_positive_integer(
            artifact.get("previous_version"), f"artifacts[{index}].previous_version"
        )
        new_version = require_positive_integer(
            artifact.get("release_version"), f"artifacts[{index}].release_version"
        )
        upgrade = validate_contract(
            artifact.get("upgrade"), f"artifacts[{index}].upgrade", old_version
        )
        rollback_contract = validate_contract(
            artifact.get("rollback"), f"artifacts[{index}].rollback", new_version
        )

        migration = require_object(artifact.get("migration"), f"artifacts[{index}].migration")
        require_text(migration.get("id"), f"artifacts[{index}].migration.id")
        evidence = require_object(artifact.get("evidence"), f"artifacts[{index}].evidence")
        upgrade_evidence = evidence_paths(
            evidence.get("upgrade"), f"artifacts[{index}].evidence.upgrade"
        )
        rollback_evidence = evidence_paths(
            evidence.get("rollback"), f"artifacts[{index}].evidence.rollback"
        )
        for evidence_path in upgrade_evidence + rollback_evidence:
            validate_evidence(pathlib.PurePosixPath(evidence_path), evidence_root)
        evidence_count += len(upgrade_evidence) + len(rollback_evidence)

        if old_version != new_version and upgrade["mode"] == "read-write":
            raise ValueError(
                f"artifacts[{index}] changes version but declares read-write upgrade compatibility"
            )
        if old_version != new_version and rollback_contract["mode"] == "read-write":
            raise ValueError(
                f"artifacts[{index}] changes version but declares read-write rollback compatibility"
            )

    return len(raw_artifacts), evidence_count


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=pathlib.Path)
    parser.add_argument("--evidence-dir", required=True, type=pathlib.Path)
    parser.add_argument("--release-revision")
    parser.add_argument("--release-version")
    args = parser.parse_args()

    try:
        manifest_path = args.manifest.expanduser().resolve()
        evidence_root = args.evidence_dir.expanduser().resolve()
        if not manifest_path.is_file():
            raise ValueError(f"compatibility manifest does not exist: {manifest_path}")
        if not evidence_root.is_dir():
            raise ValueError(f"evidence directory does not exist: {evidence_root}")
        artifacts, evidence = validate_manifest(
            manifest_path,
            evidence_root,
            args.release_revision,
            args.release_version,
        )
    except (OSError, ValueError) as error:
        print(f"upgrade compatibility validation failed: {error}", file=sys.stderr)
        return 1

    print(f"upgrade compatibility validation passed: {artifacts} artifacts, {evidence} evidence records")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
