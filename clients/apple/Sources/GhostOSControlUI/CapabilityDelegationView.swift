import SwiftUI

public struct CapabilityDelegationView: View {
    @ObservedObject private var store: ClusterStore
    @State private var resource = ""
    @State private var subjectNode = ""
    @State private var expiresAt = Date().addingTimeInterval(3_600)

    public init(store: ClusterStore) {
        self.store = store
    }

    public var body: some View {
        Form {
            Section("Restricted grant") {
                TextField("Resource ID", text: $resource)
                TextField("Subject node", text: $subjectNode)
                DatePicker(
                    "Expires",
                    selection: $expiresAt,
                    in: Date()...
                )
                Button("Delegate read access") {
                    guard let resourceID = UInt64(resource),
                          let nodeID = UInt32(subjectNode) else {
                        return
                    }
                    Task {
                        await store.delegate(
                            resource: resourceID,
                            subjectNode: nodeID,
                            expiresAt: expiresAt
                        )
                    }
                }
                .disabled(
                    UInt64(resource) == nil
                        || UInt32(subjectNode) == nil
                        || store.isLoading
                )
            }
            if let capability = store.delegatedCapability {
                Section("Delegated capability") {
                    Text(capability.bytes.base64EncodedString())
                        .font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                }
            }
        }
        .navigationTitle("Delegate")
    }
}
