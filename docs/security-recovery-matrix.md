# Key and trust recovery test matrix

The security recovery regression set covers six boundaries. Tests use fixed
keys, content IDs, nonces, sequences, revisions, and measurements so failures
are replayable.

| Boundary | Direct regression | Covered behavior |
| --- | --- | --- |
| Boot handoff | `ghostos-boot-protocol::tests::boot_recovery_rejects_replay_rollback_downgrade_and_untrusted_roots` | replayed regions, old and future protocol versions, bad magic, BIOS recovery |
| Packages | `ghostos-pkg/tests/security_recovery.rs::package_key_rotation_revocation_and_generation_downgrade_are_fenced` | key rotation, receipt revocation, stale root rollback/downgrade, recovered signing root |
| Applications | `ghostos-pkg/tests/security_recovery.rs::application_signature_rotation_replay_and_rollback_are_fenced` | signed app replay, application key rotation/revocation, activation recovery |
| Compiler toolchains | `ghostos-rustd/tests/security_recovery.rs::compiler_toolchain_rotation_revocation_replay_rollback_and_downgrade_are_fenced` | package-backed toolchain rotation/revocation, stage predecessor replay, target downgrade, trust-root recovery |
| Cluster membership | `ghostos-shield/tests/security_recovery.rs::cluster_key_rotation_revocation_replay_rollback_and_root_recovery_are_fenced` | peer key rotation/revocation, sequence replay and rollback, re-admission under recovered key |
| Confidential evidence | `ghostos-confidential/tests/security_recovery.rs::evidence_key_rotation_revocation_replay_downgrade_and_root_recovery_are_fenced` | attestation key rotation/revocation, capability invalidation, quote replay, encrypted-frame replay/version downgrade, recovered KEM root |

Production gates added for this matrix:

- `PackageDaemon::revoke_key` removes the trust key and all instantiation
  receipts issued by it.
- `AdmissionController::rotate_key` and `revoke` reset or remove node trust.
- `EnclaveManager` rotates attestation keys and invalidates old capabilities.
- `FrameVerifier::revoke_peer` removes cluster peer trust.

Compile evidence for the test targets:

```text
cargo check -p ghostos-boot-protocol -p ghostos-pkg -p ghostos-shield \
  -p ghostos-confidential -p ghostos-rustd --tests
```

Test execution remains a separate local step because this repository's agent
workflow does not execute tests automatically.
