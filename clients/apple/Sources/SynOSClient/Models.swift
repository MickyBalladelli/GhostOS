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
