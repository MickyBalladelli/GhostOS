# Retained fuzz failures

`scripts/fuzz-smoke.sh` copies every crash, hang, timeout, OOM, leak, or
excessive-allocation artifact into `fuzz/corpus/<target>/regression-<sha256>`.
Metadata in the target directory records the boundary, owner, revision, stable
inventory ID, failure class, digest, and deterministic replay command.

The complete target-to-boundary ownership map is
[`fuzz/triage.toml`](../triage.toml). Validate it with
`scripts/validate-fuzz-inventory.py`.

Replay one retained input from the repository root:

```sh
scripts/replay-fuzz.sh vm-snapshot fuzz/corpus/vm-snapshot/regression-<sha256>
```

Keep the input after fixing the bug. A fixed input becomes a permanent corpus
regression and must complete without reproducing the failure.
