#!/usr/bin/env python3
"""Create or verify an integrity-protected full-validation evidence manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
from datetime import datetime, timezone


MANIFEST_NAME = "evidence-manifest.json"
DIGEST_NAME = "evidence-manifest.json.sha256"
RESULT_NAMES = {"result.json", "evidence.json"}
SEPARATE_TIERS = {"qemu", "hardware-accelerated", "hardware-boot", "fuzz", "soak"}
ROOT = pathlib.Path(__file__).resolve().parent.parent


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def evidence_files(evidence_dir: pathlib.Path) -> list[pathlib.Path]:
    excluded = {MANIFEST_NAME, DIGEST_NAME}
    return sorted(
        path
        for path in evidence_dir.rglob("*")
        if path.is_file()
        and not path.is_symlink()
        and path.relative_to(evidence_dir).as_posix() not in excluded
    )


def result_records(evidence_dir: pathlib.Path) -> tuple[list[dict[str, object]], list[str]]:
    records: list[dict[str, object]] = []
    errors: list[str] = []
    for path in evidence_files(evidence_dir):
        if path.name not in RESULT_NAMES:
            continue
        try:
            value = json.loads(path.read_text())
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            errors.append(f"{path.relative_to(evidence_dir)}: invalid JSON ({error})")
            continue
        if not isinstance(value, dict):
            errors.append(f"{path.relative_to(evidence_dir)}: result is not an object")
            continue
        relative = path.relative_to(evidence_dir)
        section = relative.parts[0] if relative.parts else ""
        declared_tier = value.get("tier")
        if isinstance(declared_tier, str) and declared_tier in SEPARATE_TIERS and section != declared_tier:
            errors.append(
                f"{relative}: tier {declared_tier!r} is stored under {section!r}; "
                "external results must stay separate"
            )
        if section in SEPARATE_TIERS and section != "hardware-boot" and declared_tier != section:
            errors.append(f"{relative}: tier must be {section!r}")
        state = value.get("result_state", value.get("state"))
        if state not in {"passed", "failed", "skipped", "inconclusive"}:
            errors.append(f"{path.relative_to(evidence_dir)}: invalid state {state!r}")
            continue
        reason = value.get("reason")
        if not isinstance(reason, str) or not reason.strip():
            errors.append(f"{path.relative_to(evidence_dir)}: result has no reason")
        prerequisite = value.get("prerequisite")
        records.append(
            {
                "path": path.relative_to(evidence_dir).as_posix(),
                "state": state,
                "tier": value.get("tier"),
                "test_id": value.get("test_id"),
                "reason": reason,
                "prerequisite": prerequisite,
            }
        )
    return records, errors


def manifest_value(evidence_dir: pathlib.Path) -> dict[str, object]:
    records, errors = result_records(evidence_dir)
    if errors:
        raise ValueError("\n".join(errors))
    if not records:
        raise ValueError("evidence directory contains no result or evidence records")
    files = [
        {
            "path": path.relative_to(evidence_dir).as_posix(),
            "size": path.stat().st_size,
            "sha256": sha256(path),
        }
        for path in evidence_files(evidence_dir)
    ]
    skipped = [record for record in records if record["state"] == "skipped"]
    failed = [record for record in records if record["state"] == "failed"]
    inconclusive = [record for record in records if record["state"] == "inconclusive"]
    return {
        "schema": 1,
        "kind": "synos-full-validation-evidence",
        "revision": git_revision(),
        "evidence_directory": evidence_dir.name,
        "generated_at": datetime.now(timezone.utc).replace(microsecond=0).isoformat(),
        "state": "failed" if failed else "inconclusive" if inconclusive else "passed",
        "file_count": len(files),
        "files": files,
        "results": records,
        "skipped_count": len(skipped),
        "failed_count": len(failed),
        "inconclusive_count": len(inconclusive),
    }


def canonical_bytes(value: dict[str, object]) -> bytes:
    return json.dumps(value, indent=2, sort_keys=True).encode() + b"\n"


def digest_path(manifest_path: pathlib.Path) -> pathlib.Path:
    return manifest_path.with_name(DIGEST_NAME)


def write_manifest(evidence_dir: pathlib.Path) -> None:
    manifest_path = evidence_dir / MANIFEST_NAME
    manifest_bytes = canonical_bytes(manifest_value(evidence_dir))
    manifest_path.write_bytes(manifest_bytes)
    digest_path(manifest_path).write_text(
        f"{hashlib.sha256(manifest_bytes).hexdigest()}  {MANIFEST_NAME}\n"
    )
    print(f"wrote {manifest_path}")


def verify_manifest(evidence_dir: pathlib.Path) -> None:
    manifest_path = evidence_dir / MANIFEST_NAME
    digest_file = digest_path(manifest_path)
    if not manifest_path.is_file() or not digest_file.is_file():
        raise ValueError("evidence manifest or detached digest is missing")
    try:
        manifest = json.loads(manifest_path.read_text())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid evidence manifest: {error}") from error
    if not isinstance(manifest, dict) or manifest.get("schema") != 1:
        raise ValueError("evidence manifest has an unsupported schema")
    digest_fields = digest_file.read_text().strip().split()
    if len(digest_fields) != 2 or digest_fields[1] != MANIFEST_NAME:
        raise ValueError("evidence manifest detached digest is malformed")
    expected_digest = digest_fields[0]
    actual_digest = sha256(manifest_path)
    if expected_digest != actual_digest:
        raise ValueError("evidence manifest detached digest does not match")

    listed = {
        str(entry.get("path")): entry
        for entry in manifest.get("files", [])
        if isinstance(entry, dict)
    }
    actual_files = {
        path.relative_to(evidence_dir).as_posix(): path
        for path in evidence_files(evidence_dir)
    }
    if set(listed) != set(actual_files):
        missing = sorted(set(listed) - set(actual_files))
        extra = sorted(set(actual_files) - set(listed))
        raise ValueError(f"evidence file set changed: missing={missing}, extra={extra}")
    for relative, entry in listed.items():
        path = actual_files[relative]
        if entry.get("size") != path.stat().st_size:
            raise ValueError(f"evidence file size changed: {relative}")
        if entry.get("sha256") != sha256(path):
            raise ValueError(f"evidence file digest changed: {relative}")

    records, errors = result_records(evidence_dir)
    if errors:
        raise ValueError("\n".join(errors))
    if records != manifest.get("results"):
        raise ValueError("evidence result records changed")
    if manifest.get("revision") != git_revision():
        raise ValueError("evidence manifest revision does not match HEAD")
    if manifest.get("failed_count") != sum(record["state"] == "failed" for record in records):
        raise ValueError("evidence manifest failed count is stale")
    if manifest.get("skipped_count") != sum(record["state"] == "skipped" for record in records):
        raise ValueError("evidence manifest skipped count is stale")
    if manifest.get("inconclusive_count") != sum(
        record["state"] == "inconclusive" for record in records
    ):
        raise ValueError("evidence manifest inconclusive count is stale")
    print(f"evidence manifest verified: {len(actual_files)} files")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", required=True, type=pathlib.Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    args = parser.parse_args()
    evidence_dir = args.evidence_dir.expanduser().resolve()
    try:
        if not evidence_dir.is_dir():
            raise ValueError(f"evidence directory does not exist: {evidence_dir}")
        if args.write:
            write_manifest(evidence_dir)
        else:
            verify_manifest(evidence_dir)
    except (OSError, ValueError) as error:
        print(f"evidence manifest error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
