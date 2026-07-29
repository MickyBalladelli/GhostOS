import SwiftUI
import SynOSClient
import SynOSControlUI

@main
struct SynOSControlApp: App {
    @StateObject private var store: ClusterStore

    init() {
        let configuredEndpoint = ProcessInfo.processInfo.environment["SYNOS_GATEWAY_URL"]
        let endpoint = URL(
            string: configuredEndpoint ?? "http://127.0.0.1:8443/rpc"
        )!
        let client = SynOSClient(transport: HTTPTransport(endpoint: endpoint))
        _store = StateObject(wrappedValue: ClusterStore(client: client))
    }

    var body: some Scene {
        WindowGroup {
            ControlRootView(store: store)
        }
    }
}
