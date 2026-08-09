#!/usr/bin/env python3
"""Write executed evidence records for one VM inventory tier."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import platform
import subprocess
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
INVENTORY_PATH = ROOT / "virtual_machine/tests/inventory.toml"
STATES = {
    "pass": "passed",
    "passed": "passed",
    "fail": "failed",
    "failed": "failed",
    "skipped": "skipped",
}


def image_digest(paths: list[pathlib.Path]) -> tuple[list[dict[str, str]], str]:
    images = []
    seen = set()
    for path in paths:
        resolved = path.resolve()
        if resolved in seen:
            continue
        seen.add(resolved)
        digest = hashlib.sha256()
        with resolved.open("rb") as image:
            for chunk in iter(lambda: image.read(1024 * 1024), b""):
                digest.update(chunk)
        images.append({"path": resolved.as_posix(), "sha256": digest.hexdigest()})
    if not images:
        return [], "not-applicable"
    if len(images) == 1:
        return images, images[0]["sha256"]
    aggregate = hashlib.sha256()
    for image in images:
        aggregate.update(image["path"].encode())
        aggregate.update(b"\0")
        aggregate.update(image["sha256"].encode())
        aggregate.update(b"\0")
    return images, aggregate.hexdigest()


def revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def load_result(args: argparse.Namespace) -> tuple[str, int | None, str, str | None]:
    if args.result_file:
        value = json.loads(args.result_file.read_text())
        raw_state = value.get("state", value.get("result_state"))
        state = STATES.get(raw_state) if isinstance(raw_state, str) else None
        if state is None:
            raise ValueError(f"unsupported result state {raw_state!r}")
        exit_code = value.get("exit_code")
        reason = value.get("reason")
        prerequisite = value.get("prerequisite")
    else:
        state = args.state
        if state not in {"passed", "failed", "skipped"}:
            raise ValueError(f"unsupported result state {state!r}")
        exit_code = args.exit_code
        reason = args.reason
        prerequisite = None

    if not isinstance(reason, str) or not reason.strip():
        if state == "passed":
            reason = "command completed successfully"
        elif state == "failed" and exit_code is not None:
            reason = f"command exited with status {exit_code}"
        else:
            raise ValueError(f"{state} evidence requires a reason")
    if state == "failed" and (not isinstance(exit_code, int) or exit_code == 0):
        raise ValueError("failed evidence requires a non-zero integer exit code")
    return state, exit_code, reason.strip(), prerequisite


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", required=True, type=pathlib.Path)
    parser.add_argument("--tier", required=True)
    parser.add_argument("--command", required=True)
    parser.add_argument("--firmware", required=True)
    parser.add_argument("--cpu-count", required=True, type=int)
    parser.add_argument("--image", action="append", default=[], type=pathlib.Path)
    parser.add_argument("--state")
    parser.add_argument("--exit-code", type=int)
    parser.add_argument("--reason")
    parser.add_argument("--result-file", type=pathlib.Path)
    parser.add_argument("--started-at", required=True)
    parser.add_argument("--ended-at", required=True)
    args = parser.parse_args()

    if args.cpu_count < 1:
        parser.error("--cpu-count must be positive")
    for image in args.image:
        if not image.is_file():
            parser.error(f"image does not exist: {image}")

    try:
        state, exit_code, reason, prerequisite = load_result(args)
        inventory = tomllib.loads(INVENTORY_PATH.read_text())
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"cannot record VM evidence: {error}", file=sys.stderr)
        return 1

    tiers = {entry.get("name") for entry in inventory.get("tier", [])}
    if args.tier not in tiers:
        parser.error(f"unknown VM inventory tier: {args.tier}")
    tests = [entry for entry in inventory.get("test", []) if args.tier in entry.get("tiers", [])]
    if not tests:
        parser.error(f"VM inventory tier has no test IDs: {args.tier}")

    images, digest = image_digest(args.image)
    host = {
        "system": platform.system(),
        "release": platform.release(),
        "architecture": platform.machine(),
    }
    commit = revision()
    tier_output = args.evidence_dir / args.tier / "result.json"
    tier_result = {
        "schema": 1,
        "tier": args.tier,
        "state": state,
        "reason": reason,
    }
    if exit_code is not None:
        tier_result["exit_code"] = exit_code
    if prerequisite:
        tier_result["prerequisite"] = prerequisite
    tier_output.parent.mkdir(parents=True, exist_ok=True)
    tier_output.write_text(json.dumps(tier_result, indent=2, sort_keys=True) + "\n")
    for test in tests:
        record = {
            "schema": 1,
            "test_id": test["id"],
            "test_file": test["file"],
            "tier": args.tier,
            "command": args.command,
            "revision": commit,
            "host": host,
            "firmware": args.firmware,
            "cpu_count": args.cpu_count,
            "images": images,
            "image_digest": digest,
            "result_state": state,
            "exit_code": exit_code,
            "reason": reason,
            "started_at": args.started_at,
            "ended_at": args.ended_at,
        }
        output = args.evidence_dir / args.tier / test["id"] / "evidence.json"
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")

    print(f"recorded {len(tests)} executed VM evidence entries for {args.tier}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
