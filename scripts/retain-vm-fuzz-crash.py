#!/usr/bin/env python3
"""Retain a VM fuzz artifact as a deterministic corpus regression."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
from datetime import datetime, timezone


ROOT = pathlib.Path(__file__).resolve().parent.parent
TARGETS = {
    "vm-decoder": "vm.fuzz.decoder",
    "vm-devices": "vm.fuzz.device",
    "vm-images": "vm.fuzz.disk-image",
    "vm-snapshot": "vm.fuzz.snapshot",
    "vm-terminal": "vm.fuzz.terminal",
    "vm-migration": "vm.fuzz.migration",
}


def revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=sorted(TARGETS))
    parser.add_argument("artifact", type=pathlib.Path)
    args = parser.parse_args()
    if not args.artifact.is_file():
        parser.error(f"artifact does not exist: {args.artifact}")

    data = args.artifact.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    relative_case = pathlib.Path("fuzz/corpus") / args.target / f"regression-{digest}"
    retained = ROOT / relative_case
    retained.parent.mkdir(parents=True, exist_ok=True)
    if not retained.exists():
        shutil.copyfile(args.artifact, retained)

    metadata_dir = ROOT / "fuzz/regressions" / args.target
    metadata_dir.mkdir(parents=True, exist_ok=True)
    metadata = {
        "schema": 1,
        "target": args.target,
        "test_id": TARGETS[args.target],
        "sha256": digest,
        "bytes": len(data),
        "revision": revision(),
        "retained_at": datetime.now(timezone.utc).isoformat(),
        "input": relative_case.as_posix(),
        "replay": ["scripts/replay-vm-fuzz.sh", args.target, relative_case.as_posix()],
    }
    (metadata_dir / f"{digest}.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(f"retained {args.target} crash as {relative_case}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
