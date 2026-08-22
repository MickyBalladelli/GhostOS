import SwiftUI

public struct ClusterDashboardView: View {
    @ObservedObject private var store: ClusterStore

    public init(store: ClusterStore) {
        self.store = store
    }

    public var body: some View {
        Group {
            if let cluster = store.cluster {
                List {
                    if let summary = store.summary {
                        Section("Identity") {
                            LabeledContent("Name", value: summary.name)
                            LabeledContent("ID", value: summary.id.description)
                            LabeledContent("Lifecycle", value: String(describing: summary.lifecycle))
                            LabeledContent("Health", value: String(describing: summary.health))
                            LabeledContent("Generation", value: String(summary.generation))
                            LabeledContent("Coordinator", value: "Node \(summary.coordinator)")
                        }
                    }
                    if let health = store.health {
                        Section("Health") {
                            LabeledContent("Quorum", value: health.quorum ? "Available" : "Lost")
                            LabeledContent("Healthy", value: String(health.healthyNodes))
                            LabeledContent("Degraded", value: String(health.degradedNodes))
                            LabeledContent("Failed", value: String(health.failedNodes))
                        }
                    }
                    if !store.alerts.isEmpty {
                        Section("Alerts") {
                            ForEach(store.alerts) { alert in
                                VStack(alignment: .leading) {
                                    Text(alert.title)
                                        .foregroundStyle(alert.critical ? .red : .primary)
                                    Text(alert.detail)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                    Section("Nodes") {
                        ForEach(cluster.nodes) { node in
                            NodeRowView(node: node)
                        }
                    }
                    if let resources = store.resources {
                        Section("Capacity") {
                            LabeledContent("CPU", value: "\(resources.cpuAvailable) / \(resources.cpuCapacity)")
                            LabeledContent("Memory", value: "\(resources.memoryAvailableBytes) / \(resources.memoryCapacityBytes) bytes")
                            LabeledContent("CXL", value: "\(resources.cxlAvailableBytes) / \(resources.cxlCapacityBytes) bytes")
                            LabeledContent("Storage", value: "\(resources.storageAvailableBytes) / \(resources.storageCapacityBytes) bytes")
                            LabeledContent("Network", value: "\(resources.networkBandwidthMbps) Mbps")
                        }
                    }
                    if !store.invitations.isEmpty {
                        Section("Pending Admissions") {
                            ForEach(store.invitations) { invitation in
                                LabeledContent("Node \(invitation.node)", value: "State \(invitation.state)")
                            }
                        }
                    }
                    if let topology = store.topology {
                        Section("Connectivity") {
                            ForEach(topology.links) { link in
                                VStack(alignment: .leading) {
                                    Text("Node \(link.from) → \(link.to)")
                                    Text("\(link.latencyMicroseconds) µs · \(link.bandwidthMbps) Mbps · MTU \(link.mtu)")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                    if !store.recentActions.isEmpty {
                        Section("Recent Actions") {
                            ForEach(store.recentActions) { action in
                                LabeledContent("Operation \(action.operation)", value: "Node \(action.actor) · status \(action.status)")
                            }
                        }
                    }
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
