#!/usr/bin/env python3
"""Symbolize, retain, and compare GhostOS folded-stack profile archives."""

from __future__ import annotations

import argparse
import json
import pathlib
import struct
from collections import defaultdict


MAGIC = b"SNPROF01"
VERSION = 1
HEADER_BYTES = 112
STACK_BYTES = 208
CHECKSUM_OFFSET = 104


def checksum(data: bytes) -> int:
    value = 0xCBF29CE484222325
    for index, byte in enumerate(data):
        if CHECKSUM_OFFSET <= index < CHECKSUM_OFFSET + 8:
            continue
        value = ((value ^ byte) * 0x100000001B3) & ((1 << 64) - 1)
    return value


def parse_binary(path: pathlib.Path) -> dict[str, object]:
    data = path.read_bytes()
    if len(data) < HEADER_BYTES or data[:8] != MAGIC:
        raise ValueError("invalid profile magic")
    version, header = struct.unpack_from("<HH", data, 8)
    if version != VERSION or header != HEADER_BYTES:
        raise ValueError("unsupported profile version")
    stack_count = struct.unpack_from("<I", data, 100)[0]
    required = HEADER_BYTES + stack_count * STACK_BYTES
    if len(data) < required:
        raise ValueError("truncated profile")
    stored_checksum = struct.unpack_from("<Q", data, CHECKSUM_OFFSET)[0]
    if stored_checksum != checksum(data[:required]):
        raise ValueError("profile checksum mismatch")

    stacks = []
    offset = HEADER_BYTES
    for _ in range(stack_count):
        domain = data[offset]
        depth = min(data[offset + 1], 16)
        samples = struct.unpack_from("<Q", data, offset + 4)[0]
        frames = []
        for index in range(depth):
            frame_offset = offset + 12 + index * 12
            symbol, delta = struct.unpack_from("<QI", data, frame_offset)
            if symbol == 0:
                raise ValueError("profile contains an empty symbol")
            frames.append({"symbol": f"0x{symbol:016x}", "offset": delta})
        stacks.append({"domain_id": domain, "samples": samples, "frames": frames})
        offset += STACK_BYTES

    return {
        "schema": 1,
        "format": "ghostos-folded-profile",
        "revision": data[12:44].hex(),
        "host_id": data[44:60].hex(),
        "sample_period_us": struct.unpack_from("<Q", data, 60)[0],
        "started_at": struct.unpack_from("<Q", data, 68)[0],
        "ended_at": struct.unpack_from("<Q", data, 76)[0],
        "sample_count": struct.unpack_from("<Q", data, 84)[0],
        "dropped": struct.unpack_from("<Q", data, 92)[0],
        "stacks": stacks,
    }


def symbolize(profile: dict[str, object], symbols_path: pathlib.Path) -> dict[str, object]:
    symbols = json.loads(symbols_path.read_text())
    names = symbols.get("symbols", symbols)
    for stack in profile["stacks"]:
        stack["domain"] = domain_name(stack.pop("domain_id"))
        for frame in stack["frames"]:
            name = names.get(frame["symbol"])
            if not isinstance(name, str) or not name:
                raise ValueError(f"missing symbol for {frame['symbol']}")
            frame["name"] = name
    return profile


def domain_name(domain: int) -> str:
    domains = {
        1: "boot",
        2: "ipc",
        3: "scheduler",
        4: "ghostfs",
        5: "networking",
        6: "package-activation",
        7: "compiler-build",
        8: "vm-execution",
        9: "client-rpc",
    }
    try:
        return domains[domain]
    except KeyError as error:
        raise ValueError(f"unknown profile domain {domain}") from error


def validate_json_profile(profile: dict[str, object]) -> None:
    revision = profile.get("revision")
    host_id = profile.get("host_id")
    if not isinstance(revision, str) or len(revision) != 64 or any(c not in "0123456789abcdef" for c in revision):
        raise ValueError("profile needs a 32-byte lowercase revision")
    if not isinstance(host_id, str) or len(host_id) != 32 or any(c not in "0123456789abcdef" for c in host_id):
        raise ValueError("profile needs a 16-byte redacted host ID")
    forbidden = {"hostname", "address", "ip", "path", "argv", "payload", "identity", "command"}

    def walk(value: object) -> None:
        if isinstance(value, dict):
            if forbidden.intersection(value):
                raise ValueError("profile contains a redaction-sensitive field")
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    walk(profile)


def write_json(profile: dict[str, object], path: pathlib.Path) -> None:
    validate_json_profile(profile)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(profile, sort_keys=True, indent=2) + "\n")


def retain(input_path: pathlib.Path, root: pathlib.Path) -> None:
    profile = json.loads(input_path.read_text())
    validate_json_profile(profile)
    destination = root / str(profile["revision"]) / f"{profile['host_id']}.json"
    write_json(profile, destination)
    print(destination)


def compare(left_path: pathlib.Path, right_path: pathlib.Path, output: pathlib.Path | None) -> None:
    left = json.loads(left_path.read_text())
    right = json.loads(right_path.read_text())
    validate_json_profile(left)
    validate_json_profile(right)
    if left["revision"] != right["revision"]:
        raise ValueError("profiles must have the same source revision")

    def normalized(profile: dict[str, object]) -> dict[str, float]:
        totals = defaultdict(int)
        total = max(1, int(profile["sample_count"]))
        for stack in profile["stacks"]:
            key = f"{stack['domain']}:{'>'.join(frame['name'] for frame in stack['frames'])}"
            totals[key] += int(stack["samples"])
        return {key: value / total for key, value in totals.items()}

    left_values = normalized(left)
    right_values = normalized(right)
    keys = sorted(set(left_values) | set(right_values))
    result = {
        "schema": 1,
        "revision": left["revision"],
        "left_host_id": left["host_id"],
        "right_host_id": right["host_id"],
        "stacks": [
            {
                "stack": key,
                "left_fraction": left_values.get(key, 0.0),
                "right_fraction": right_values.get(key, 0.0),
                "delta": right_values.get(key, 0.0) - left_values.get(key, 0.0),
            }
            for key in keys
        ],
    }
    if output is None:
        print(json.dumps(result, sort_keys=True, indent=2))
    else:
        output.write_text(json.dumps(result, sort_keys=True, indent=2) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    symbolize_parser = subparsers.add_parser("symbolize")
    symbolize_parser.add_argument("--input", type=pathlib.Path, required=True)
    symbolize_parser.add_argument("--symbols", type=pathlib.Path, required=True)
    symbolize_parser.add_argument("--output", type=pathlib.Path, required=True)
    retain_parser = subparsers.add_parser("retain")
    retain_parser.add_argument("--input", type=pathlib.Path, required=True)
    retain_parser.add_argument("--root", type=pathlib.Path, required=True)
    compare_parser = subparsers.add_parser("compare")
    compare_parser.add_argument("--left", type=pathlib.Path, required=True)
    compare_parser.add_argument("--right", type=pathlib.Path, required=True)
    compare_parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()

    if args.command == "symbolize":
        profile = symbolize(parse_binary(args.input), args.symbols)
        write_json(profile, args.output)
    elif args.command == "retain":
        retain(args.input, args.root)
    else:
        compare(args.left, args.right, args.output)


if __name__ == "__main__":
    main()
