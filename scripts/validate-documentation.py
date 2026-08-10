#!/usr/bin/env python3
"""Check documentation maps, links, generated inventories, and test coverage maps."""

from __future__ import annotations

import re
import subprocess
import sys
import tomllib
from pathlib import Path
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parent.parent
BOOK = ROOT / "book"
MARKDOWN_LINK = re.compile(r"(?<!!)\[[^\]]+\]\(([^)\n]+)\)")
SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
CATALOG_ROW = re.compile(r"^[|] +\x60([^\x60]+)\x60 +[|]", re.MULTILINE)


def relative(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def link_target(raw_target: str) -> str:
    target = raw_target.strip()
    if target.startswith("<") and ">" in target:
        return target[1 : target.index(">")]
    return target.split()[0]


def heading_slugs(path: Path) -> set[str]:
    slugs: set[str] = set()
    counts: dict[str, int] = {}
    for line in path.read_text().splitlines():
        match = re.match(r"^#{1,6}[ \t]+(.+?)[ \t]*#?[ \t]*$", line)
        if not match:
            continue
        title = re.sub(r"<[^>]*>", "", match.group(1).strip().lower())
        slug = re.sub(r"[^\w\s-]", "", title)
        slug = re.sub(r"\s+", "-", slug)
        occurrence = counts.get(slug, 0)
        counts[slug] = occurrence + 1
        slugs.add(slug if occurrence == 0 else f"{slug}-{occurrence}")
    return slugs


def check_links(paths: list[Path], errors: list[str]) -> None:
    cached_slugs: dict[Path, set[str]] = {}
    for path in paths:
        in_fence = False
        for line_number, line in enumerate(path.read_text().splitlines(), start=1):
            if line.lstrip().startswith((chr(96) * 3, "~" * 3)):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            for match in MARKDOWN_LINK.finditer(line):
                target = link_target(match.group(1))
                if not target or SCHEME.match(target) or target.startswith("//"):
                    continue
                path_part, separator, fragment = target.partition("#")
                source_path = path if not path_part else path.parent / unquote(path_part)
                try:
                    resolved = source_path.resolve()
                    resolved.relative_to(ROOT.resolve())
                except ValueError:
                    errors.append(f"{relative(path)}:{line_number}: link escapes repository: {target}")
                    continue
                if not resolved.exists():
                    errors.append(f"{relative(path)}:{line_number}: stale link target: {target}")
                    continue
                if separator and fragment and not re.fullmatch(r"L\d+(?:-L\d+)?", fragment):
                    if resolved not in cached_slugs:
                        cached_slugs[resolved] = heading_slugs(resolved)
                    if unquote(fragment).lower() not in cached_slugs[resolved]:
                        errors.append(f"{relative(path)}:{line_number}: stale link anchor: {target}")


def check_book_index(errors: list[str]) -> None:
    index = (BOOK / "README.md").read_text()
    expected = {path.name for path in BOOK.glob("[0-9][0-9]-*.md")}
    expected.update(path.name for path in BOOK.glob("appendix-*.md"))
    linked = {
        Path(link).name
        for link in re.findall(r"\]\(([^)#\s]+\.md)", index)
        if Path(link).name in expected
    }
    if linked != expected:
        errors.append(
            "book/README.md table of contents is out of sync: "
            f"missing={sorted(expected - linked)}, extra={sorted(linked - expected)}"
        )


def workspace_packages() -> set[str]:
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    packages: set[str] = set()
    for pattern in cargo.get("workspace", {}).get("members", []):
        for match in ROOT.glob(pattern):
            manifest = match if match.name == "Cargo.toml" else match / "Cargo.toml"
            if manifest.is_file():
                package = tomllib.loads(manifest.read_text()).get("package", {})
                if package.get("name"):
                    packages.add(str(package["name"]))
    return packages


def check_crate_catalog(errors: list[str]) -> None:
    catalog = (BOOK / "appendix-a-crate-catalog.md").read_text()
    listed = set(CATALOG_ROW.findall(catalog))
    allowed_non_workspace = {"synos-fuzz"}
    packages = workspace_packages()
    missing = sorted(packages - listed)
    unknown = sorted(listed - packages - allowed_non_workspace)
    if missing:
        errors.append(f"book/appendix-a-crate-catalog.md missing workspace packages: {missing}")
    if unknown:
        errors.append(f"book/appendix-a-crate-catalog.md names unknown packages: {unknown}")


def run_check(label: str, command: list[str], errors: list[str]) -> None:
    result = subprocess.run(command, cwd=ROOT, check=False)
    if result.returncode:
        errors.append(f"{label} failed with exit status {result.returncode}")


def main() -> int:
    errors: list[str] = []
    markdown_paths = [
        ROOT / "README.md",
        *sorted(BOOK.glob("*.md")),
        *sorted((ROOT / "docs").glob("*.md")),
    ]
    check_links(markdown_paths, errors)
    check_book_index(errors)
    check_crate_catalog(errors)

    run_check(
        "roadmap metadata",
        [sys.executable, str(ROOT / "scripts" / "validate-roadmap-metadata.py")],
        errors,
    )
    run_check(
        "test inventory",
        [sys.executable, str(ROOT / "scripts" / "validate-test-inventory.py")],
        errors,
    )
    run_check(
        "generated inventory diagrams",
        [sys.executable, str(ROOT / "scripts" / "generate-inventory-diagrams.py"), "--check"],
        errors,
    )
    run_check(
        "roadmap validation",
        [sys.executable, str(ROOT / "scripts" / "validate-roadmaps.py")],
        errors,
    )
    run_check(
        "review baseline",
        [sys.executable, str(ROOT / "scripts" / "validate-review-baseline.py")],
        errors,
    )

    if errors:
        for error in errors:
            print(f"documentation error: {error}", file=sys.stderr)
        return 1
    print(
        "documentation valid: "
        f"{len(markdown_paths)} Markdown files, book map, source maps, and test inventory"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
