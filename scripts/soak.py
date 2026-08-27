#!/usr/bin/env python3
"""Run bounded workflow soaks and report resource drift."""

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
DEFAULT_MEMORY_TOLERANCE = 64 * 1024 * 1024
DEFAULT_TIMEOUT_SECONDS = 300.0

SCENARIOS: dict[str, dict[str, object]] = {
    "boot": {
        "description": "boot contract and image metadata workflow",
        "command": ["cargo", "test", "-p", "ghostos-test-support", "--test", "boot_contracts", "--", "--nocapture"],
    },
    "shell": {
        "description": "shell command, job, and terminal workflow",
        "command": ["cargo", "test", "-p", "ghostos-shell", "--test", "line_editor_utf8_and_history", "--", "--nocapture"],
    },
    "filesystem": {
        "description": "GhostFS persistence and restart workflow",
        "command": ["cargo", "test", "-p", "ghostos-ghostfs", "--test", "persistence", "--", "--nocapture"],
    },
    "network": {
        "description": "network lease, rollback, and client workflow",
        "command": ["cargo", "test", "-p", "ghostos-netd", "--test", "dhcp_client", "--", "--nocapture"],
    },
    "compiler": {
        "description": "compiler cancellation, timeout, and recovery workflow",
        "command": ["cargo", "test", "-p", "ghostos-rustd", "--test", "model", "--", "--nocapture"],
    },
    "cluster": {
        "description": "storage cluster membership and recovery workflow",
        "command": ["cargo", "test", "-p", "ghostos-storaged", "--test", "storage_queue_and_nvme", "--", "--nocapture"],
    },
    "vm": {
        "description": "VM translation cache and terminal teardown workflow",
        "command": ["cargo", "test", "-p", "ghostos-vm", "--test", "soak_leaks", "--", "--nocapture"],
    },
    "lifecycle": {
        "description": "reboot, suspend/resume, memory-hotplug, and service-restart ownership campaign",
        "command": ["cargo", "test", "-p", "ghostos-vm", "--test", "lifecycle_soak", "--", "--nocapture"],
    },
    "capabilities": {
        "description": "capability slot reclamation, generation reuse, and stale-handle rejection campaign",
        "command": ["cargo", "test", "-p", "ghostos-kernel", "--test", "capability_soak", "--", "--nocapture"],
    },
}


def now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def positive_int(name: str, value: str | None, default: int) -> int:
    raw = value if value is not None else os.environ.get(name, str(default))
    try:
        parsed = int(raw)
    except ValueError as error:
        raise ValueError(f"{name} must be a positive integer") from error
    if parsed < 1:
        raise ValueError(f"{name} must be a positive integer")
    return parsed


def positive_float(name: str, value: str | None, default: float) -> float:
    raw = value if value is not None else os.environ.get(name, str(default))
    try:
        parsed = float(raw)
    except ValueError as error:
        raise ValueError(f"{name} must be positive") from error
    if parsed <= 0:
        raise ValueError(f"{name} must be positive")
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
        return int(value) * 1024 if value else None
    except (OSError, ValueError):
        return None


def fd_count(pid: int) -> int | None:
    path = pathlib.Path(f"/proc/{pid}/fd")
    if not path.is_dir():
        return None
    try:
        return sum(1 for _ in path.iterdir())
    except OSError:
        return None


def cpu_seconds() -> float | None:
    try:
        import resource

        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        return round(usage.ru_utime + usage.ru_stime, 6)
    except (ImportError, OSError):
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
        result = subprocess.run(["tasklist", "/fo", "csv", "/nh"], capture_output=True, text=True, check=False)
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


def numeric_delta(before: dict[str, object], after: dict[str, object]) -> dict[str, int]:
    return {
        key: int(after[key]) - int(before[key])
        for key in ("files", "bytes", "sockets", "locks")
    }


def scenario_command(name: str) -> list[str]:
    value = os.environ.get(f"GHOSTOS_SOAK_{name.upper()}_COMMAND")
    if value is None:
        return list(SCENARIOS[name]["command"])
    try:
        command = shlex.split(value)
    except ValueError as error:
        raise ValueError(f"invalid GHOSTOS_SOAK_{name.upper()}_COMMAND: {error}") from error
    if not command:
        raise ValueError(f"GHOSTOS_SOAK_{name.upper()}_COMMAND must not be empty")
    return command


