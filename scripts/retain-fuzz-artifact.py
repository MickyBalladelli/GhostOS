#!/usr/bin/env python3
"""Retain a fuzz failure as a corpus input with assigned triage metadata."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
from datetime import datetime, timezone

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
TRIAGE = ROOT / "fuzz/triage.toml"


def revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def targets() -> dict[str, dict[str, str]]:
    data = tomllib.loads(TRIAGE.read_text())
    return {entry["name"]: entry for entry in data["target"]}


def main() -> int:
    known_targets = targets()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=sorted(known_targets))
    parser.add_argument("artifact", type=pathlib.Path)
    args = parser.parse_args()
    artifact = args.artifact.resolve()
    if not artifact.is_file():
        parser.error(f"artifact does not exist: {artifact}")

    data = artifact.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    failure_class = artifact.name.split("-", 1)[0]
    target = known_targets[args.target]
    relative_case = pathlib.Path("fuzz/corpus") / args.target / f"regression-{digest}"
    retained = ROOT / relative_case
    retained.parent.mkdir(parents=True, exist_ok=True)
    if not retained.exists():
        shutil.copyfile(artifact, retained)

    metadata_dir = ROOT / "fuzz/regressions" / args.target
    metadata_dir.mkdir(parents=True, exist_ok=True)
    metadata = {
        "schema": 1,
        "target": args.target,
        "boundary": target["boundary"],
        "test_id": target["test_id"],
        "owner": target["owner"],
        "status": "open",
        "failure_class": failure_class,
        "sha256": digest,
        "bytes": len(data),
        "revision": revision(),
        "retained_at": datetime.now(timezone.utc).isoformat(),
        "input": relative_case.as_posix(),
        "replay": ["scripts/replay-fuzz.sh", args.target, relative_case.as_posix()],
    }
    (metadata_dir / f"{digest}.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(f"retained {args.target} {failure_class} as {relative_case}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
