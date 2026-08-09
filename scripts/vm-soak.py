#!/usr/bin/env python3
"""Repeat VM teardown checks and report host-resource leaks."""

from __future__ import annotations

import argparse
import csv
import json
import os
import pathlib
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone


ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_COMMAND = [
    "cargo",
    "test",
    "-p",
    "synos-vm",
    "--test",
    "soak_leaks",
    "--",
    "--nocapture",
]
TRANSLATION_MARKER = "soak_translation_cache_and_terminal_state_release"
DEFAULT_MEMORY_TOLERANCE = 64 * 1024 * 1024


def now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def positive_env(name: str, default: int) -> int:
    value = os.environ.get(name, str(default))
    try:
        parsed = int(value)
    except ValueError as error:
        raise ValueError(f"{name} must be a positive integer") from error
    if parsed < 1:
        raise ValueError(f"{name} must be a positive integer")
    return parsed


def rss_bytes(pid: int) -> int | None:
    if os.name == "nt":
        try:
            import ctypes
            from ctypes import wintypes

            class Counters(ctypes.Structure):
                _fields_ = [
                    ("cb", wintypes.DWORD),
                    ("page_fault_count", wintypes.DWORD),
                    ("peak_working_set_size", ctypes.c_size_t),
                    ("working_set_size", ctypes.c_size_t),
                    ("quota_peak_paged_pool_usage", ctypes.c_size_t),
                    ("quota_paged_pool_usage", ctypes.c_size_t),
                    ("quota_peak_non_paged_pool_usage", ctypes.c_size_t),
                    ("quota_non_paged_pool_usage", ctypes.c_size_t),
                    ("pagefile_usage", ctypes.c_size_t),
                    ("peak_pagefile_usage", ctypes.c_size_t),
                ]

            process = ctypes.windll.kernel32.OpenProcess(0x0400 | 0x0010, False, pid)
            if not process:
                return None
            counters = Counters()
            counters.cb = ctypes.sizeof(counters)
            result = ctypes.windll.psapi.GetProcessMemoryInfo(
                process, ctypes.byref(counters), ctypes.sizeof(counters)
            )
            ctypes.windll.kernel32.CloseHandle(process)
            return counters.working_set_size if result else None
        except (AttributeError, OSError):
            return None
    status_path = pathlib.Path(f"/proc/{pid}/status")
    if status_path.is_file():
        for line in status_path.read_text(errors="replace").splitlines():
            if line.startswith("VmRSS:"):
                fields = line.split()
                return int(fields[1]) * 1024 if len(fields) > 1 else None
    try:
        result = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)],
            capture_output=True,
            text=True,
            check=False,
        )
        value = result.stdout.strip()
        if value:
            return int(value) * 1024
    except (OSError, ValueError):
        pass
    return None


