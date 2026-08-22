# GhostOS Apple control client

This Swift package shares one native client and SwiftUI control surface across
macOS 14+ and iOS 17+. It speaks the versioned `SYRP` binary protocol exposed by
`ghostos-client-sdk`.

The Swift client follows the stable SDK contract in
[`docs/sdk-compatibility.md`](../../docs/sdk-compatibility.md).

The control surface shows cluster health, submits bounded jobs, and requests
attenuated capability grants. Capability bytes travel inside the RPC envelope;
the app never receives an issuer key.

The Swift client can also be embedded in your own macOS or iOS app. `GhostOSClient`
reads cluster identity, health, members, capacity, topology, alerts, pending
admissions, and recent actions; it can submit bounded jobs and delegate limited
capabilities to another node. `ControlRootView` provides the ready-made SwiftUI
dashboard, Jobs, and Delegate screens. Set `GHOSTOS_GATEWAY_URL` to point the
sample app at a GhostOS HTTP gateway; the default is
`http://127.0.0.1:8443/rpc`. Rust supports lifecycle mutations and live polling
through its SDK, while the Swift UI currently exposes the read and action flows
listed above.

Run the macOS app from Xcode or:

```sh
GHOSTOS_GATEWAY_URL=https://cluster.example/rpc swift run GhostOSControl
```

For iOS, add this directory as a local Swift package in an Xcode app and use
`ControlRootView` as the root view. Configure the client with `HTTPTransport`
and a TLS gateway URL. The library targets already declare iOS support.

Android and browser clients use the Rust SDK through their normal FFI or Wasm
transport bridge. Implement `RpcTransport` with the platform HTTP stack; the SDK
does not assume a socket API or async executor.
