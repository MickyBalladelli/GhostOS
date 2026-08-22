#!/usr/bin/env python3
"""Report new production Rust panic paths introduced by a review diff."""

from __future__ import annotations

import argparse
import os
import pathlib
import re
import subprocess
import sys
from dataclasses import dataclass


ROOT = pathlib.Path(__file__).resolve().parent.parent
PRODUCTION_ROOTS = ("boot/", "crates/", "kernel/", "tools/", "virtual_machine/")
IGNORED_PARTS = {"benches", "examples", "tests"}
HUNK_PATTERN = re.compile(r"^@@ -\d+(?:,\d+)? \+(?P<line>\d+)(?:,\d+)? @@")
PANIC_PATTERNS = (
    ("panic macro", re.compile(r"\bpanic\s*!\s*\(")),
    ("todo macro", re.compile(r"\btodo\s*!\s*\(")),
    ("unimplemented macro", re.compile(r"\bunimplemented\s*!\s*\(")),
    ("unreachable macro", re.compile(r"\bunreachable\s*!\s*\(")),
    ("assertion macro", re.compile(r"\b(?:debug_)?assert(?:_eq|_ne)?\s*!\s*\(")),
    ("unwrap call", re.compile(r"\.\s*unwrap\s*\(")),
    ("expect call", re.compile(r"\.\s*expect\s*\(")),
)


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    source: str


def is_production_source(path: str) -> bool:
    """Return whether a repository path is production Rust source."""

    normalized = path.replace("\\", "/")
    parts = pathlib.PurePosixPath(normalized).parts
    return (
        normalized.endswith(".rs")
        and normalized.startswith(PRODUCTION_ROOTS)
        and not any(part in IGNORED_PARTS for part in parts)
        and pathlib.PurePosixPath(normalized).name not in {"tests.rs", "test.rs"}
    )


def code_without_comments_and_literals(line: str) -> str:
    """Keep Rust code while removing comments, strings, and character literals."""

    output: list[str] = []
    index = 0
    in_block_comment = False

    while index < len(line):
        if in_block_comment:
            end = line.find("*/", index)
            if end < 0:
                break
            in_block_comment = False
            index = end + 2
            continue

        if line.startswith("//", index):
            break
        if line.startswith("/*", index):
            in_block_comment = True
            index += 2
            continue

        character = line[index]
        if character in {'"', "'"}:
            quote = character
            index += 1
            while index < len(line):
                if line[index] == "\\":
                    index += 2
                elif line[index] == quote:
                    index += 1
                    break
                else:
                    index += 1
            output.append(" ")
            continue

        if character == "r" and index + 1 < len(line) and line[index + 1] == '"':
            index += 2
            while index < len(line) and line[index] != '"':
                index += 1
            if index < len(line):
                index += 1
            output.append(" ")
            continue

        output.append(character)
        index += 1

    return "".join(output)


def findings_from_diff(diff: str) -> list[Finding]:
    """Find panic expressions on added lines in a unified Git diff."""

    findings: list[Finding] = []
    path = ""
    new_line = 0

    for raw_line in diff.splitlines():
        if raw_line.startswith("+++ b/"):
            path = raw_line[6:]
            continue
        if raw_line.startswith("+++ "):
            path = ""
            continue

        hunk = HUNK_PATTERN.match(raw_line)
        if hunk:
            new_line = int(hunk.group("line"))
            continue
        if not path or new_line == 0 or not is_production_source(path):
            continue

        if raw_line.startswith("+"):
            source = raw_line[1:]
            code = code_without_comments_and_literals(source)
            for kind, pattern in PANIC_PATTERNS:
                if pattern.search(code):
                    findings.append(Finding(path, new_line, kind, source.strip()))
            new_line += 1
        elif raw_line.startswith(" "):
            new_line += 1

    return findings


def git_diff(base: str | None) -> str:
    command = ["git", "diff", "--no-ext-diff", "--unified=0"]
    if base:
        command.append(base)
    command.extend(["--", "boot", "crates", "kernel", "tools", "virtual_machine"])
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, check=False)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "git diff failed")
    return result.stdout


def default_base() -> str | None:
    configured = os.environ.get("GHOSTOS_REVIEW_BASE")
    if configured:
        return configured

    result = subprocess.run(
        ["git", "symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode == 0:
        return result.stdout.strip()
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base",
        help="Git revision to compare with; defaults to GHOSTOS_REVIEW_BASE or origin/HEAD",
    )
    args = parser.parse_args()
    base = args.base or default_base()

    try:
        findings = findings_from_diff(git_diff(base))
    except RuntimeError as error:
        print(f"panic policy check failed: {error}", file=sys.stderr)
        return 2

    if findings:
        print("production panic paths introduced by this review:", file=sys.stderr)
        for finding in findings:
            print(
                f"- {finding.path}:{finding.line}: {finding.kind}: {finding.source}",
                file=sys.stderr,
            )
        print(
            "Convert the path to a stable error/status, or route it through "
            "the explicit fatal_kernel_halt boundary.",
            file=sys.stderr,
        )
        return 1

    comparison = base or "working tree"
    print(f"panic policy clean: no new production panic paths against {comparison}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