def process_snapshot() -> list[dict[str, str]]:
    if os.name == "nt":
        try:
            result = subprocess.run(
                [
                    "powershell",
                    "-NoProfile",
                    "-Command",
                    "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name | ConvertTo-Json -Compress",
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            if result.returncode == 0 and result.stdout.strip():
                value = json.loads(result.stdout)
                rows = value if isinstance(value, list) else [value]
                return [
                    {
                        "pid": str(row["ProcessId"]),
                        "ppid": str(row["ParentProcessId"]),
                        "command": str(row["Name"]),
                    }
                    for row in rows
                ]
        except (OSError, json.JSONDecodeError, KeyError, TypeError):
            pass
    try:
        result = subprocess.run(
            ["ps", "-axo", "pid=,ppid=,command="],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode == 0:
            rows = []
            for line in result.stdout.splitlines():
                fields = line.strip().split(None, 2)
                if len(fields) == 3:
                    rows.append({"pid": fields[0], "ppid": fields[1], "command": fields[2]})
            return rows
    except OSError:
        pass
    try:
        result = subprocess.run(
            ["tasklist", "/fo", "csv", "/nh"],
            capture_output=True,
            text=True,
            check=False,
        )
        return [
            {"pid": row[1], "ppid": "?", "command": row[0]}
            for row in csv.reader(result.stdout.splitlines())
            if len(row) > 1
        ]
    except OSError:
        return []


def child_processes(rows: list[dict[str, str]], root_pid: int) -> list[dict[str, str]]:
    descendants: list[dict[str, str]] = []
    pending = {str(root_pid)}
    while pending:
        parents = pending
        pending = set()
        for row in rows:
            if row["ppid"] in parents and row not in descendants:
                descendants.append(row)
                pending.add(row["pid"])
    return descendants


def resource_snapshot(root: pathlib.Path) -> dict[str, object]:
    files = 0
    bytes_total = 0
    sockets = 0
    locks = 0
    paths: list[str] = []
    if root.exists():
        for path in root.rglob("*"):
            try:
                info = path.lstat()
            except OSError:
                continue
            relative = path.relative_to(root).as_posix()
            if stat.S_ISSOCK(info.st_mode):
                sockets += 1
                paths.append(relative)
            elif stat.S_ISREG(info.st_mode):
                files += 1
                bytes_total += info.st_size
                if ".lock" in path.name or path.name.endswith(".lck"):
                    locks += 1
                paths.append(relative)
    return {
        "files": files,
        "bytes": bytes_total,
        "sockets": sockets,
        "locks": locks,
        "paths": sorted(paths),
    }


def terminal_state() -> str | None:
    if not sys.stdin.isatty():
        return None
    try:
        result = subprocess.run(["stty", "-g"], capture_output=True, text=True, check=False)
    except OSError:
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def run_once(
    command: list[str],
    run_number: int,
    scratch: pathlib.Path,
    report_dir: pathlib.Path,
    environment: dict[str, str],
    memory_tolerance: int,
    require_translation_marker: bool,
) -> dict[str, object]:
    before = resource_snapshot(scratch)
    terminal_before = terminal_state()
    process_before = child_processes(process_snapshot(), os.getpid())
    rss_before = rss_bytes(os.getpid())
    stdout_path = report_dir / f"run-{run_number}.stdout.log"
    stderr_path = report_dir / f"run-{run_number}.stderr.log"
    started = time.monotonic()
    peak_child_rss = 0
    status = 1
    error = None
    try:
        with stdout_path.open("w") as stdout, stderr_path.open("w") as stderr:
            child = subprocess.Popen(command, cwd=ROOT, env=environment, stdout=stdout, stderr=stderr)
            while child.poll() is None:
                current_rss = rss_bytes(child.pid)
                if current_rss is not None:
                    peak_child_rss = max(peak_child_rss, current_rss)
                time.sleep(0.05)
            status = child.wait()
    except OSError as exception:
        error = str(exception)
    after = resource_snapshot(scratch)
    terminal_after = terminal_state()
    process_after = child_processes(process_snapshot(), os.getpid())
    rss_after = rss_bytes(os.getpid())
    stdout_text = stdout_path.read_text(errors="replace") if stdout_path.exists() else ""
    findings: list[str] = []
    for key in ("files", "sockets", "locks"):
        if after[key]:
            findings.append(f"{key} remain in soak scratch space: {after[key]}")
    if process_after:
        findings.append(f"child processes remain after run: {process_after}")
    if terminal_before is not None and terminal_after != terminal_before:
        findings.append("host terminal mode changed during VM soak")
    if rss_before is not None and rss_after is not None and rss_after > rss_before + memory_tolerance:
        findings.append(
            f"runner memory grew by {rss_after - rss_before} bytes (tolerance {memory_tolerance})"
        )
    if require_translation_marker and TRANSLATION_MARKER not in stdout_text:
        findings.append(f"soak output did not contain {TRANSLATION_MARKER}")
    if status != 0:
        findings.append(f"soak command exited with status {status}")
    if error:
        findings.append(error)
    return {
        "run": run_number,
        "state": "passed" if not findings else "failed",
        "exit_code": status,
        "duration_seconds": round(time.monotonic() - started, 3),
        "resource_before": before,
        "resource_after": after,
        "process_before": len(process_before),
        "process_after": len(process_after),
        "memory": {
            "runner_before_bytes": rss_before,
            "runner_after_bytes": rss_after,
            "child_peak_bytes": peak_child_rss or None,
        },
        "terminal_state": "unchanged" if terminal_before is not None else "non-tty",
        "translation_cache": {
            "state": "passed" if TRANSLATION_MARKER in stdout_text else "not-observed",
            "test": TRANSLATION_MARKER,
        },
        "findings": findings,
        "stdout": str(stdout_path),
        "stderr": str(stderr_path),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=pathlib.Path, default=ROOT / "build/vm-soak/report.json")
    args = parser.parse_args()
    try:
        runs = positive_env("SYNOS_VM_SOAK_RUNS", 3)
        inner_runs = positive_env("SYNOS_VM_SOAK_INNER_RUNS", 32)
        memory_tolerance = positive_env(
            "SYNOS_VM_SOAK_MEMORY_TOLERANCE_BYTES", DEFAULT_MEMORY_TOLERANCE
        )
    except ValueError as error:
        print(error, file=sys.stderr)
        return 2
    command_value = os.environ.get("SYNOS_VM_SOAK_COMMAND")
    try:
        command = shlex.split(command_value) if command_value else DEFAULT_COMMAND
    except ValueError as error:
        print(f"invalid SYNOS_VM_SOAK_COMMAND: {error}", file=sys.stderr)
        return 2
    if not command:
        print("SYNOS_VM_SOAK_COMMAND must not be empty", file=sys.stderr)
        return 2

    args.report.parent.mkdir(parents=True, exist_ok=True)
    scratch = pathlib.Path(tempfile.mkdtemp(prefix="synos-vm-soak-"))
    environment = os.environ.copy()
    environment["SYNOS_VM_SOAK_INNER_RUNS"] = str(inner_runs)
    for variable in ("TMPDIR", "TMP", "TEMP"):
        environment[variable] = str(scratch)
    report: dict[str, object] = {
        "schema": 1,
        "started_at": now(),
        "runs_requested": runs,
        "inner_runs_requested": inner_runs,
        "memory_tolerance_bytes": memory_tolerance,
        "command": " ".join(command),
        "resource_checks": ["files", "sockets", "processes", "locks", "memory", "translation-cache", "terminal-state"],
        "runs": [],
    }
    failures = 0
    try:
        for run_number in range(1, runs + 1):
            print(f"VM soak run {run_number}/{runs}", flush=True)
            result = run_once(
                command,
                run_number,
                scratch,
                args.report.parent,
                environment,
                memory_tolerance,
                command == DEFAULT_COMMAND,
            )
            report["runs"].append(result)
            if result["state"] != "passed":
                failures += 1
                break
    finally:
        report["ended_at"] = now()
        report["result_state"] = "passed" if failures == 0 and len(report["runs"]) == runs else "failed"
        report["scratch"] = str(scratch)
        if failures == 0:
            shutil.rmtree(scratch, ignore_errors=True)
            report["scratch"] = "removed"
        args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    if failures:
        print(f"VM soak failed; report: {args.report}", file=sys.stderr)
        return 1
    print(f"VM soak passed: {runs} runs; report: {args.report}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
