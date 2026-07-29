import SwiftUI

public struct ClusterDashboardView: View {
    @ObservedObject private var store: ClusterStore

    public init(store: ClusterStore) {
        self.store = store
    }

    public var body: some View {
        Group {
            if let cluster = store.cluster {
                List(cluster.nodes) { node in
                    NodeRowView(node: node)
                }
                .overlay {
                    if cluster.nodes.isEmpty {
                        ContentUnavailableView(
                            "No cluster nodes",
                            systemImage: "server.rack"
                        )
                    }
                }
            } else {
                ContentUnavailableView(
                    "No cluster sample",
                    systemImage: "waveform.path.ecg",
                    description: Text("Refresh to contact the gateway.")
                )
            }
        }
        .navigationTitle("Cluster")
        .toolbar {
            Button {
                Task { await store.refresh() }
            } label: {
                Label("Refresh", systemImage: "arrow.clockwise")
            }
            .disabled(store.isLoading)
        }
        .task {
            if store.cluster == nil {
                await store.refresh()
            }
        }
    }
}
