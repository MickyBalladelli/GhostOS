#!/usr/bin/env python3
"""Validate the release manifest, checksums, and archive contents."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import tarfile


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_archive(path: pathlib.Path, tool_name: str) -> None:
    with tarfile.open(path, "r:gz") as archive:
        names = {member.name for member in archive.getmembers()}
    required = {"manifest.json", "SHA256SUMS", "MEDIA-SHA256SUMS"}
    required.update({"payload/ghostos-bios.img", "payload/ghostos-uefi.img", f"tools/{tool_name}"})
    if not required.issubset(names):
        missing = ", ".join(sorted(required - names))
        raise ValueError(f"{path.name} is missing archive members: {missing}")
    for name in names:
        if name.startswith("/") or ".." in pathlib.PurePosixPath(name).parts:
            raise ValueError(f"{path.name} contains unsafe member: {name}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=pathlib.Path, required=True)
    args = parser.parse_args()
    directory = args.directory.resolve()
    manifest_path = directory / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("schema") != 1 or manifest.get("product") != "GhostOS":
        raise ValueError("unsupported installer/recovery manifest")

    entries = manifest.get("artifacts", []) + manifest.get("tools", []) + manifest.get("support", [])
    if not entries:
        raise ValueError("manifest has no artifacts")
    for entry in entries:
        path = (directory / entry["name"]).resolve()
        if path.parent != directory or not path.is_file():
            raise ValueError(f"manifest artifact is missing: {entry['name']}")
        if path.stat().st_size != entry["size"] or sha256(path) != entry["sha256"]:
            raise ValueError(f"manifest digest mismatch: {entry['name']}")

    for line in (directory / "SHA256SUMS").read_text().splitlines():
        digest, name = line.split(maxsplit=1)
        path = directory / name.strip()
        if not path.is_file() or sha256(path) != digest:
            raise ValueError(f"checksum mismatch: {name}")

    validate_archive(directory / "ghostos-installer.tar.gz", "install-ghostos.sh")
    validate_archive(directory / "ghostos-recovery.tar.gz", "recover-ghostos.sh")
    print(f"validated installer and recovery artifacts: {directory}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
