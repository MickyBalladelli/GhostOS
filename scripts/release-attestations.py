#!/usr/bin/env python3
"""Create and verify signed SynOS release attestations.

Each artifact receives one signed in-toto-style statement. The statement binds
the artifact digest to the Cargo SBOM, dependency lockfile provenance,
compiler identity, source and configuration digests, and a byte-for-byte
reproducibility comparison against an independently produced artifact.

The signing key is supplied by the release operator. Only the public key and
detached signatures are written to the attestation directory.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
ATTESTATION_SCHEMA = 1
PREDICATE_TYPE = "https://synos.dev/attestations/release/v1"
INDEX_NAME = "attestations-index.json"
INDEX_SIGNATURE_NAME = "attestations-index.json.sig"
PUBLIC_KEY_NAME = "attestation-public-key.pem"
LOCKFILE = ROOT / "Cargo.lock"

DEFAULT_CONFIGURATION_FILES = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".cargo/config.toml",
    "Dockerfile",
    "scripts/build-bios-image.sh",
    "scripts/build-portable-image.sh",
    "scripts/check-reproducible-image.sh",
    "scripts/package-vm-release.py",
    "scripts/release-report.py",
    "scripts/release-gate.sh",
    "scripts/release-slo-gate.py",
    "scripts/release-attestations.py",
)


def canonical_json(value: object) -> bytes:
    return json.dumps(value, indent=2, sort_keys=True).encode() + b"\n"


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_path(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def hash_files(paths: list[pathlib.Path]) -> tuple[str, int]:
    digest = hashlib.sha256()
    for path in sorted(paths):
        relative = path.relative_to(ROOT).as_posix().encode()
        digest.update(relative)
        digest.update(b"\0")
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        digest.update(b"\0")
    return digest.hexdigest(), len(paths)


def git(*args: str, check: bool = True) -> str:
    result = subprocess.run(
        ["git", "-C", str(ROOT), *args],
        check=False,
        capture_output=True,
        text=True,
    )
    if check and result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or f"git {' '.join(args)} failed")
    return result.stdout.strip()


def parse_artifact(value: str) -> tuple[str, pathlib.Path]:
    if "=" in value:
        name, raw_path = value.split("=", 1)
        if not name or not raw_path:
            raise ValueError(f"invalid artifact `{value}`; use NAME=PATH or PATH")
    else:
        raw_path = value
        name = pathlib.Path(value).name
    if pathlib.PurePosixPath(name).name != name or name in {"", ".", ".."}:
        raise ValueError(f"artifact name must be a single path component: {name!r}")
    path = pathlib.Path(raw_path).expanduser().resolve()
    if not path.is_file():
        raise ValueError(f"artifact does not exist or is not a regular file: {path}")
    return name, path


def parse_artifacts(values: list[str], option: str) -> dict[str, pathlib.Path]:
    artifacts = dict(parse_artifact(value) for value in values)
    if len(artifacts) != len(values):
        raise ValueError(f"{option} names must be unique")
    if not artifacts:
        raise ValueError(f"at least one {option} is required")
    return artifacts


def source_record() -> dict[str, object]:
    tracked = [ROOT / relative for relative in git("ls-files", "-z").split("\0") if relative]
    missing = [path.relative_to(ROOT).as_posix() for path in tracked if not path.is_file()]
    if missing:
        raise ValueError(f"tracked source files are missing: {', '.join(missing)}")
    digest, count = hash_files(tracked)
    return {
        "algorithm": "sha256",
        "digest": digest,
        "git_commit": git("rev-parse", "HEAD"),
        "git_tree": git("rev-parse", "HEAD^{tree}"),
        "tracked_file_count": count,
        "clean": not bool(git("status", "--porcelain", check=True)),
    }


def configuration_record(paths: list[pathlib.Path]) -> dict[str, object]:
    digest, count = hash_files(paths)
    return {
        "algorithm": "sha256",
        "digest": digest,
        "files": [path.relative_to(ROOT).as_posix() for path in sorted(paths)],
        "file_count": count,
    }


def command_output(command: list[str], required: bool = True) -> str | None:
    executable = shutil.which(command[0])
    if executable is None:
        if required:
            raise ValueError(f"required compiler identity command is missing: {command[0]}")
        return None
    result = subprocess.run(command, check=False, capture_output=True, text=True)
    if result.returncode != 0:
        if required:
            raise ValueError(f"compiler identity command failed: {' '.join(command)}")
        return None
    return (result.stdout or result.stderr).strip()


def compiler_record(target: str | None) -> dict[str, object]:
    record: dict[str, object] = {
        "rustc_version_verbose": command_output(["rustc", "--version", "--verbose"]),
        "cargo_version_verbose": command_output(["cargo", "--version", "--verbose"]),
        "rustup_toolchain": command_output(["rustup", "show", "active-toolchain"], required=False),
        "clang_version": command_output(["clang", "--version"], required=False),
        "target": target or "unspecified",
        "profile": "release",
        "rustflags": os.environ.get("RUSTFLAGS", ""),
        "source_date_epoch": os.environ.get("SOURCE_DATE_EPOCH", ""),
    }
    return record


def cargo_components() -> tuple[dict[str, object], list[dict[str, object]]]:
    with LOCKFILE.open("rb") as stream:
        lockfile = tomllib.load(stream)
    components: list[dict[str, object]] = []
    provenance: list[dict[str, object]] = []
    for package in lockfile.get("package", []):
        name = str(package["name"])
        version = str(package["version"])
        source = package.get("source")
        purl = f"pkg:cargo/{name}@{version}"
        component: dict[str, object] = {
            "type": "library",
            "bom-ref": purl,
            "name": name,
            "version": version,
            "purl": purl,
        }
        if package.get("checksum"):
            component["hashes"] = [{"alg": "SHA-256", "content": package["checksum"]}]
        if source:
            component["properties"] = [{"name": "synos:cargo-source", "value": source}]
        components.append(component)
        provenance.append(
            {
                "name": name,
                "version": version,
                "source": source,
                "checksum": package.get("checksum"),
            }
        )
    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": f"urn:uuid:{sha256_path(LOCKFILE)}",
        "version": 1,
        "metadata": {"component": {"type": "application", "name": "SynOS"}},
        "components": components,
    }
    return sbom, provenance


def openssl(*args: str, input_path: pathlib.Path | None = None, output_path: pathlib.Path | None = None) -> None:
    command = ["openssl", *args]
    if input_path is not None:
        command.extend(["-in", str(input_path)])
    if output_path is not None:
        command.extend(["-out", str(output_path)])
    try:
        subprocess.run(command, check=True, capture_output=True, text=True)
    except FileNotFoundError as error:
        raise ValueError("openssl is required to create or verify release signatures") from error
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() or "openssl command failed"
        raise ValueError(detail) from error


def sign(key: pathlib.Path, payload: bytes, output: pathlib.Path) -> None:
    temporary = output.with_suffix(output.suffix + ".payload")
    temporary.write_bytes(payload)
    try:
        openssl("pkeyutl", "-sign", "-inkey", str(key), "-in", str(temporary), "-out", str(output))
    finally:
        temporary.unlink(missing_ok=True)


def verify(public_key: pathlib.Path, payload: bytes, signature: pathlib.Path) -> None:
    temporary = signature.with_suffix(signature.suffix + ".payload")
    temporary.write_bytes(payload)
    try:
        openssl(
            "pkeyutl",
            "-verify",
            "-pubin",
            "-inkey",
            str(public_key),
            "-sigfile",
            str(signature),
            "-in",
            str(temporary),
        )
    finally:
        temporary.unlink(missing_ok=True)


def public_key_id(path: pathlib.Path) -> str:
    return sha256_path(path)


def statement_for(
    name: str,
    artifact: pathlib.Path,
    reproducible: pathlib.Path,
    source: dict[str, object],
    configuration: dict[str, object],
    compiler: dict[str, object],
    sbom: dict[str, object],
    provenance: list[dict[str, object]],
) -> dict[str, object]:
    artifact_digest = sha256_path(artifact)
    reproducible_digest = sha256_path(reproducible)
    if artifact_digest != reproducible_digest:
        raise ValueError(
            f"reproducibility mismatch for {name}: {artifact_digest} != {reproducible_digest}"
        )
    return {
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [{"name": name, "digest": {"sha256": artifact_digest}}],
        "predicateType": PREDICATE_TYPE,
        "predicate": {
            "schema": ATTESTATION_SCHEMA,
            "artifact": {"path": artifact.name, "size": artifact.stat().st_size},
            "sbom": sbom,
            "dependency_provenance": {
                "lockfile": "Cargo.lock",
                "lockfile_sha256": sha256_path(LOCKFILE),
                "packages": provenance,
            },
            "compiler_identity": compiler,
            "source": source,
            "configuration": configuration,
            "reproducible_build": {
                "verified": True,
                "comparison": "byte-for-byte",
                "independent_artifact": reproducible.name,
                "independent_artifact_sha256": reproducible_digest,
            },
        },
    }


def write_attestations(
    output_dir: pathlib.Path,
    artifacts: dict[str, pathlib.Path],
    reproducible: dict[str, pathlib.Path],
    signing_key: pathlib.Path,
    configuration_files: list[pathlib.Path],
    target: str | None,
) -> None:
    if not signing_key.is_file():
        raise ValueError(f"signing key does not exist: {signing_key}")
    if set(artifacts) != set(reproducible):
        raise ValueError("artifact and reproducible-artifact names must match exactly")
    output_dir.mkdir(parents=True, exist_ok=True)
    source = source_record()
    if not source["clean"]:
        raise ValueError("release attestations require a clean source tree")
    configuration = configuration_record(configuration_files)
    compiler = compiler_record(target)
    sbom, provenance = cargo_components()
    public_key = output_dir / PUBLIC_KEY_NAME
    openssl("pkey", "-pubout", "-in", str(signing_key), output_path=public_key)
    key_id = public_key_id(public_key)
    entries: list[dict[str, object]] = []
    for name in sorted(artifacts):
        statement = statement_for(
            name,
            artifacts[name],
            reproducible[name],
            source,
            configuration,
            compiler,
            sbom,
            provenance,
        )
        statement_path = output_dir / f"{name}.intoto.json"
        signature_path = output_dir / f"{name}.intoto.json.sig"
        statement_bytes = canonical_json(statement)
        statement_path.write_bytes(statement_bytes)
        sign(signing_key, statement_bytes, signature_path)
        entries.append(
            {
                "name": name,
                "statement": statement_path.name,
                "statement_sha256": sha256_bytes(statement_bytes),
                "signature": signature_path.name,
                "artifact_sha256": sha256_path(artifacts[name]),
            }
        )
    index = {
        "schema": ATTESTATION_SCHEMA,
        "kind": "synos-release-attestations",
        "predicate_type": PREDICATE_TYPE,
        "source_commit": source["git_commit"],
        "source_digest": source["digest"],
        "public_key": public_key.name,
        "public_key_sha256": key_id,
        "attestations": entries,
    }
    index_bytes = canonical_json(index)
    index_path = output_dir / INDEX_NAME
    index_path.write_bytes(index_bytes)
    sign(signing_key, index_bytes, output_dir / INDEX_SIGNATURE_NAME)
    print(f"wrote {len(entries)} signed release attestations to {output_dir}")


def check_attestations(
    attestation_dir: pathlib.Path,
    artifacts: dict[str, pathlib.Path],
) -> None:
    index_path = attestation_dir / INDEX_NAME
    index_signature = attestation_dir / INDEX_SIGNATURE_NAME
    public_key = attestation_dir / PUBLIC_KEY_NAME
    if not index_path.is_file() or not index_signature.is_file() or not public_key.is_file():
        raise ValueError("attestation index, signature, or public key is missing")
    index_bytes = index_path.read_bytes()
    verify(public_key, index_bytes, index_signature)
    index = json.loads(index_bytes)
    if (
        not isinstance(index, dict)
        or index.get("schema") != ATTESTATION_SCHEMA
        or index.get("kind") != "synos-release-attestations"
    ):
        raise ValueError("unsupported attestation index schema")
    if index.get("public_key_sha256") != public_key_id(public_key):
        raise ValueError("attestation public key digest does not match the index")
    raw_entries = index.get("attestations")
    if not isinstance(raw_entries, list) or any(not isinstance(entry, dict) for entry in raw_entries):
        raise ValueError("attestation index entries are malformed")
    entries = {entry.get("name"): entry for entry in raw_entries}
    if len(entries) != len(raw_entries):
        raise ValueError("attestation index contains duplicate artifact names")
    if set(entries) != set(artifacts):
        raise ValueError("attestation coverage does not match release artifacts")
    expected_files = {INDEX_NAME, INDEX_SIGNATURE_NAME, PUBLIC_KEY_NAME}
    expected_files.update(
        filename
        for entry in raw_entries
        for filename in (entry.get("statement"), entry.get("signature"))
        if isinstance(filename, str)
    )
    actual_files = {path.name for path in attestation_dir.iterdir() if path.is_file()}
    if actual_files != expected_files:
        raise ValueError(
            f"attestation directory contains unexpected files: "
            f"missing={sorted(expected_files - actual_files)}, "
            f"extra={sorted(actual_files - expected_files)}"
        )
    for name, artifact in artifacts.items():
        entry = entries[name]
        statement_path = attestation_dir / str(entry["statement"])
        signature_path = attestation_dir / str(entry["signature"])
        statement_bytes = statement_path.read_bytes()
        verify(public_key, statement_bytes, signature_path)
        if entry.get("statement_sha256") != sha256_bytes(statement_bytes):
            raise ValueError(f"statement digest does not match the index: {name}")
        if entry.get("artifact_sha256") != sha256_path(artifact):
            raise ValueError(f"artifact digest does not match its attestation: {name}")
        statement = json.loads(statement_bytes)
        if not isinstance(statement, dict) or statement.get("predicateType") != PREDICATE_TYPE:
            raise ValueError(f"statement predicate type is unsupported: {name}")
        subject = statement.get("subject", [{}])[0]
        if subject.get("name") != name or subject.get("digest", {}).get("sha256") != sha256_path(artifact):
            raise ValueError(f"statement subject does not match artifact: {name}")
        predicate = statement.get("predicate", {})
        required_records = {
            "sbom",
            "dependency_provenance",
            "compiler_identity",
            "source",
            "configuration",
            "reproducible_build",
        }
        if not required_records.issubset(predicate):
            raise ValueError(f"statement is missing required release records: {name}")
        reproducible = predicate["reproducible_build"]
        if reproducible.get("verified") is not True:
            raise ValueError(f"reproducibility was not verified: {name}")
    print(f"verified {len(artifacts)} signed release attestations")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    parser.add_argument("--output-dir", type=pathlib.Path)
    parser.add_argument("--attestation-dir", type=pathlib.Path)
    parser.add_argument("--artifact", action="append", default=[])
    parser.add_argument("--reproducible-artifact", action="append", default=[])
    parser.add_argument("--signing-key", type=pathlib.Path)
    parser.add_argument("--config-file", action="append", default=[])
    parser.add_argument("--target")
    args = parser.parse_args()
    try:
        artifacts = parse_artifacts(args.artifact, "artifact")
        if args.write:
            if args.output_dir is None or args.signing_key is None:
                raise ValueError("--write requires --output-dir and --signing-key")
            reproducible = parse_artifacts(args.reproducible_artifact, "reproducible-artifact")
            configuration_files = [
                ROOT / value for value in (args.config_file or list(DEFAULT_CONFIGURATION_FILES))
            ]
            missing = [path.relative_to(ROOT).as_posix() for path in configuration_files if not path.is_file()]
            if missing:
                raise ValueError(f"configuration files are missing: {', '.join(missing)}")
            write_attestations(
                args.output_dir.expanduser().resolve(),
                artifacts,
                reproducible,
                args.signing_key.expanduser().resolve(),
                configuration_files,
                args.target,
            )
        else:
            if args.attestation_dir is None:
                raise ValueError("--check requires --attestation-dir")
            if args.reproducible_artifact:
                raise ValueError("--reproducible-artifact is only valid with --write")
            check_attestations(args.attestation_dir.expanduser().resolve(), artifacts)
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as error:
        print(f"release attestation error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
