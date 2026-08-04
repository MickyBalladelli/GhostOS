import Foundation
import SynOSClient

@MainActor
public final class ClusterStore: ObservableObject {
    @Published public private(set) var cluster: ClusterState?
    @Published public private(set) var summary: ClusterSummary?
    @Published public private(set) var health: ClusterHealthSnapshot?
    @Published public private(set) var resources: ClusterResources?
    @Published public private(set) var topology: TopologyState?
    @Published public private(set) var invitations: [ClusterInvitation] = []
    @Published public private(set) var recentActions: [ClusterAuditEvent] = []
    @Published public private(set) var alerts: [ClusterAlert] = []
    @Published public private(set) var lastJob: JobReceipt?
    @Published public private(set) var delegatedCapability: SynOSCapability?
    @Published public private(set) var errorMessage: String?
    @Published public private(set) var isLoading = false

    private let client: SynOSClient

    public init(client: SynOSClient) {
        self.client = client
    }

    public func refresh() async {
        await perform {
            self.cluster = try await self.client.clusterState()
            self.summary = try await self.client.clusterSummary()
            self.health = try await self.client.clusterHealth()
            self.alerts = makeAlerts(from: self.health)
            self.resources = try await self.client.clusterResources()
            self.topology = try await self.client.topologyState()
            self.invitations = try await self.client.clusterInvitations()
            self.recentActions = try await self.client.clusterAudit()
        }
    }

    public func submit(
        command: String,
        priority: UInt8,
        timeoutSeconds: UInt64
    ) async {
        await perform {
            self.lastJob = try await self.client.submitJob(
                command: command,
                priority: priority,
                timeoutMicroseconds: timeoutSeconds * 1_000_000
            )
        }
    }

    public func delegate(
        resource: UInt64,
        subjectNode: UInt32,
        expiresAt: Date
    ) async {
        await perform {
            let lifetime = UInt64(
                max(1, expiresAt.timeIntervalSinceNow) * 1_000_000
            )
            self.delegatedCapability = try await self.client.delegateCapability(
                CapabilityDelegation(
                    resource: resource,
                    subjectNode: subjectNode,
                    rights: [.read, .send],
                    transports: [.layer2],
                    validForMicroseconds: lifetime
                )
            )
        }
    }

    private func perform(_ operation: () async throws -> Void) async {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }
        do {
            try await operation()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func makeAlerts(from health: ClusterHealthSnapshot?) -> [ClusterAlert] {
        guard let health else {
            return []
        }
        var alerts: [ClusterAlert] = []
        if !health.quorum {
            alerts.append(ClusterAlert(id: "quorum", title: "Quorum lost", detail: "Cluster is read-only until quorum returns.", critical: true))
        }
        if health.failedNodes > 0 {
            alerts.append(ClusterAlert(id: "failed-nodes", title: "Failed nodes", detail: "\(health.failedNodes) node(s) need recovery.", critical: true))
        }
        return alerts
    }
}
