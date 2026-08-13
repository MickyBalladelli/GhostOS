# SynOS durability contract

Contract version: `1` (`synos-durability::DURABILITY_CONTRACT`).

The contract has two states that callers must not confuse:

- **published** means the new state is visible to the running system.
- **durable** means the state survived the required device or remote-service
  durability fence and is eligible for recovery after power loss.

`write`, `rename`, and `commit` publish state. Only `fsync`/`sync` (or a
lower-layer `flush` that is explicitly part of the sync path) may report
durable state.

## End-to-end order

For transaction `T`, a successful end-to-end sync must observe this order:

```text
application write
  -> SynFS write staging
  -> SynFS commit / rename publication
  -> storage-daemon forwarding
  -> cache write and cache flush, when a cache is present
  -> block data and type-map writes
  -> block commit record (superblock)
  -> block-device flush
  -> sync acknowledgement
```

The commit record is written last. A power loss before the final flush may
discard `T`; a power loss after the flush may recover `T`. Recovery never
selects a generation merely because its header exists: it validates the map,
objects, root, and checksums first.

The fixed-capacity ordering proof in
[`crates/durability/tests/contract.rs`](../crates/durability/tests/contract.rs)
rejects missing fences, reordered data and commit records, and recovery of an
unacknowledged transaction.

## Layer guarantees

| Layer | `write` | `flush` | `sync` | `rename` | `commit` | power-loss recovery |
| --- | --- | --- | --- | --- | --- | --- |
| Application/runtime | Bytes handed to the filesystem; volatile | No durable meaning by itself | Must wait for the lower-layer fence before success | Volatile until sync | Delegated publication | May assume the result only after sync success |
| SynFS | New CoW version in memory | `SynFs::flush` writes a staging image; `flush_to_device` writes an inactive bank and fences the device | `SynFs::sync` is the durable volume barrier | Atomic metadata publication in memory | `SynFsTransaction::commit` atomically publishes a root, not power durability | Chooses the newest complete checksummed bank |
| Storage daemon | Request is queued or transport-accepted; volatile | `IoOperation::Flush` must forward and wait for the backend fence | Adapter must complete only after its remote/device fence | No filesystem rename guarantee | Request completion is not a commit record | Reopens only state confirmed by the backend |
| Cache | CoW entry becomes dirty; volatile | `CowCache::flush` writes dirty entries, calls `RemoteFileBackend::flush`, then clears dirty state | `CowCache::sync` has the same fenced guarantee | No rename guarantee | Dirty state is not committed state | Dirty entries remain retryable if write or fence fails |
| Block store/device | Block write may be cached | `BlockStore::flush` / `BlockDevice::flush` must wait for power-loss durability | Same fence used by SynFS sync | No rename guarantee | SynFS superblock is the commit record | Torn or incomplete writes are ignored by SynFS recovery |
| Recovery | Does not publish speculative state | Replays only fenced records | Returns the last complete durable generation | Restores the rename only when its generation is selected | Accepts only a complete generation | Never guesses between partial generations |

## Operation rules

### `write`

An application write returns after the bytes become part of a published
in-memory CoW generation. It does not promise power-loss durability. The
filesystem daemon's `write` path uses this rule.

### `commit`

`SynFsTransaction::commit` makes all staged operations visible with one root
swap. A failed transaction leaves the old root visible. It is an atomicity
boundary, not a device fence. Call `SynFs::sync` after commit when the caller
needs a recovery promise.

### `rename`

SynFS performs rename as one metadata transaction within one volume. The old
name and new name cannot be observed as a half-applied pair in one generation:
after publication, readers see the old name or the new name, never both. A
failed or abandoned transaction leaves the old root and old name visible. The
destination must not already exist; SynFS does not silently replace it. A
directory rename moves its complete subtree as one transaction. The rename is
volatile until the generation containing it reaches the block commit record
and device flush.

### `flush`, `fsync`, and `sync`

At the volume layer, `flush_to_device` and `sync` write all inactive-bank data
and metadata first, write the superblock last, then call `BlockStore::flush`.
`fsync` is the explicit public name for this same end-to-end operation. It
returns success only after the block-store durability fence completes; an
error does not claim that the new generation survived power loss. Because
SynFS commits a complete root generation, callers do not need a separate
parent-directory `fsync` after a successful file rename.
At the cache layer, `flush` first sends dirty data and then calls the remote
backend fence; dirty bits clear only after that fence succeeds. A lower-layer
write acknowledgement without its fence is not an end-to-end sync.

### Power loss

Recovery scans both volume banks and selects the newest complete checksummed
generation. A torn map, object, or superblock makes that bank ineligible, so
the previous complete generation remains the result. The disk-image failure
and recovery coverage lives in
[`crates/synfs/tests/persistence.rs`](../crates/synfs/tests/persistence.rs)
and [`crates/synfs/tests/coverage_59_5.rs`](../crates/synfs/tests/coverage_59_5.rs).
