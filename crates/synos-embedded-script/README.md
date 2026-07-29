# synos-embedded-script

Bounded, `no_std` Rhai automation for SynOS services.

Scripts receive one `synos` object. They can inspect and enqueue only operations
granted by the caller:

```text
if synos.has("cluster/jobs", 2) {
    synos.request("cluster/jobs", 2, "rebalance")
}
```

The engine limits source bytes, instructions, recursion, expression depth,
functions, modules, variables, collections, strings, queued requests, and
payload bytes. It performs no system I/O itself. A service validates and
executes the returned typed requests.
