# SynOS Apple control client

This Swift package shares one native client and SwiftUI control surface across
macOS 14+ and iOS 17+. It speaks the versioned `SYRP` binary protocol exposed by
`synos-client-sdk`.

The control surface shows cluster health, submits bounded jobs, and requests
attenuated capability grants. Capability bytes travel inside the RPC envelope;
the app never receives an issuer key.

Run the macOS app from Xcode or:

```sh
SYNOS_GATEWAY_URL=https://cluster.example/rpc swift run SynOSControl
```

For iOS, add this directory as a local Swift package in an Xcode app and use
`ControlRootView` as the root view. Configure the client with `HTTPTransport`
and a TLS gateway URL. The library targets already declare iOS support.

Android and browser clients use the Rust SDK through their normal FFI or Wasm
transport bridge. Implement `RpcTransport` with the platform HTTP stack; the SDK
does not assume a socket API or async executor.
