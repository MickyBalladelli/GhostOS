import SwiftUI

public struct ControlRootView: View {
    @ObservedObject private var store: ClusterStore

    public init(store: ClusterStore) {
        self.store = store
    }

    public var body: some View {
        TabView {
            NavigationStack {
                ClusterDashboardView(store: store)
            }
            .tabItem {
                Label("Cluster", systemImage: "server.rack")
            }

            NavigationStack {
                JobSubmissionView(store: store)
            }
            .tabItem {
                Label("Jobs", systemImage: "terminal")
            }

            NavigationStack {
                CapabilityDelegationView(store: store)
            }
            .tabItem {
                Label("Delegate", systemImage: "key")
            }
        }
        .overlay(alignment: .bottom) {
            if let error = store.errorMessage {
                Text(error)
                    .font(.caption)
                    .padding(10)
                    .background(.red.opacity(0.9), in: Capsule())
                    .foregroundStyle(.white)
                    .padding()
            }
        }
    }
}
