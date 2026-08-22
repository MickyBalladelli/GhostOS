# 12. Identity, Security, Observability, and Recovery

Security in GhostOS is not one login screen. It is the combined behavior of identity, capabilities, isolation, audit, and failure recovery.

## Authentication

`ghostos-auth` stores bounded local and node-local identity records with passkey, TPM 2.0, and SSH public credentials. Authentication uses one-shot challenges and a platform crypto verifier. A successful session creates a kernel-owned persona and mints only the configured initial capability set.

Remote administration uses WebAuthn-style ceremonies. Assertions bind to a server nonce, device, relying-party ID, origin, credential, expiry, presence, verification, and monotonic authenticator counter.

## Capability tokens

Cross-node tokens use HMAC-SHA256 wire records binding issuer, borrower, resource, transport, expiry, and revocation epoch. Macaroon-like caveats only narrow authority.

Remote tokens cannot gain dangerous rights such as map, create, delegate, or revoke unless an explicit trusted policy permits them. Federation and DSM checks require both a valid token and the current DLM epoch.

## Runtime shield and confidential computing

`ghostos-shield` provides bounded runtime protection and hardware admission. `ghostos-confidential` handles attestation, enclave admission, confidential fabric transport, and downgrade rejection. Policy decides whether a workload can run with CXL-IDE, SEV, TDX, CCA, or kernel-only isolation.

## Audit and observability

`ghostos-observability` defines bounded events, fields, levels, traces, and metrics. `ghostos-auditd` records security-sensitive actions such as authentication, authorization, package verification, filesystem mutation, fencing, patching, and destructive operations.

Important evidence fields include:

- principal;
- operation;
- target resource;
- capability or policy decision;
- node and epoch;
- result status;
- correlation ID;
- timestamp or monotonic sequence.

`ghostos-logd` handles bounded logging, filtering, sinks, flush, rotation, and restart. Sinks can fail without making the core event model unbounded.

## Inspection and debugging

`ghostos-inspect` provides capability-scoped system inspection. `ghostos-debug` provides protected probes, GDB protocol support, breakpoints, watchpoints, register/memory access, and coredumps. Remote GDB sessions bind a signed, expiring capability to one process resource, require the dedicated debug right for every operation, and recheck revocation before each packet. Debugging authority is itself a capability.

`ghostos-top` renders node memory, VRAM, DSM latency, and capability trees. It can target ANSI terminals and VGA/GOP output.

## Replay and healing

`ghostos-replay` records deterministic inputs, checkpoints, and flight recorder state. A replay detects divergent input and can restore a previous checkpoint.

`ghostos-heal` selects clean CoW snapshots, restores daemon state, preserves connections where safe, and limits repeated crash loops.

## Safe recovery order

For an unsafe cluster node:

```text
inspect -> drain -> fence -> reconcile -> recover/rejoin
```

Do not release locks, memory ownership, or storage claims before isolation is confirmed. Use abandon only when reconciliation is impossible.
