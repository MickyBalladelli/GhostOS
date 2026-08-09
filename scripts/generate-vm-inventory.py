#!/usr/bin/env python3
"""Generate the source, public API, and device portion of the VM inventory."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
VM_ROOT = ROOT / "virtual_machine"
COVERAGE_PATH = VM_ROOT / "tests/inventory.toml"
OUTPUT_PATH = VM_ROOT / "tests/generated-inventory.toml"

PUBLIC_ITEM = re.compile(
    r"^\s*pub\s+(?:(?:async|const|unsafe)\s+)*(?:extern\s+\"[^\"]+\"\s+)?"
    r"(fn|struct|enum|union|trait|type|const|static|mod)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
IMPL_HEADER = re.compile(r"^\s*impl(?:<[^{}]*>)?\s+(.+?)\s*\{")
DEVICE_IMPL = re.compile(
    r"^\s*impl\s+(Device|PortDevice)\s+for\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)
VIRTIO_DEVICE = re.compile(r"impl_virtio_port_device!\(\s*([A-Za-z_][A-Za-z0-9_]*)")
PUBLIC_USE = re.compile(r"(?ms)^\s*pub\s+use\s+(.+?);")


def quoted(value: str) -> str:
    return json.dumps(value)


def source_paths() -> list[pathlib.Path]:
    return sorted((VM_ROOT / "src").rglob("*.rs"))


def source_name(path: pathlib.Path) -> str:
    return path.relative_to(VM_ROOT).as_posix()


def strip_line(line: str) -> str:
    line = line.split("//", 1)[0]
    return re.sub(r'"(?:\\.|[^"\\])*"', '""', line)


def production_text(path: pathlib.Path) -> str:
    return re.split(r"(?m)^\s*#\[cfg\(test\)\]", path.read_text(), maxsplit=1)[0]


def impl_owner(header: str) -> str:
    target = header.rsplit(" for ", 1)[-1].strip()
    target = re.sub(r"\s+where\s+.*$", "", target)
    target = re.sub(r"<.*>", "", target)
    return target.split("::")[-1].strip()


def public_items(path: pathlib.Path) -> list[tuple[str, str]]:
    items: set[tuple[str, str]] = set()
    depth = 0
    impls: list[tuple[int, str]] = []

    for original in production_text(path).splitlines():
        line = strip_line(original)
        while impls and depth < impls[-1][0]:
            impls.pop()

        header = IMPL_HEADER.match(line)
        if header:
            impls.append((depth + line.count("{") - line.count("}"), impl_owner(header.group(1))))

        item = PUBLIC_ITEM.match(line)
        if item:
            kind, name = item.groups()
            owner = impls[-1][1] if kind == "fn" and impls else ""
            qualified = f"{owner}::{name}" if owner else name
            items.add((kind, qualified))

        depth += line.count("{") - line.count("}")
        while impls and depth < impls[-1][0]:
            impls.pop()

    for expression in PUBLIC_USE.findall(production_text(path)):
        expression = " ".join(expression.split())
        if "{" in expression and "}" in expression:
            body = expression.split("{", 1)[1].rsplit("}", 1)[0]
            exports = body.split(",")
        else:
            exports = [expression]
        for export in exports:
            export = export.strip()
            if not export:
                continue
            name = export.rsplit(" as ", 1)[-1].split("::")[-1].strip()
            if name == "self":
                name = expression.split("{", 1)[0].rstrip(":").split("::")[-1]
            items.add(("use", name))

    return sorted(items, key=lambda item: (item[1], item[0]))


def device_items(path: pathlib.Path) -> list[tuple[str, str]]:
    text = production_text(path)
    devices = {
        (match.group(2), match.group(1))
        for match in DEVICE_IMPL.finditer(text)
        if match.group(2) != "Rc"
    }
    if path.name == "virtio.rs":
        for name in VIRTIO_DEVICE.findall(text):
            devices.add((name, "Device"))
            devices.add((name, "PortDevice"))
    return sorted(devices)


def coverage() -> tuple[dict[str, list[str]], dict[str, list[str]], dict[str, list[str]]]:
    data = tomllib.loads(COVERAGE_PATH.read_text())
    sources = {entry["path"]: entry.get("tests", []) for entry in data.get("source", [])}

    apis: dict[str, list[str]] = {}
    for entry in data.get("public_api", []):
        apis.setdefault(entry["source"], [])
        apis[entry["source"]].extend(entry.get("tests", []))

    devices: dict[str, list[str]] = {}
    for entry in data.get("device", []):
        devices.setdefault(entry["source"], [])
        devices[entry["source"]].extend(entry.get("tests", []))

    return (
        {path: sorted(set(tests)) for path, tests in sources.items()},
        {path: sorted(set(tests)) for path, tests in apis.items()},
        {path: sorted(set(tests)) for path, tests in devices.items()},
    )


def existing_assignments() -> tuple[dict[tuple[str, str, str], list[str]], dict[tuple[str, str, str], list[str]]] | None:
    if not OUTPUT_PATH.is_file():
        return None
    try:
        data = tomllib.loads(OUTPUT_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError):
        return None
    apis: dict[tuple[str, str, str], list[str]] = {}
    devices: dict[tuple[str, str, str], list[str]] = {}
    for entry in data.get("public_api", []):
        apis[(entry["source"], entry["kind"], entry["name"])] = entry.get("tests", [])
    for entry in data.get("device", []):
        devices[(entry["source"], entry["name"], entry["interface"])] = entry.get("tests", [])
    for source in data.get("source", []):
        path = source["path"]
        for group in source.get("public_api", []):
            for symbol in group.get("symbols", []):
                kind, name = symbol.split(":", 1)
                apis[(path, kind, name)] = group.get("tests", [])
        for group in source.get("device", []):
            for symbol in group.get("symbols", []):
                interface, name = symbol.split(":", 1)
                devices[(path, name, interface)] = group.get("tests", [])
    return apis, devices


def render() -> tuple[str, list[str]]:
    source_tests, api_tests, device_tests = coverage()
    existing = existing_assignments()
    existing_apis, existing_devices = existing or ({}, {})
    errors: list[str] = []
    lines = [
        "# Generated by scripts/generate-vm-inventory.py.",
        "# Entries are generated; assign tests to new public_api and device entries here.",
        "schema = 1",
        "",
    ]

    for path in source_paths():
        relative = source_name(path)
        tests = source_tests.get(relative, [])
        if not tests:
            errors.append(f"source module has no named test: {relative}")
        lines.extend(("[[source]]", f"path = {quoted(relative)}", f"tests = {json.dumps(tests)}", ""))

        api_groups: dict[tuple[str, ...], list[str]] = {}
        for kind, name in public_items(path):
            key = (relative, kind, name)
            tests = existing_apis.get(key, []) if existing else api_tests.get(relative, [])
            if not tests:
                errors.append(f"public API has no named test: {relative}::{name}")
            api_groups.setdefault(tuple(tests), []).append(f"{kind}:{name}")

        for assigned_tests, symbols in sorted(api_groups.items()):
            lines.extend(("[[source.public_api]]", f"tests = {json.dumps(assigned_tests)}", "symbols = ["))
            lines.extend(f"    {quoted(symbol)}," for symbol in sorted(symbols))
            lines.extend(("]", ""))

        device_groups: dict[tuple[str, ...], list[str]] = {}
        for name, interface in device_items(path):
            key = (relative, name, interface)
            tests = existing_devices.get(key, []) if existing else device_tests.get(relative, [])
            if not tests:
                errors.append(f"device has no named test: {relative}::{name}")
            device_groups.setdefault(tuple(tests), []).append(f"{interface}:{name}")

        for assigned_tests, symbols in sorted(device_groups.items()):
            lines.extend(("[[source.device]]", f"tests = {json.dumps(assigned_tests)}", "symbols = ["))
            lines.extend(f"    {quoted(symbol)}," for symbol in sorted(symbols))
            lines.extend(("]", ""))

    return "\n".join(lines), errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="fail if the committed inventory is stale")
    args = parser.parse_args()
    generated, errors = render()

    if errors and args.check:
        for error in errors:
            print(f"VM inventory error: {error}", file=sys.stderr)
        return 1

    if args.check:
        current = OUTPUT_PATH.read_text() if OUTPUT_PATH.is_file() else ""
        if current != generated:
            print(
                "VM generated inventory is stale; run python3 scripts/generate-vm-inventory.py",
                file=sys.stderr,
            )
            return 1
        print("VM generated inventory is current")
        return 0

    OUTPUT_PATH.write_text(generated)
    print(f"wrote {OUTPUT_PATH.relative_to(ROOT)}")
    if errors:
        for error in errors:
            print(f"VM inventory error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