def run_once(
    name: str,
    command: list[str],
    run_number: int,
    scratch: pathlib.Path,
    report_dir: pathlib.Path,
    environment: dict[str, str],
    memory_tolerance: int,
    timeout_seconds: float,
) -> dict[str, object]:
    run_scratch = scratch / name / f"run-{run_number}"
    run_scratch.mkdir(parents=True, exist_ok=True)
    stdout_path = report_dir / f"run-{run_number}.stdout.log"
    stderr_path = report_dir / f"run-{run_number}.stderr.log"
    before = resource_snapshot(run_scratch)
    terminal_before = terminal_state()
    process_before = child_processes(process_snapshot(), os.getpid())
    process_before_pids = {row["pid"] for row in process_before}
    rss_before = rss_bytes(os.getpid())
    fd_before = fd_count(os.getpid())
    cpu_before = cpu_seconds()
    started = time.monotonic()
    peak_child_rss = 0
    status = 1
    error = None
    timed_out = False
    lifecycle_report = None
    capability_report = None
    try:
        with stdout_path.open("w") as stdout, stderr_path.open("w") as stderr:
            run_environment = environment
            if name == "lifecycle":
                run_environment = environment.copy()
                lifecycle_report = report_dir / f"run-{run_number}.lifecycle.json"
                run_environment["GHOSTOS_LIFECYCLE_REPORT"] = str(lifecycle_report)
            elif name == "capabilities":
                run_environment = environment.copy()
                capability_report = report_dir / f"run-{run_number}.capabilities.json"
                run_environment["GHOSTOS_CAPABILITY_SOAK_REPORT"] = str(capability_report)
            child = subprocess.Popen(command, cwd=ROOT, env=run_environment, stdout=stdout, stderr=stderr)
            while child.poll() is None:
                current_rss = rss_bytes(child.pid)
                if current_rss is not None:
                    peak_child_rss = max(peak_child_rss, current_rss)
                if time.monotonic() - started > timeout_seconds:
                    timed_out = True
                    child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=5)
                    break
                time.sleep(0.05)
            status = child.wait()
    except OSError as exception:
        error = str(exception)
    after = resource_snapshot(run_scratch)
    terminal_after = terminal_state()
    process_after = child_processes(process_snapshot(), os.getpid())
    new_process_after = [row for row in process_after if row["pid"] not in process_before_pids]
    rss_after = rss_bytes(os.getpid())
    fd_after = fd_count(os.getpid())
    cpu_after = cpu_seconds()
    findings: list[str] = []
    for key in ("files", "sockets", "locks"):
        if after[key]:
            findings.append(f"{key} remain in soak scratch space: {after[key]}")
    if new_process_after:
        findings.append(f"child processes remain after run: {new_process_after}")
    if terminal_before is not None and terminal_after != terminal_before:
        findings.append("host terminal mode changed during soak")
    if rss_before is not None and rss_after is not None and rss_after > rss_before + memory_tolerance:
        findings.append(
            f"runner RSS grew by {rss_after - rss_before} bytes (tolerance {memory_tolerance})"
        )
    if timed_out:
        findings.append(f"soak command exceeded {timeout_seconds:g} seconds and was terminated")
    if status != 0:
        findings.append(f"soak command exited with status {status}")
    if error:
        findings.append(error)
    return {
        "run": run_number,
        "state": "passed" if not findings else "failed",
        "exit_code": status,
        "duration_seconds": round(time.monotonic() - started, 3),
        "command": command,
        "resource_drift": {
            "scratch": {
                "before": before,
                "after": after,
                "delta": numeric_delta(before, after),
            },
            "runner": {
                "rss_before_bytes": rss_before,
                "rss_after_bytes": rss_after,
                "rss_delta_bytes": rss_after - rss_before if rss_before is not None and rss_after is not None else None,
                "fd_before": fd_before,
                "fd_after": fd_after,
                "fd_delta": fd_after - fd_before if fd_before is not None and fd_after is not None else None,
                "child_cpu_before_seconds": cpu_before,
                "child_cpu_after_seconds": cpu_after,
                "child_cpu_delta_seconds": round(cpu_after - cpu_before, 6) if cpu_before is not None and cpu_after is not None else None,
                "child_peak_rss_bytes": peak_child_rss or None,
            },
        },
        "process_before": process_before,
        "process_after": new_process_after,
        "terminal_state": "unchanged" if terminal_before is not None else "non-tty",
        "findings": findings,
        "stdout": str(stdout_path),
        "stderr": str(stderr_path),
        **({"lifecycle_report": str(lifecycle_report)} if lifecycle_report is not None else {}),
        **({"capability_report": str(capability_report)} if capability_report is not None else {}),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", action="append", choices=sorted(SCENARIOS))
    parser.add_argument("--runs", type=int)
    parser.add_argument("--timeout-seconds", type=float)
    parser.add_argument("--memory-tolerance-bytes", type=int)
    parser.add_argument(
        "--report",
        type=pathlib.Path,
        default=pathlib.Path(os.environ.get("GHOSTOS_SOAK_REPORT", ROOT / "build/soak/report.json")),
    )
    args = parser.parse_args()
    try:
        runs = positive_int("GHOSTOS_SOAK_RUNS", str(args.runs) if args.runs is not None else None, 3)
        timeout_seconds = positive_float(
            "GHOSTOS_SOAK_TIMEOUT_SECONDS",
            str(args.timeout_seconds) if args.timeout_seconds is not None else None,
            DEFAULT_TIMEOUT_SECONDS,
        )
        memory_tolerance = positive_int(
            "GHOSTOS_SOAK_MEMORY_TOLERANCE_BYTES",
            str(args.memory_tolerance_bytes) if args.memory_tolerance_bytes is not None else None,
            DEFAULT_MEMORY_TOLERANCE,
        )
        names = args.scenario or list(SCENARIOS)
        commands = {name: scenario_command(name) for name in names}
    except ValueError as error:
        print(error, file=sys.stderr)
        return 2

    report_path = args.report if args.report.is_absolute() else ROOT / args.report
    report_path.parent.mkdir(parents=True, exist_ok=True)
    scratch = pathlib.Path(tempfile.mkdtemp(prefix="ghostos-soak-"))
    environment = os.environ.copy()
    for variable in ("TMPDIR", "TMP", "TEMP"):
        environment[variable] = str(scratch)
    report: dict[str, object] = {
        "schema": 1,
        "started_at": now(),
        "runs_requested_per_scenario": runs,
        "timeout_seconds": timeout_seconds,
        "memory_tolerance_bytes": memory_tolerance,
        "scenarios": {
            name: {
                "description": SCENARIOS[name]["description"],
                "command": commands[name],
                "runs": [],
            }
            for name in names
        },
        "resource_checks": [
            "scratch-files",
            "scratch-sockets",
            "scratch-locks",
            "processes",
            "file-descriptors",
            "rss",
            "child-cpu-time",
            "terminal-state",
        ],
    }
    failures = 0
    try:
        for name in names:
            report_dir = report_path.parent / name
            report_dir.mkdir(parents=True, exist_ok=True)
            for run_number in range(1, runs + 1):
                print(f"{name} soak run {run_number}/{runs}", flush=True)
                result = run_once(
                    name,
                    commands[name],
                    run_number,
                    scratch,
                    report_dir,
                    environment,
                    memory_tolerance,
                    timeout_seconds,
                )
                report["scenarios"][name]["runs"].append(result)
                if result["state"] != "passed":
                    failures += 1
    finally:
        final_state = "passed" if failures == 0 else "failed"
        report["ended_at"] = now()
        report["state"] = final_state
        report["result_state"] = final_state
        report["reason"] = (
            "all bounded workflow soak runs completed without resource drift"
            if final_state == "passed"
            else "one or more workflow soak runs reported a resource leak, drift, timeout, or command failure"
        )
        report["scratch"] = str(scratch)
        if failures == 0:
            shutil.rmtree(scratch, ignore_errors=True)
            report["scratch"] = "removed"
        report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    if failures:
        print(f"workflow soak failed; report: {report_path}", file=sys.stderr)
        return 1
    print(f"workflow soak passed: {len(names)} scenarios x {runs} runs; report: {report_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
