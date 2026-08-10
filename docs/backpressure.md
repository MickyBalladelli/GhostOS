# Backpressure contracts

All bounded producer/consumer queues use one of four explicit contracts:

- **Fail fast:** return a full/capacity error; the producer keeps ownership and may retry.
- **Drop:** accept the producer call but discard work when full; the drop counter or error is observable.
- **Retry:** return a retryable result and keep the item queued or eligible for another attempt.
- **Block:** wait for consumer progress. No kernel or no-std queue in this repository blocks.

## Queue matrix

| Producer → consumer | Queue | Full behavior | Empty/consumer behavior | Evidence |
| --- | --- | --- | --- | --- |
| IPC clients → Ring 0 | `synos_ipc::Ring`, `kernel::ipc::Channel` | Fail fast: `RingError::Full` / `IpcError::Full`; IPC quota is refunded | Fail fast: `Empty` | `crates/ipc/src/tests.rs`, `kernel/src/tests.rs` |
| Clients → device driver | `synos_platform_io::AsyncQueue`, `IoQueue`, `MediaQueue`, accelerator queue | Fail fast: `QueueFull` | `dispatch`/`poll` return `None` | `crates/platform-io/tests/model.rs` |
| Filesystem → block backend | `synos_synfs::BlockIoQueue` | Fail fast: `BlockIoError::QueueFull` | `poll` returns `None`; cancellation frees a slot | `crates/synfs/tests/coverage_59_5.rs` |
| NIC driver ↔ network stack | `synos_netd::PacketQueue` | Fail fast: `PacketError::Full`; dropped loan frees the slot | `PacketError::Empty` | `crates/netd/tests/coverage_59_7.rs` |
| Socket client → network daemon | `ClientChannel` request/completion rings | Fail fast: completion-full is returned before consuming a request | Empty request ring means no work | `crates/netd/src/service.rs` |
| Storage client → storage daemon | `StorageDaemon` pending/completion arrays | Fail fast: `StorageError::QueueFull`; completion failure is surfaced in the completion | `complete_next`/`poll_completion` are non-blocking | `crates/synos-storaged/tests/coverage_59_5.rs` |
| Remote storage → NVMe transport | `synos_storaged::NvmeQueue` | Fail fast: `NvmeError::QueueFull` | `complete` returns `None` for unknown work | `crates/synos-storaged/tests/coverage_59_5.rs` |
| Shell producers → workers | `syn_shell::JobQueue` | Fail fast: `Error::QueueFull`; completed slots require reap | No eligible job returns `Ok(None)`; expired leases retry | `crates/syn-shell/tests/coverage_59_6.rs` |
| Context producers → embedding worker | `synos_agentd::ContextBus` | Fail fast: `AgentError::QueueFull`; same source coalesces/replaces | `poll` drains within its budget | `crates/synos-agentd/tests/coverage_59_8.rs` |
| Read observer → prefetch worker | `PredictivePrefetcher` | Best-effort drop: `observe` stops at queue capacity and returns partial count | `pop` returns `None`; no retry is promised | `crates/llm-runtime/src/prefetch.rs` |
| Kernel/device producers → trace consumer | `TraceRing` | Drop oldest and increment `dropped`; never blocks | `try_pop` returns `None` | `crates/observability/tests/coverage_59_9.rs` |
| Audit producers → recovery journal | `AuditJournal` | Fail fast: `AuditJournalError::Capacity`; never overwrites evidence | Records are read/exported explicitly | `crates/observability/src/lib.rs` |
| Metric producers → metric consumer | `MetricRegistry` | Fail fast with `MetricError::Capacity` and increment `dropped` | `samples`/`export` drain by copy | `crates/observability/src/lib.rs` |
| Alert producers → alert consumer | `AlertRegistry` | Fail fast with `MetricError::Capacity` and increment `dropped` | `drain` removes entries | `crates/observability/src/lib.rs` |
| Replay producers → replay consumer | `ReplayRing` | Drop oldest and increment `dropped`; never blocks | `try_pop` returns `None` | `crates/synos-replay/tests/coverage_59_9.rs` |

## VM device queues

The VM keeps the same rule visible at its host boundary. Network packet queues
return `NetError::QueueFull`; loopback and deterministic segments cap each
receive queue at 256 packets. E1000 records queue-full as its last network
error. UART input has two deliberate modes: `push_input` drops excess bytes
and raises overrun, while the internal lossless paste path grows a pending
buffer until the guest drains it. PS/2 output drops bytes after its 64-byte
queue is full. Guest-agent, power-notification, cluster-delivery, and monitor
nonce `VecDeque`s are host control buffers with no fixed limit; they grow and
therefore do not provide backpressure.

Hardware descriptor queues (USB, NVMe, virtio) are bounded by guest-programmed
ring depth. Their device models consume descriptors without blocking; invalid
or unavailable completion space is reported through the device status path.
