# IPC safety contract

## Endpoints

- An endpoint capability is owned by one address space.
- Closing a send endpoint revokes that capability and all authority derived from it.
- Closing a receive endpoint closes the channel, rejects later sends and receives, and drains requests that no receiver can service.
- Process teardown calls `Channel::cleanup_owner` for every owned channel. Cleanup is idempotent because stale generation-checked handles are ignored.

## Shared buffers

- Every guarded shared region is registered to one principal with a generation, rights mask, and unguessable token.
- A transfer requires `TRANSFER`, no active lease, and changes the registered owner.
- Revocation rejects new authorization immediately. The registry retains a revoking slot until every unique in-flight lease permit is released, then advances the generation before reuse.
- Process teardown moves all its live buffers into revocation.

## Backpressure and deadlock

- Rings remain bounded and non-blocking. A full ring returns `IpcError::Full`; quota rejection returns `RateLimited` with a retry delay.
- A blocked sender propagates priority to the endpoint owner.
- The scheduler rejects a wait edge that would create a cycle with `IpcDeadlock`; it never installs a cyclic priority-inheritance graph.

## Quotas and fairness

- Send and receive work consumes the caller endpoint's IPC token bucket.
- Delegated endpoints retain a private bucket and also charge every parent bucket. A process cannot escape the service-wide limit by creating child capabilities.
- Quota accounting uses fair ticket locks per resource, and a failed enqueue or empty receive refunds its charge.

## Diagnostics

- Channels count enqueue, dequeue, full, and rate-limited events and retain a high-water mark and progress timestamps.
- Send, receive, saturation, throttling, and close paths emit IPC trace events with channel and correlation data.
- `Channel::stuck_report` reports pending depth, stall duration, saturation count, and the latest request correlation and label after a caller-selected threshold.
