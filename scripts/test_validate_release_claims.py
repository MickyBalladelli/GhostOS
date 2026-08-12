#!/usr/bin/env python3
"""Direct regression coverage for validate-release-claims.py."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import pathlib
import sys
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("validate-release-claims.py")
SPEC = importlib.util.spec_from_file_location("validate_release_claims", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class ReleaseClaimsRegressionTests(unittest.TestCase):
    def manifest(self, evidence: pathlib.Path, notes: pathlib.Path) -> pathlib.Path:
        artifact = evidence / "benchmark.json"
        artifact.write_text(json.dumps({"revision": MODULE.git_revision(), "p99_ns": 983}) + "\n")
        claims = {
            "schema": 1,
            "kind": "synos-release-claims",
            "revision": MODULE.git_revision(),
            "claims": [
                {
                    "id": "scale",
                    "term": "scalable",
                    "statement": "The scheduler is scalable for the named workload.",
                    "workload": "scheduler dispatch across five CPU tiers",
                    "measured_threshold": {
                        "metric": "p99 latency",
                        "operator": "<=",
                        "value": 100000,
                        "observed": 983,
                        "unit": "ns",
                    },
                    "host_configuration": {
                        "system": "Darwin",
                        "release": "25.5.0",
                        "architecture": "arm64",
                        "configuration": "release profile; 20,000 iterations",
                    },
                    "artifact": {
                        "path": "benchmark.json",
                        "sha256": hashlib.sha256(artifact.read_bytes()).hexdigest(),
                        "description": "retained benchmark output",
                    },
                }
            ],
        }
        path = evidence / "release-claims.json"
        path.write_text(json.dumps(claims) + "\n")
        notes.write_text("# Changelog\n\n## [Unreleased]\n\n- scalable scheduler\n")
        return path

    def test_valid_claim_requires_all_evidence_fields(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            evidence = pathlib.Path(directory)
            notes = evidence / "CHANGELOG.md"
            claims = self.manifest(evidence, notes)
            self.assertEqual(MODULE.validate(claims, evidence, notes), 1)

    def test_changed_retained_artifact_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            evidence = pathlib.Path(directory)
            notes = evidence / "CHANGELOG.md"
            claims = self.manifest(evidence, notes)
            (evidence / "benchmark.json").write_text("changed\n")
            with self.assertRaisesRegex(ValueError, "digest changed"):
                MODULE.validate(claims, evidence, notes)


if __name__ == "__main__":
    unittest.main()
