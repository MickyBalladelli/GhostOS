# Retained VM fuzz failures

`scripts/fuzz-smoke.sh` copies every VM crash, timeout, OOM, or leak artifact
into `fuzz/corpus/<target>/regression-<sha256>`. Metadata in this directory
records the revision, stable inventory ID, digest, and deterministic replay
command.

Replay one retained input from the repository root:

```sh
scripts/replay-vm-fuzz.sh vm-snapshot fuzz/corpus/vm-snapshot/regression-<sha256>
```

Keep the input after fixing the bug. A fixed input becomes a permanent corpus
regression and must complete without reproducing the failure.
