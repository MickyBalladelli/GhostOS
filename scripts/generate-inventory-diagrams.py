#!/usr/bin/env python3
"""Generate the project inventory diagrams from Cargo and Rust source metadata."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "docs" / "inventory-diagrams.md"

MAGIC_PATTERN = re.compile(r'b"(SYN[A-Z0-9]{3,})"')
MAGIC_DECLARATION = re.compile(
    r"(?:pub\s+)?(?:const|static)\s+(\w*MAGIC\w*)\s*:[^=]+="
    r"\s*(?:\&\s*)?(?:\[[^\]]+\]\s*)?b\"(SYN[A-Z0-9]{3,})\""
)
VERSION_DECLARATION = re.compile(
    r"(?:pub\s+)?(?:const|static)\s+(\w*(?:VERSION|FORMAT|SCHEMA)\w*)\s*:[^=]+="
    r"\s*(\d+)"
)
CONST_DECLARATION = re.compile(
    r"(?:pub\s+)?const\s+(\w*(?:PROTOCOL_VERSION|SCHEMA(?:_VERSION)?)\w*)\s*:[^=]+=\s*(\d+)"
)
SERVICE_NAME_DECLARATION = re.compile(
    r"(?:pub\s+)?const\s+\w*SERVICE_NAME\w*\s*:\s*&str\s*=\s*\"([^\"]+)\""
)
TEST_FUNCTION = re.compile(
    r"#\[(?:test|tokio::test)\]\s*(?:pub\s+)?fn\s+([A-Za-z0-9_]+)"
)
ENUM_DECLARATION = re.compile(r"(?:pub\s+)?enum\s+(\w+)\s*\{")
STRUCT_DECLARATION = re.compile(r"(?:pub\s+)?struct\s+(\w+)\s*\([^)]*\)")
IMPL_DECLARATION = re.compile(r"impl\s+(\w+)\s*\{")
ENUM_VARIANT = re.compile(r"^\s*([A-Z][A-Za-z0-9_]*)\s*(?:=|\{|\(|,|$)")
ASSOCIATED_CONST = re.compile(r"(?:pub\s+)?const\s+([A-Z][A-Z0-9_]*)\s*:")


def relative(path: pathlib.Path) -> str:
    return path.relative_to(ROOT).as_posix()


def mermaid_id(prefix: str, value: str) -> str:
    safe = re.sub(r"[^A-Za-z0-9_]", "_", value)
    return f"{prefix}_{safe}"


def label(value: str) -> str:
    return value.replace('"', "'").replace("\n", " ")


def source_files(package: dict[str, object]) -> list[pathlib.Path]:
    manifest = pathlib.Path(str(package["manifest_path"])).resolve()
    source_root = manifest.parent / "src"
    return sorted(source_root.rglob("*.rs")) if source_root.is_dir() else []


def package_index(
    metadata: dict[str, object],
) -> tuple[list[dict[str, object]], dict[pathlib.Path, dict[str, object]]]:
    workspace_members = set(metadata["workspace_members"])
    packages = [
        package for package in metadata["packages"] if package["id"] in workspace_members
    ]
    packages.sort(key=lambda package: str(package["name"]))
    by_path = {
        pathlib.Path(str(package["manifest_path"])).resolve().parent: package
        for package in packages
    }
    return packages, by_path


def cargo_metadata() -> dict[str, object]:
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def matching_body(text: str, start: int) -> str:
    opening = text.find("{", start)
    if opening < 0:
        return ""
    depth = 0
    for index in range(opening, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return text[opening + 1 : index]
    return text[opening + 1 :]


def line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, max(offset, 0)) + 1


def crate_inventory(
    packages: list[dict[str, object]],
) -> tuple[dict[str, str], list[tuple[str, str]]]:
    nodes = {
        str(package["name"]): str(package.get("description") or package["name"])
        for package in packages
    }
    package_names = set(nodes)
    edges: set[tuple[str, str]] = set()
    for package in packages:
        source = str(package["name"])
        for dependency in package.get("dependencies", []):
            target = dependency.get("package") or dependency.get("name")
            if target in package_names:
                edges.add((source, str(target)))
    return nodes, sorted(edges)


def service_inventory(
    packages: list[dict[str, object]],
    files_by_package: dict[str, list[pathlib.Path]],
) -> list[dict[str, str]]:
    services = []
    for package in packages:
        package_name = str(package["name"])
        for path in files_by_package[package_name]:
            text = path.read_text()
            names = SERVICE_NAME_DECLARATION.findall(text)
            is_service_module = path.name in {"daemon.rs", "service.rs", "server.rs"}
            if not names and not is_service_module:
                continue
            service_name = names[0] if names else package_name
            role = path.stem if is_service_module else "service"
            marker = text.find("SERVICE_NAME") if names else 0
            services.append(
                {
                    "name": service_name,
                    "package": package_name,
                    "role": role,
                    "source": relative(path),
                    "line": str(line_number(text, marker)),
                }
            )
    unique = {
        (entry["name"], entry["package"], entry["source"]): entry
        for entry in services
    }
    return sorted(
        unique.values(),
        key=lambda entry: (entry["name"], entry["package"], entry["source"]),
    )


def protocol_inventory(
    packages: list[dict[str, object]],
    files_by_package: dict[str, list[pathlib.Path]],
) -> list[dict[str, str]]:
    protocols: list[dict[str, str]] = []
    for package in packages:
        package_name = str(package["name"])
        for path in files_by_package[package_name]:
            text = path.read_text()
            versions = {
                name: value for name, value in CONST_DECLARATION.findall(text)
            }
            if path.name == "protocol.rs":
                versions.setdefault("protocol module", "module")
            for enum_match in ENUM_DECLARATION.finditer(text):
                enum_name = enum_match.group(1)
                if enum_name not in {"TrafficClass", "Protocol"}:
                    continue
                body = matching_body(text, enum_match.start())
                variants = [
                    match.group(1)
                    for line in body.splitlines()
                    if (match := ENUM_VARIANT.match(line))
                ]
                if variants:
                    protocols.append(
                        {
                            "name": enum_name,
                            "version": ", ".join(variants),
                            "package": package_name,
                            "source": relative(path),
                            "line": str(line_number(text, enum_match.start())),
                        }
                    )
            for name, version in versions.items():
                protocols.append(
                    {
                        "name": name,
                        "version": version,
                        "package": package_name,
                        "source": relative(path),
                        "line": str(line_number(text, text.find(name))),
                    }
                )
    unique = {
        (entry["name"], entry["package"], entry["source"]): entry
        for entry in protocols
    }
    return sorted(
        unique.values(),
        key=lambda entry: (entry["package"], entry["name"], entry["source"]),
    )


def capability_inventory(
    packages: list[dict[str, object]],
    files_by_package: dict[str, list[pathlib.Path]],
) -> list[dict[str, str]]:
    capabilities: list[dict[str, str]] = []
    for package in packages:
        package_name = str(package["name"])
        for path in files_by_package[package_name]:
            text = path.read_text()
            declarations = list(ENUM_DECLARATION.finditer(text))
            struct_names = {
                match.group(1)
                for match in re.finditer(r"(?:pub\s+)?struct\s+(\w+)", text)
                if "Capability" in match.group(1) or "Rights" in match.group(1)
            }
            for declaration in declarations:
                type_name = declaration.group(1)
                if not ("Capability" in type_name or "Rights" in type_name):
                    continue
                if type_name.endswith("Error"):
                    continue
                body = matching_body(text, declaration.start())
                variants = [
                    match.group(1)
                    for line in body.splitlines()
                    if (match := ENUM_VARIANT.match(line))
                ]
                if not variants:
                    for impl_match in IMPL_DECLARATION.finditer(text):
                        if impl_match.group(1) != type_name:
                            continue
                        impl_body = matching_body(text, impl_match.start())
                        variants.extend(ASSOCIATED_CONST.findall(impl_body))
                if not variants:
                    continue
                capabilities.append(
                    {
                        "name": type_name,
                        "variants": ", ".join(sorted(set(variants))),
                        "package": package_name,
                        "source": relative(path),
                        "line": str(line_number(text, declaration.start())),
                    }
                )
            for type_name in sorted(struct_names):
                if any(declaration.group(1) == type_name for declaration in declarations):
                    continue
                variants = []
                for impl_match in IMPL_DECLARATION.finditer(text):
                    if impl_match.group(1) != type_name:
                        continue
                    impl_body = matching_body(text, impl_match.start())
                    variants.extend(ASSOCIATED_CONST.findall(impl_body))
                if variants:
                    marker = text.find(f"struct {type_name}")
                    capabilities.append(
                        {
                            "name": type_name,
                            "variants": ", ".join(sorted(set(variants))),
                            "package": package_name,
                            "source": relative(path),
                            "line": str(line_number(text, marker)),
                        }
                    )
    unique = {
        (entry["name"], entry["package"], entry["source"]): entry
        for entry in capabilities
    }
    return sorted(
        unique.values(),
        key=lambda entry: (entry["package"], entry["name"], entry["source"]),
    )


def storage_inventory(
    packages: list[dict[str, object]],
    files_by_package: dict[str, list[pathlib.Path]],
) -> list[dict[str, str]]:
    formats: list[dict[str, str]] = []
    for package in packages:
        package_name = str(package["name"])
        for path in files_by_package[package_name]:
            text = path.read_text()
            versions = [value for _, value in VERSION_DECLARATION.findall(text)]
            names_by_magic = {
                magic: name for name, magic in MAGIC_DECLARATION.findall(text)
            }
            seen: set[str] = set()
            for match in MAGIC_PATTERN.finditer(text):
                magic = match.group(1)
                if magic in seen:
                    continue
                seen.add(magic)
                version = (
                    versions[0]
                    if len(set(versions)) == 1
                    else ", ".join(sorted(set(versions)))
                )
                formats.append(
                    {
                        "name": names_by_magic.get(magic, magic),
                        "magic": magic,
                        "version": version,
                        "package": package_name,
                        "source": relative(path),
                        "line": str(line_number(text, match.start())),
                    }
                )
    unique = {
        (entry["magic"], entry["package"], entry["source"]): entry
        for entry in formats
    }
    return sorted(
        unique.values(),
        key=lambda entry: (entry["magic"], entry["package"], entry["source"]),
    )


def test_inventory(
    packages: list[dict[str, object]],
    files_by_package: dict[str, list[pathlib.Path]],
) -> list[dict[str, object]]:
    tests: list[dict[str, object]] = []
    for package in packages:
        package_name = str(package["name"])
        package_root = pathlib.Path(str(package["manifest_path"])).resolve().parent
        test_root = package_root / "tests"
        test_files = set(test_root.rglob("*.rs")) if test_root.is_dir() else set()
        test_files.update(
            path for path in files_by_package[package_name] if "#[test]" in path.read_text()
        )
        for path in sorted(test_files):
            text = path.read_text()
            names = TEST_FUNCTION.findall(text)
            tests.append(
                {
                    "package": package_name,
                    "source": relative(path),
                    "count": len(names),
                    "names": names,
                }
            )
    return sorted(tests, key=lambda entry: (str(entry["package"]), str(entry["source"])))


def graph(
    nodes: dict[str, str],
    edges: list[tuple[str, str]],
    prefix: str,
    direction: str = "LR",
) -> list[str]:
    lines = [chr(96) * 3 + "mermaid", f"graph {direction}"]
    for node_id, node_label in sorted(nodes.items()):
        lines.append(
            f'    {mermaid_id(prefix, node_id)}["{label(node_label)}"]'
        )
    for source, target in sorted(set(edges)):
        lines.append(
            f"    {mermaid_id(prefix, source)} --> {mermaid_id(prefix, target)}"
        )
    lines.append(chr(96) * 3)
    return lines


def render_graphs(
    crates: tuple[dict[str, str], list[tuple[str, str]]],
    services: list[dict[str, str]],
    protocols: list[dict[str, str]],
    capabilities: list[dict[str, str]],
    formats: list[dict[str, str]],
    tests: list[dict[str, object]],
) -> list[str]:
    crate_nodes, crate_edges = crates
    lines: list[str] = []
    lines.extend(["## Crates", "", *graph(crate_nodes, crate_edges, "crate"), ""])

    service_nodes = {
        f"{entry['name']}::{entry['source']}": f"{entry['name']} ({entry['role']})"
        for entry in services
    }
    service_nodes.update(
        {f"crate::{name}": name for name in {entry["package"] for entry in services}}
    )
    service_edges = [
        (
            f"{entry['name']}::{entry['source']}",
            f"crate::{entry['package']}",
        )
        for entry in services
    ]
    lines.extend(["## Services", "", *graph(service_nodes, service_edges, "service"), ""])

    protocol_nodes = {
        f"{entry['name']}::{entry['package']}::{entry['source']}":
        f"{entry['name']} {entry['version']}"
        for entry in protocols
    }
    protocol_nodes.update(
        {f"crate::{entry['package']}": entry["package"] for entry in protocols}
    )
    protocol_edges = [
        (
            f"crate::{entry['package']}",
            f"{entry['name']}::{entry['package']}::{entry['source']}",
        )
        for entry in protocols
    ]
    lines.extend(["## Protocols", "", *graph(protocol_nodes, protocol_edges, "protocol"), ""])

    capability_nodes = {
        f"{entry['name']}::{entry['package']}::{entry['source']}":
        f"{entry['name']}: {entry['variants']}"
        for entry in capabilities
    }
    capability_nodes.update(
        {f"crate::{entry['package']}": entry["package"] for entry in capabilities}
    )
    capability_edges = [
        (
            f"crate::{entry['package']}",
            f"{entry['name']}::{entry['package']}::{entry['source']}",
        )
        for entry in capabilities
    ]
    lines.extend(["## Capabilities", "", *graph(capability_nodes, capability_edges, "capability"), ""])

    format_nodes = {
        f"{entry['magic']}::{entry['package']}::{entry['source']}":
        f"{entry['name']} {entry['magic']} v{entry['version'] or '?'}"
        for entry in formats
    }
    format_nodes.update(
        {f"crate::{entry['package']}": entry["package"] for entry in formats}
    )
    format_edges = [
        (
            f"crate::{entry['package']}",
            f"{entry['magic']}::{entry['package']}::{entry['source']}",
        )
        for entry in formats
    ]
    lines.extend(["## Storage formats", "", *graph(format_nodes, format_edges, "format"), ""])

    test_nodes: dict[str, str] = {}
    test_edges: list[tuple[str, str]] = []
    for entry in tests:
        test_key = f"test::{entry['package']}::{entry['source']}"
        test_nodes[test_key] = f"{entry['source']} ({entry['count']} tests)"
        crate_key = f"crate::{entry['package']}"
        test_nodes[crate_key] = str(entry["package"])
        test_edges.append((crate_key, test_key))
    lines.extend(["## Tests", "", *graph(test_nodes, test_edges, "test"), ""])
    return lines


def source_table(
    title: str,
    entries: list[dict[str, str]],
    columns: list[tuple[str, str]],
) -> list[str]:
    lines = [
        f"### {title}",
        "",
        "| " + " | ".join(name for name, _ in columns) + " |",
        "| " + " | ".join("---" for _ in columns) + " |",
    ]
    for entry in entries:
        values = []
        for _, key in columns:
            value = str(entry.get(key, ""))
            if key == "source":
                value = (
                    f"[{chr(96)}{value}:{entry.get('line', '?')}{chr(96)}]"
                    f"(../{value}#L{entry.get('line', '?')})"
                )
            values.append(value.replace("|", "\\|"))
        lines.append("| " + " | ".join(values) + " |")
    return lines + [""]


def render(
    packages: list[dict[str, object]],
    crates: tuple[dict[str, str], list[tuple[str, str]]],
    services: list[dict[str, str]],
    protocols: list[dict[str, str]],
    capabilities: list[dict[str, str]],
    formats: list[dict[str, str]],
    tests: list[dict[str, object]],
) -> str:
    test_entries = [
        {
            "package": entry["package"],
            "source": entry["source"],
            "line": "1",
            "count": str(entry["count"]),
        }
        for entry in tests
    ]
    lines = [
        "<!-- Generated by scripts/generate-inventory-diagrams.py. -->",
        "<!-- Do not edit this file; update Cargo manifests or Rust source metadata. -->",
        "",
        "# SynOS source inventory diagrams",
        "",
        "This file is generated from workspace Cargo metadata, Rust declarations, and test source files.",
        "It has no hand-maintained inventory list. Run "
        + chr(96)
        + "python3 scripts/generate-inventory-diagrams.py"
        + chr(96)
        + " after source changes; local validation uses "
        + chr(96)
        + "--check"
        + chr(96)
        + " to reject stale output.",
        "",
        "| Inventory | Entries | Source metadata |",
        "| --- | ---: | --- |",
        f"| Crates | {len(packages)} | Cargo workspace packages and path dependencies |",
        f"| Services | {len(services)} | service.rs, daemon.rs, server.rs, and *_SERVICE_NAME constants |",
        f"| Protocols | {len(protocols)} | protocol modules, protocol/schema version constants, and traffic enums |",
        f"| Capabilities | {len(capabilities)} | capability/rights enums, structs, and associated constants |",
        f"| Storage formats | {len(formats)} | SYN* magic literals and nearby format/version constants |",
        f"| Test files | {len(tests)} | Rust tests/ files and source files with #[test] functions |",
        "",
        *render_graphs(crates, services, protocols, capabilities, formats, tests),
        "## Source index",
        "",
        *source_table(
            "Services",
            services,
            [("Service", "name"), ("Package", "package"), ("Source", "source")],
        ),
        *source_table(
            "Protocols",
            protocols,
            [
                ("Protocol", "name"),
                ("Version or variants", "version"),
                ("Package", "package"),
                ("Source", "source"),
            ],
        ),
        *source_table(
            "Capabilities",
            capabilities,
            [
                ("Type", "name"),
                ("Variants or rights", "variants"),
                ("Package", "package"),
                ("Source", "source"),
            ],
        ),
        *source_table(
            "Storage formats",
            formats,
            [
                ("Format", "name"),
                ("Magic", "magic"),
                ("Version", "version"),
                ("Package", "package"),
                ("Source", "source"),
            ],
        ),
        *source_table(
            "Test files",
            test_entries,
            [("Package", "package"), ("Test file", "source"), ("Tests", "count")],
        ),
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="fail when the generated file is stale")
    mode.add_argument("--update", action="store_true", help="write the generated file")
    args = parser.parse_args()

    metadata = cargo_metadata()
    packages, _ = package_index(metadata)
    files_by_package = {
        str(package["name"]): source_files(package) for package in packages
    }
    crates = crate_inventory(packages)
    services = service_inventory(packages, files_by_package)
    protocols = protocol_inventory(packages, files_by_package)
    capabilities = capability_inventory(packages, files_by_package)
    formats = storage_inventory(packages, files_by_package)
    tests = test_inventory(packages, files_by_package)
    generated = render(
        packages,
        crates,
        services,
        protocols,
        capabilities,
        formats,
        tests,
    )

    if args.check:
        current = OUTPUT.read_text() if OUTPUT.is_file() else ""
        if current != generated + "\n":
            print(
                "inventory diagrams are stale; run "
                "python3 scripts/generate-inventory-diagrams.py",
                file=sys.stderr,
            )
            return 1
        print(f"inventory diagrams current: {relative(OUTPUT)}")
        return 0

    OUTPUT.write_text(generated + "\n")
    print(f"wrote {relative(OUTPUT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
