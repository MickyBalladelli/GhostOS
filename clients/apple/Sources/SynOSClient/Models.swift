import Foundation

public enum NodeHealth: UInt8, Sendable {
    case healthy = 1
    case degraded = 2
    case failed = 3
}

public struct ClusterNode: Identifiable, Equatable, Sendable {
    public let id: UInt32
    public let health: NodeHealth
    public let cpuLoadPermille: UInt16
    public let memoryUsedBytes: UInt64
    public let memoryTotalBytes: UInt64
    public let runningJobs: UInt32
    public let queuedJobs: UInt32

    public var cpuFraction: Double {
        Double(cpuLoadPermille) / 1_000
    }

    public var memoryFraction: Double {
        guard memoryTotalBytes > 0 else {
            return 0
        }
        return Double(memoryUsedBytes) / Double(memoryTotalBytes)
    }
}

public struct ClusterState: Equatable, Sendable {
    public let generation: UInt64
    public let sampledAtMicroseconds: UInt64
    public let nodes: [ClusterNode]
}

public enum ClusterLifecycle: UInt8, Sendable {
    case creating = 1
    case pendingAdmission = 2
    case active = 3
    case degraded = 4
    case partitioned = 5
    case draining = 6
    case leaving = 7
    case retired = 8
    case deleted = 9
}

public enum ClusterHealth: UInt8, Sendable {
    case healthy = 1
    case degraded = 2
    case partitioned = 3
    case unavailable = 4
}

public struct ClusterSummary: Equatable, Sendable {
    public let id: UInt128Value
    public let name: String
    public let lifecycle: ClusterLifecycle
    public let health: ClusterHealth
    public let generation: UInt64
    public let sampledAtMicroseconds: UInt64
    public let leader: UInt32
    public let coordinator: UInt32
    public let memberCount: UInt16
    public let healthyMembers: UInt16
    public let votingMembers: UInt16
    public let quorumRequired: UInt16
    public let quorumAvailable: UInt16
}

public struct UInt128Value: Equatable, Hashable, Sendable, CustomStringConvertible {
    public let high: UInt64
    public let low: UInt64

    public var description: String {
        String(format: "%016llx%016llx", high, low)
    }
}

public struct ClusterHealthSnapshot: Equatable, Sendable {
    public let generation: UInt64
    public let health: ClusterHealth
    public let quorum: Bool
    public let heartbeatPeriodMicroseconds: UInt64
    public let missedHeartbeatLimit: UInt16
    public let lastChangeMicroseconds: UInt64
    public let healthyNodes: UInt16
    public let degradedNodes: UInt16
    public let failedNodes: UInt16
}

public struct ClusterResources: Equatable, Sendable {
    public let generation: UInt64
    public let cpuCapacity: UInt64
    public let cpuAvailable: UInt64
    public let memoryCapacityBytes: UInt64
    public let memoryAvailableBytes: UInt64
    public let cxlCapacityBytes: UInt64
    public let cxlAvailableBytes: UInt64
    public let storageCapacityBytes: UInt64
    public let storageAvailableBytes: UInt64
    public let networkBandwidthMbps: UInt64
    public let acceleratorCapacity: UInt64
    public let acceleratorAvailable: UInt64
}

public struct ClusterInvitation: Identifiable, Equatable, Sendable {
    public let id: UInt64
    public let node: UInt32
    public let state: UInt8
    public let expiresAtMicroseconds: UInt64
}

public struct ClusterAuditEvent: Identifiable, Equatable, Sendable {
    public let id: UInt64
    public let timestampMicroseconds: UInt64
    public let actor: UInt32
    public let operation: UInt16
    public let status: UInt16
    public let target: UInt64
}

public struct ClusterAlert: Identifiable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let detail: String
    public let critical: Bool

    public init(id: String, title: String, detail: String, critical: Bool) {
        self.id = id
        self.title = title
        self.detail = detail
        self.critical = critical
    }
}

public enum TopologyTransport: UInt8, Sendable {
    case cxl = 1
    case ethernet = 2
    case wireless = 3
    case cellular5g = 4
    case loopback = 5
    case tunnel = 6
}

public enum TopologyRoute: UInt8, Sendable {
    case direct = 1
    case nat = 2
    case relay = 3
    case offline = 4
}

public enum TopologyReachability: UInt8, Sendable {
    case unknown = 1
    case reachable = 2
    case unreachable = 3
    case offline = 4
}

public struct TopologyLink: Identifiable, Equatable, Sendable {
    public let id: String
    public let from: UInt32
    public let to: UInt32
    public let transport: TopologyTransport
    public let route: TopologyRoute
    public let reachability: TopologyReachability
    public let latencyMicroseconds: UInt64
    public let bandwidthMbps: UInt64
    public let mtu: UInt16
}

public struct TopologyState: Equatable, Sendable {
    public let generation: UInt64
    public let sampledAtMicroseconds: UInt64
    public let links: [TopologyLink]
}

public struct JobReceipt: Equatable, Sendable {
    public let jobID: UInt64
    public let acceptedAtMicroseconds: UInt64
}

public struct CapabilityDelegation: Equatable, Sendable {
    public let resource: UInt64
    public let subjectNode: UInt32
    public let rights: CapabilityRights
    public let transports: TransportRights
    public let validForMicroseconds: UInt64

    public init(
        resource: UInt64,
        subjectNode: UInt32,
        rights: CapabilityRights,
        transports: TransportRights,
        validForMicroseconds: UInt64
    ) {
        self.resource = resource
        self.subjectNode = subjectNode
        self.rights = rights
        self.transports = transports
        self.validForMicroseconds = validForMicroseconds
    }
}

public struct CapabilityRights: OptionSet, Equatable, Sendable {
    public let rawValue: UInt16

    public init(rawValue: UInt16) {
        self.rawValue = rawValue
    }

    public static let read = Self(rawValue: 1 << 0)
    public static let write = Self(rawValue: 1 << 1)
    public static let execute = Self(rawValue: 1 << 2)
    public static let map = Self(rawValue: 1 << 3)
    public static let create = Self(rawValue: 1 << 4)
    public static let send = Self(rawValue: 1 << 5)
    public static let receive = Self(rawValue: 1 << 6)
    public static let delegate = Self(rawValue: 1 << 7)
    public static let revoke = Self(rawValue: 1 << 8)
}

public struct TransportRights: OptionSet, Equatable, Sendable {
    public let rawValue: UInt8

    public init(rawValue: UInt8) {
        self.rawValue = rawValue
    }

    public static let cxl = Self(rawValue: 1)
    public static let layer2 = Self(rawValue: 2)
}

public struct SynOSCapability: Equatable, Sendable {
    public static let wireBytes = 192

    public let bytes: Data

    public init(bytes: Data) throws {
        guard bytes.count == Self.wireBytes,
              bytes.prefix(4) == Data([0x53, 0x59, 0x43, 0x41]),
              bytes[4] == 1 else {
            throw SynOSClientError.invalidCapability
        }
        self.bytes = bytes
    }
}
