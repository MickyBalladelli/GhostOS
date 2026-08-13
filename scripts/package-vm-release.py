#!/usr/bin/env python3
"""Package VM release files with provenance and compatibility metadata.

The archive is deliberately boring: files are copied unchanged and a signed
release process signs the release inputs before this command runs.
The manifest itself records every file digest, firmware mode, default device
topology, executed evidence, and known host limitations.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from gzip import GzipFile
from typing import Iterable


ROOT = pathlib.Path(__file__).resolve().parent.parent
VM_MANIFEST = ROOT / "virtual_machine/Cargo.toml"
CHANGELOG = ROOT / "CHANGELOG.md"

DEFAULT_TOPOLOGY = [
    {"name": "legacy-pic", "transport": "io", "address": "0x20,0xa0", "interrupt": "isa-pic"},
    {"name": "local-apic", "transport": "mmio/msr", "address": "0xfee00000/0x1b", "interrupt": "n/a"},
    {"name": "pit", "transport": "io", "address": "0x40", "interrupt": "0x20"},
    {"name": "hpet", "transport": "mmio", "address": "0xfed00000", "interrupt": "0x20+timer"},
    {"name": "ps2", "transport": "io", "address": "0x60-0x64", "interrupt": "0x21/0x2c"},
    {"name": "serial-16550", "transport": "io", "address": "0x3f8-0x3ff", "interrupt": "0x24"},
    {"name": "pci-host", "transport": "io/mmio", "address": "0xcf8/0xe0000000", "interrupt": "n/a"},
    {"name": "vga-vesa", "transport": "io/mmio", "address": "0x3c0/0xb8000/0xf0000000", "interrupt": "n/a"},
    {"name": "power-control", "transport": "io", "address": "0x604", "interrupt": "n/a"},
    {"name": "ahci", "transport": "pci-mmio", "address": "00:04.0/bar5=0xf1000000", "interrupt": "0x2b"},
    {"name": "nvme", "transport": "pci-mmio", "address": "00:05.0/bar0=0xf1100000", "interrupt": "0x31"},
    {"name": "e1000", "transport": "pci-mmio", "address": "00:06.0/bar0=0xf1200000", "interrupt": "0x2d"},
    {"name": "virtio-net", "transport": "pci-io", "address": "00:07.0/bar0=0x5000", "interrupt": "0x2e"},
    {"name": "virtio-blk", "transport": "pci-io", "address": "00:08.0/bar0=0x5100", "interrupt": "0x32"},
    {"name": "virtio-console", "transport": "pci-io", "address": "00:09.0/bar0=0x5200", "interrupt": "0x33"},
    {"name": "virtio-rng", "transport": "pci-io", "address": "00:0a.0/bar0=0x5300", "interrupt": "0x34"},
    {"name": "memory-hotplug", "transport": "mmio", "address": "0xfebe0000", "interrupt": "0x36"},
    {"name": "guest-agent", "transport": "mmio", "address": "0xfebf0000", "interrupt": "0x35"},
    {"name": "pv-clock", "transport": "msr", "address": "0x4b564d00/0x4b564d01", "interrupt": "n/a"},
]

KNOWN_HOST_LIMITATIONS = [
    "The portable CPU executor remains the correctness path; native acceleration probes host handles but does not replace guest execution.",
    "KVM requires Linux and /dev/kvm; HVF requires macOS; WHPX requires Windows; HAXM requires its host device and supported platform.",
    "Migration authentication does not encrypt TCP; use mutually authenticated TLS, an authorized VPN, or an SSH tunnel.",
    "Raw terminal mode changes host settings only for an interactive TTY and is unavailable on non-TTY streams; terminal state is host-owned.",
    "Network backends default to an in-process loopback hub; external network connectivity is not part of the VM artifact.",
    "QEMU, hardware, fuzz, coverage, mutation, cluster, and soak evidence may be skipped when their host prerequisites are absent.",
]


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_value(*args: str, default: str = "unknown") -> str:
    result = subprocess.run(
        ["git", "-C", str(ROOT), *args],
        check=False,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else default


def source_date_epoch() -> int:
    value = os.environ.get("SOURCE_DATE_EPOCH")
    if value is not None:
        try:
            return max(0, int(value))
        except ValueError as error:
            raise ValueError("SOURCE_DATE_EPOCH must be an integer") from error
    value = git_value("show", "-s", "--format=%ct")
    try:
        return max(0, int(value))
    except ValueError:
        return 0


def vm_version() -> str:
    with VM_MANIFEST.open("rb") as stream:
        package = tomllib.load(stream).get("package", {})
    return str(package.get("version", "unknown"))


def parse_artifact(value: str) -> tuple[str, pathlib.Path]:
    if "=" in value:
        name, raw_path = value.split("=", 1)
        if not name or not raw_path:
            raise ValueError(f"invalid artifact `{value}`; use NAME=PATH or PATH")
    else:
        raw_path = value
        name = pathlib.Path(value).name
    path = pathlib.Path(raw_path).expanduser().resolve()
    if not path.is_file():
        raise ValueError(f"artifact does not exist or is not a regular file: {path}")
    return name, path


def evidence_records(root: pathlib.Path) -> tuple[list[dict[str, object]], list[pathlib.Path]]:
    root = root.expanduser().resolve()
    if not root.is_dir():
        raise ValueError(f"evidence directory does not exist: {root}")
    files = sorted(path for path in root.rglob("*") if path.is_file() and not path.is_symlink())
    if not files:
        raise ValueError(f"evidence directory is empty: {root}")

    records: list[dict[str, object]] = []
    for path in files:
        if path.suffix != ".json":
            continue
        try:
            value = json.loads(path.read_text())
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            continue
        state = value.get("result_state", value.get("state"))
        if state is None:
            continue
        if state == "pass":
            state = "passed"
        if state == "fail":
            state = "failed"
        if state not in {"passed", "failed", "skipped"}:
            continue
        records.append(
            {
                "path": path.relative_to(root).as_posix(),
                "sha256": sha256(path),
                "state": state,
                "tier": value.get("tier"),
                "test_id": value.get("test_id"),
                "command": value.get("command"),
                "firmware": value.get("firmware"),
                "host": value.get("host"),
                "reason": value.get("reason") or value.get("prerequisite"),
            }
        )
    if not records:
        raise ValueError(f"evidence directory contains no result/evidence records: {root}")
    return records, files


def artifact_entry(name: str, path: pathlib.Path) -> dict[str, object]:
    return {
        "name": name,
        "source": path.name,
        "size": path.stat().st_size,
        "sha256": sha256(path),
    }


def add_file(archive: tarfile.TarFile, path: pathlib.Path, name: str, timestamp: int) -> None:
    info = archive.gettarinfo(str(path), arcname=name)
    info.mtime = timestamp
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    with path.open("rb") as stream:
        archive.addfile(info, stream)


def write_archive(
    output: pathlib.Path,
    manifest: dict[str, object],
    artifacts: list[tuple[str, pathlib.Path]],
    changelog: pathlib.Path,
    release_report: pathlib.Path,
    release_claims: pathlib.Path,
    upgrade_compatibility: pathlib.Path,
    evidence_root: pathlib.Path,
    evidence_files: Iterable[pathlib.Path],
    attestation_files: Iterable[pathlib.Path],
    timestamp: int,
) -> None:
    output = output.expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{output.name}.", suffix=".tmp", dir=output.parent
    )
    os.close(descriptor)
    temporary = pathlib.Path(temporary_name)
    try:
        with temporary.open("wb") as raw:
            with GzipFile(fileobj=raw, mode="wb", filename="", mtime=timestamp) as compressed:
                with tarfile.open(fileobj=compressed, mode="w") as archive:
                    manifest_bytes = json.dumps(manifest, indent=2, sort_keys=True).encode() + b"\n"
                    info = tarfile.TarInfo("release-manifest.json")
                    info.size = len(manifest_bytes)
                    info.mtime = timestamp
                    info.uid = 0
                    info.gid = 0
                    info.uname = ""
                    info.gname = ""
                    archive.addfile(info, io.BytesIO(manifest_bytes))
                    add_file(archive, changelog, "CHANGELOG.md", timestamp)
                    add_file(archive, release_report, "release-report.json", timestamp)
                    add_file(archive, release_claims, "release-claims.json", timestamp)
                    add_file(archive, upgrade_compatibility, "upgrade-compatibility.json", timestamp)
                    for name, path in artifacts:
                        add_file(archive, path, f"artifacts/{name}", timestamp)
                    for path in evidence_files:
                        relative = path.relative_to(evidence_root).as_posix()
                        add_file(archive, path, f"evidence/{relative}", timestamp)
                    for path in attestation_files:
                        add_file(archive, path, f"attestations/{path.name}", timestamp)
        os.replace(temporary, output)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=pathlib.Path, help="output .tar.gz archive")
    parser.add_argument("--artifact", action="append", required=True, help="artifact PATH or NAME=PATH")
    parser.add_argument("--evidence-dir", required=True, type=pathlib.Path)
    parser.add_argument(
        "--release-report",
        required=True,
        type=pathlib.Path,
        help="verified report produced by scripts/release-report.py",
    )
    parser.add_argument(
        "--release-claims",
        required=True,
        type=pathlib.Path,
        help="evidence-backed claims manifest produced for this release",
    )
    parser.add_argument(
        "--upgrade-compatibility",
        required=True,
        type=pathlib.Path,
        help="compatibility proof for upgrade and rollback against the previous release",
    )
    parser.add_argument(
        "--attestation-dir",
        required=True,
        type=pathlib.Path,
        help="directory produced by scripts/release-attestations.py",
    )
    parser.add_argument("--firmware", action="append", required=True, choices=("bios", "uefi"))
    parser.add_argument("--host-limitation", action="append", default=[], help="append a known host limitation")
    args = parser.parse_args()

    try:
        artifacts = [parse_artifact(value) for value in args.artifact]
        names = [name for name, _ in artifacts]
        if len(names) != len(set(names)):
            raise ValueError("artifact names must be unique")
        output_path = args.output.expanduser().resolve()
        if output_path in {path for _, path in artifacts}:
            raise ValueError("output archive must not replace an input artifact")
        evidence_root = args.evidence_dir.expanduser().resolve()
        records, evidence_files = evidence_records(evidence_root)
        release_report = args.release_report.expanduser().resolve()
        if not release_report.is_file():
            raise ValueError(f"release report does not exist: {release_report}")
        subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/release-report.py"),
                "--check",
                "--evidence-dir",
                str(evidence_root),
                "--report",
                str(release_report),
            ],
            check=True,
            cwd=ROOT,
        )
        release_claims = args.release_claims.expanduser().resolve()
        if not release_claims.is_file():
            raise ValueError(f"release claims do not exist: {release_claims}")
        subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/validate-release-claims.py"),
                "--claims",
                str(release_claims),
                "--evidence-dir",
                str(evidence_root),
                "--release-notes",
                str(CHANGELOG),
            ],
            check=True,
            cwd=ROOT,
        )
        upgrade_compatibility = args.upgrade_compatibility.expanduser().resolve()
        if not upgrade_compatibility.is_file():
            raise ValueError(f"upgrade compatibility manifest does not exist: {upgrade_compatibility}")
        subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/validate-upgrade-compatibility.py"),
                "--manifest",
                str(upgrade_compatibility),
                "--evidence-dir",
                str(evidence_root),
                "--release-revision",
                git_value("rev-parse", "HEAD"),
                "--release-version",
                vm_version(),
            ],
            check=True,
            cwd=ROOT,
        )
        attestation_root = args.attestation_dir.expanduser().resolve()
        if not attestation_root.is_dir():
            raise ValueError(f"attestation directory does not exist: {attestation_root}")
        attestation_files = sorted(
            path for path in attestation_root.iterdir() if path.is_file() and not path.is_symlink()
        )
        if not attestation_files:
            raise ValueError(f"attestation directory is empty: {attestation_root}")
        attestation_command = [
            sys.executable,
            str(ROOT / "scripts/release-attestations.py"),
            "--check",
            "--attestation-dir",
            str(attestation_root),
        ]
        for name, path in artifacts:
            attestation_command.extend(["--artifact", f"{name}={path}"])
        subprocess.run(
            attestation_command,
            check=True,
            cwd=ROOT,
        )
        subprocess.run(
            [sys.executable, str(ROOT / "scripts/validate-changelog.py")],
            check=True,
            cwd=ROOT,
        )
        if not CHANGELOG.is_file():
            raise ValueError(f"changelog does not exist: {CHANGELOG}")
        timestamp = source_date_epoch()
        manifest = {
            "schema": 1,
            "product": "synos-vm",
            "version": vm_version(),
            "revision": git_value("rev-parse", "HEAD"),
            "source_date_epoch": timestamp,
            "firmware_modes": sorted(set(args.firmware)),
            "artifacts": [artifact_entry(name, path) for name, path in artifacts],
            "changelog": {"path": "CHANGELOG.md", "sha256": sha256(CHANGELOG)},
            "release_report": {"path": "release-report.json", "sha256": sha256(release_report)},
            "release_claims": {"path": "release-claims.json", "sha256": sha256(release_claims)},
            "upgrade_compatibility": {
                "path": "upgrade-compatibility.json",
                "sha256": sha256(upgrade_compatibility),
            },
            "device_topology": DEFAULT_TOPOLOGY,
            "test_evidence": records,
            "attestations": {
                "directory": "attestations",
                "files": [
                    {
                        "path": path.name,
                        "size": path.stat().st_size,
                        "sha256": sha256(path),
                    }
                    for path in attestation_files
                ],
            },
            "known_host_limitations": sorted(set(KNOWN_HOST_LIMITATIONS + args.host_limitation)),
        }
        if any(record["state"] == "failed" for record in records):
            raise ValueError("release evidence contains failed results")
        if any(record["state"] == "skipped" and not record["reason"] for record in records):
            raise ValueError("skipped release evidence must include a reason")
        write_archive(
            args.output,
            manifest,
            artifacts,
            CHANGELOG,
            release_report,
            release_claims,
            upgrade_compatibility,
            evidence_root,
            evidence_files,
            attestation_files,
            timestamp,
        )
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"cannot package VM release: {error}", file=sys.stderr)
        return 1

    print(args.output.expanduser().resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
