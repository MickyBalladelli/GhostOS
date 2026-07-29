# synos-wasm-script

`no_std` Wasmi sandbox for untrusted SynOS extensions.

Guest modules receive no WASI imports. The only host ABI is:

```text
synos.capability_check(handle: i64, operation: i32) -> i32
synos.invoke(handle: i64, operation: i32, a: i64, b: i64, c: i64, d: i64) -> i64
```

Each run has a fixed module-size and fuel budget, strict compilation checks,
memory/table/instance limits, disabled floating point, and an explicit
capability list. Unknown imports fail linking. Denied operations never reach
the host.
