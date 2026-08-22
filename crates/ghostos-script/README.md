# ghostos-script

`ghostos-script` is GhostOS's heap-free native command-procedure engine. It compiles
bounded DCL-style source into fixed-capacity statements and drives the existing
typed `ghostos-shell` command registry cooperatively.

```text
$ SET SYMBOL NODE = 7
$ DEFINE /PROCESS /FILE INPUT = "data/events.rms"
$ SHOW NODE ${NODE} | SELECT /STATE=READY
$ SET NOON
$ IF FAILURE THEN SHOW STATUS $STATUS
$ ATTENUATE ADMIN AS READER /RIGHTS=READ /TRANSPORT=CXL /EXPIRES=5000000
$ EXIT $STATUS
```

Command arguments are validated against `CommandSpec` before dispatch. A
pipeline stage receives the previous stage's `StructuredOutput`, never a text
stream. The `wire` module encodes the same arguments and output fields into a
checksummed `ghostos-ipc` shared-region payload; only its `SharedBuffer`
descriptor crosses the IPC ring.

Logical-name verbs support process, job, group, system, and cluster scopes plus
file, device, and IPC-channel targets. `AuthorizedLogicalNames` routes those
verbs through `CapabilityLogicalNames`, so script syntax cannot bypass the
kernel capability and logical-name ACL checks.

Capability symbols hold cryptographic tokens. `ATTENUATE` adds a Macaroon-style
caveat with reduced rights, transport, expiry, and optional subject. It cannot
mint or widen authority.

`ScriptEngine::status()` is the script's `$STATUS`. Odd values mean success.
Procedures stop on a failed statement by default. `SET NOON` or
`ON ERROR THEN CONTINUE` keeps execution active so `IF FAILURE THEN ...` can
handle the condition; `SET ON` restores failure propagation.

## Agent-safe execution

`ToolSchemaExporter` reflects every registered `CommandSpec` into a bounded,
OpenAI-compatible function-tool JSON array. It preserves command and argument
registration order, includes required and optional parameter types, rejects
normalized name collisions, and resolves emitted function names back to their
command routes without a heap.

`CowSandbox` gives an agent executor an exclusive GhostFS transaction. A
`SandboxExecutor` drives `ScriptEngine` commands through a
`SandboxCommandHandler` in stable pipeline order. Writes and deletes build a
private CoW root, while reads see that staged root for validation.
`SandboxDecision::Discard`, any failed transaction, or dropping the sandbox
restores the original generation. `SandboxDecision::Commit` publishes all
validated operations as one atomic root change and returns a receipt with the
base, staged, and resulting generations.
