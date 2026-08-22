# 11. Scripts, Wasm, Applications, Jobs, and Actors

GhostOS has several automation layers. Each has a different trust level and resource contract.

## `ghostos-script`

Native command procedures use DCL-style statements and typed command routes. The compiler validates pipeline stages against the command registry. Structured records, not text scraping, flow between stages.

The script language includes:

- scoped logical names;
- typed symbols;
- comments and quoting;
- conditions and pipelines;
- `$STATUS`;
- `IF SUCCESS` and `IF FAILURE`;
- `SET ON` and `SET NOON`;
- `ON ERROR THEN`;
- capability attenuation;
- bounded wire encoding.

Easy script shape:

```text
SET NOON
SHOW CLUSTER/HEALTH
IF FAILURE THEN GOTO RECOVER
SHOW MEMORY/CLUSTER
EXIT $STATUS
```

The important difference from a host shell is that the command registry is typed and the status remains structured.

## Agent sandbox

`ghostos-agent-bridge` exports live command schemas as function-tool JSON. It derives short-lived capabilities bound to one agent and exact task rights. A grant is consumed through a replay ledger.

Sandbox execution uses an isolated private GhostFS root:

```text
base generation -> private staged root -> inspect/validate
                                  |
                reject/discard <---+---> approve/publish
```

`RUN /SANDBOX` always discards its changes. An approved normal run publishes only after successful completion and a write-authorized token.

## Embedded Rhai

`ghostos-embedded-script` embeds Rhai for service automation. Limits cover source size, instructions, recursion, expression depth, functions, variables, collections, strings, and queued work. Scripts can enqueue only operations in their capability list.

## Wasm

`ghostos-wasm-script` runs untrusted Wasm through the pure-Rust Wasmi interpreter. It exposes no WASI filesystem, network, environment, or clock. Compilation, module size, fuel, memory, table growth, imports, entry signatures, and host operations are all bounded.

## Applications

`ghostos-app` defines an `App.toml` contract. A manifest names an immutable image, application kind, node placement, restart policy, and exact capability requests. The supervisor intersects those requests with administrator policy before spawning.

Optional resources may be omitted. Required resources, wrong object kinds, and rights escalation stop admission.

## Jobs and actors

`ghostos-actors` gives local and distributed processes one actor API. Local mailboxes use native IPC. Remote references use DSM mailboxes with a live write authority and DLM epoch. Actor code does not construct packets or choose transports.

Jobs add scheduling, queues, leases, cancellation, retries, and recovery semantics. The goal is not only to start work, but to know whether work was accepted, running, cancelled, retried, or committed.

## Easy example: three trust levels

```text
trusted native command -> typed registry + capability
bounded service script  -> operation allowlist + instruction budget
untrusted Wasm          -> no host authority + fuel/memory limits
```

Use the weakest runtime that can solve the task.

