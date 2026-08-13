#!/usr/bin/env python3
"""Write deterministic installer and recovery release manifests and archives."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import pathlib
import tarfile


SCHEMA = 1


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(path: pathlib.Path, root: pathlib.Path, kind: str) -> dict[str, object]:
    return {
        "name": path.name,
        "kind": kind,
        "size": path.stat().st_size,
        "sha256": sha256(path),
        "path": path.relative_to(root).as_posix(),
    }


def write_json(path: pathlib.Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def write_checksums(paths: list[pathlib.Path], output: pathlib.Path) -> None:
    lines = [f"{sha256(path)}  {path.name}" for path in sorted(paths, key=lambda item: item.name)]
    output.write_text("\n".join(lines) + "\n")


def add_file(archive: tarfile.TarFile, path: pathlib.Path, name: str, timestamp: int) -> None:
    info = tarfile.TarInfo(name)
    data = path.read_bytes()
    info.size = len(data)
    info.mtime = timestamp
    info.mode = 0o755 if path.name.endswith(".sh") else 0o644
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    archive.addfile(info, io.BytesIO(data))


def write_archive(output: pathlib.Path, files: list[tuple[pathlib.Path, str]], timestamp: int) -> None:
    with output.open("wb") as raw:
        with gzip.GzipFile(fileobj=raw, mode="wb", mtime=timestamp) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                for path, name in sorted(files, key=lambda item: item[1]):
                    add_file(archive, path, name, timestamp)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--source-date-epoch", type=int, required=True)
    args = parser.parse_args()

    root = args.root.resolve()
    output = args.output.resolve()
    payloads = [
        (output / "synos-bios.img", "bios-disk-image"),
        (output / "synos-uefi.img", "uefi-disk-image"),
        (output / "synos-installer.img", "installer-media"),
        (output / "synos-recovery.img", "recovery-media"),
        (output / "synos-loader.efi", "uefi-loader"),
    ]
    tools = [output / "install-synos.sh", output / "recover-synos.sh"]
    support = [output / "MEDIA-SHA256SUMS"]
    for path, _ in payloads + [(tool, "tool") for tool in tools] + [(path, "support") for path in support]:
        if not path.is_file():
            raise SystemExit(f"missing release artifact: {path}")

    manifest = {
        "schema": SCHEMA,
        "product": "SynOS",
        "revision": args.revision,
        "source_date_epoch": args.source_date_epoch,
        "installer": {
            "media": "synos-installer.img",
            "archive": "synos-installer.tar.gz",
            "boot_modes": ["bios", "uefi"],
            "payloads": [artifact(path, root, kind) for path, kind in payloads[:3]],
            "tool": "install-synos.sh",
        },
        "recovery": {
            "media": "synos-recovery.img",
            "archive": "synos-recovery.tar.gz",
            "boot_modes": ["bios", "uefi"],
            "payloads": [artifact(path, root, kind) for path, kind in payloads[1:4]],
            "tool": "recover-synos.sh",
            "guarantees": [
                "verify payload checksums before writing",
                "restore a bootable BIOS or UEFI image",
                "refuse an unconfirmed block-device write",
            ],
        },
        "artifacts": [artifact(path, root, kind) for path, kind in payloads],
        "tools": [artifact(path, root, "operator-tool") for path in tools],
        "support": [artifact(path, root, "integrity-metadata") for path in support],
    }
    manifest_path = output / "manifest.json"
    write_json(manifest_path, manifest)

    checksum_paths = [path for path, _ in payloads] + tools + support + [manifest_path]
    checksums_path = output / "SHA256SUMS"
    write_checksums(checksum_paths, checksums_path)

    common = [
        (manifest_path, "manifest.json"),
        (checksums_path, "SHA256SUMS"),
        (output / "MEDIA-SHA256SUMS", "MEDIA-SHA256SUMS"),
        (output / "synos-loader.efi", "payload/synos-loader.efi"),
        (output / "synos-bios.img", "payload/synos-bios.img"),
        (output / "synos-uefi.img", "payload/synos-uefi.img"),
        (output / "synos-installer.img", "media/synos-installer.img"),
        (output / "synos-recovery.img", "media/synos-recovery.img"),
    ]
    write_archive(output / "synos-installer.tar.gz", common + [
        (output / "install-synos.sh", "tools/install-synos.sh"),
        (root / "docs/installer-media.md", "docs/README.md"),
    ], args.source_date_epoch)
    write_archive(output / "synos-recovery.tar.gz", common + [
        (output / "recover-synos.sh", "tools/recover-synos.sh"),
        (root / "docs/recovery-media.md", "docs/README.md"),
    ], args.source_date_epoch)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
