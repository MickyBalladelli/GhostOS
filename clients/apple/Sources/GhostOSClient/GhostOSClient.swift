import Foundation

public struct GhostOSOperatorFailure: Error, Equatable, LocalizedError, Decodable, Sendable {
    public let code: UInt32
    public let operation: UInt16
    public let message: String
    public let action: String
    public let impact: String
    public let retrySafety: String
    public let retryAfterMicroseconds: UInt64?
    public let auditCorrelation: String
    public let auditNode: UInt32

    public var errorDescription: String? {
        message
    }

    private enum CodingKeys: String, CodingKey {
        case code
        case operation
        case message
        case action
        case impact
        case retrySafety = "retry_safety"
        case retryAfterMicroseconds = "retry_after_us"
        case audit
    }

    private enum AuditCodingKeys: String, CodingKey {
        case correlation
        case node
    }

    public init(
        code: UInt32,
        operation: UInt16,
        message: String,
        action: String,
        impact: String,
        retrySafety: String,
        retryAfterMicroseconds: UInt64?,
        auditCorrelation: String,
        auditNode: UInt32
    ) {
        self.code = code
        self.operation = operation
        self.message = message
        self.action = action
        self.impact = impact
        self.retrySafety = retrySafety
        self.retryAfterMicroseconds = retryAfterMicroseconds
        self.auditCorrelation = auditCorrelation
        self.auditNode = auditNode
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let audit = try container.nestedContainer(keyedBy: AuditCodingKeys.self, forKey: .audit)
        self.init(
            code: try container.decode(UInt32.self, forKey: .code),
            operation: try container.decode(UInt16.self, forKey: .operation),
            message: try container.decode(String.self, forKey: .message),
            action: try container.decode(String.self, forKey: .action),
            impact: try container.decode(String.self, forKey: .impact),
            retrySafety: try container.decode(String.self, forKey: .retrySafety),
            retryAfterMicroseconds: try container.decodeIfPresent(UInt64.self, forKey: .retryAfterMicroseconds),
            auditCorrelation: try audit.decode(String.self, forKey: .correlation),
            auditNode: try audit.decode(UInt32.self, forKey: .node)
        )
    }

    public static func rpcStatus(_ status: UInt16, requestID: UInt64) -> Self {
        let retryable = status == 5 || status == 6
        return Self(
            code: UInt32(status),
            operation: 5,
            message: "Gateway rejected the request with status \(status).",
            action: "Inspect the request and audit record before repeating it.",
            impact: "The requested operation did not complete.",
            retrySafety: retryable
                ? "Retry after the gateway says capacity is available."
                : "Do not retry automatically until the failure is understood.",
            retryAfterMicroseconds: retryable ? 1_000_000 : nil,
            auditCorrelation: String(requestID),
            auditNode: 0
        )
    }
}

public enum GhostOSClientError: Error, Equatable, LocalizedError, Sendable {
    case invalidCapability
    case abiMismatch
    case invalidFrame
    case invalidInput
    case mismatchedResponse
    case remoteStatus(UInt16)
    case operatorFailure(GhostOSOperatorFailure)
    case transportRejected
    case incompatibleApiVersion(code: String, offered: UInt16, minimum: UInt16, maximum: UInt16)

    public var errorDescription: String? {
        switch self {
        case .invalidCapability:
            "Capability token is not a GhostOS v1 token."
        case .abiMismatch:
            "Gateway uses an incompatible GhostOS ABI."
        case .invalidFrame:
            "Gateway returned an invalid RPC frame."
        case .invalidInput:
            "Request fields are not valid."
        case .mismatchedResponse:
            "Gateway response does not match the request."
        case let .remoteStatus(status):
            "Gateway rejected the request with status \(status)."
        case let .operatorFailure(failure):
            failure.errorDescription
        case .transportRejected:
            "Gateway transport rejected the request."
        case let .incompatibleApiVersion(code, offered, minimum, maximum):
            "\(code): GhostOS API version \(offered) is outside supported range \(minimum)..=\(maximum)."
        }
    }

    public var operatorFailure: GhostOSOperatorFailure? {
        if case let .operatorFailure(failure) = self {
            return failure
        }
        return nil
    }
}

public struct GhostOSPerformanceDiagnostics: Equatable, Sendable {
    public let queueWaitMicroseconds: UInt64
    public let serviceTimeMicroseconds: UInt64
    public let retries: UInt32
    public let requestBytes: UInt32
    public let responseBytes: UInt32
    public let tailLatencyMicroseconds: UInt64
    public let budgetExceeded: Bool

    public static let empty = GhostOSPerformanceDiagnostics(
        queueWaitMicroseconds: 0,
        serviceTimeMicroseconds: 0,
        retries: 0,
        requestBytes: 0,
        responseBytes: 0,
        tailLatencyMicroseconds: 0,
        budgetExceeded: false
    )

    private init(
        queueWaitMicroseconds: UInt64,
        serviceTimeMicroseconds: UInt64,
        retries: UInt32,
        requestBytes: UInt32,
        responseBytes: UInt32,
        tailLatencyMicroseconds: UInt64,
        budgetExceeded: Bool
    ) {
        self.queueWaitMicroseconds = queueWaitMicroseconds
        self.serviceTimeMicroseconds = serviceTimeMicroseconds
        self.retries = retries
        self.requestBytes = requestBytes
        self.responseBytes = responseBytes
        self.tailLatencyMicroseconds = tailLatencyMicroseconds
        self.budgetExceeded = budgetExceeded
    }

    fileprivate init(data: Data) throws {
        var reader = ByteReader(data)
        guard data.count == 80, try reader.readUInt16() == 1 else {
            throw GhostOSClientError.invalidFrame
        }
        let flags = try reader.readUInt16()
        self.init(
            queueWaitMicroseconds: try reader.readUInt64(),
            serviceTimeMicroseconds: try reader.readUInt64(),
            retries: try reader.readUInt32(),
            requestBytes: try reader.readUInt32(),
            responseBytes: try reader.readUInt32(),
            tailLatencyMicroseconds: try reader.readUInt64(),
            budgetExceeded: flags & 1 != 0
        )
    }
}

public actor GhostOSClient {
    public static let apiVersion = GhostOSABI.apiVersion
    private typealias Method = GhostOSRPCMethod

    private static let headerBytes = GhostOSABI.frameHeaderBytes
    private static let protocolVersion = GhostOSABI.protocolVersion
    private static let capabilityFlag = GhostOSABI.capabilityFlag

    private let transport: any GhostOSTransport
    private var authority: GhostOSCapability?
    private var nextRequestID: UInt64 = 1
    public private(set) var lastDiagnostics = GhostOSPerformanceDiagnostics.empty

    public init(
        transport: any GhostOSTransport,
        authority: GhostOSCapability? = nil
    ) {
        self.transport = transport
        self.authority = authority
    }

    public func setAuthority(_ authority: GhostOSCapability?) {
        self.authority = authority
    }

    public func clusterState() async throws -> ClusterState {
        let payload = try await call(method: .clusterState, body: Data())
        var reader = ByteReader(payload)
        let generation = try reader.readUInt64()
        let sampledAt = try reader.readUInt64()
        let nodeCount = Int(try reader.readUInt16())
        try reader.skip(6)
        guard nodeCount <= 64, reader.remaining == nodeCount * 32 else {
            throw GhostOSClientError.invalidFrame
        }
        var nodes: [ClusterNode] = []
        nodes.reserveCapacity(nodeCount)
        for _ in 0..<nodeCount {
            let nodeID = try reader.readUInt32()
            guard nodeID > 0,
                  let health = NodeHealth(rawValue: try reader.readByte()) else {
                throw GhostOSClientError.invalidFrame
            }
            try reader.skip(1)
            let cpuLoad = try reader.readUInt16()
            let memoryUsed = try reader.readUInt64()
            let memoryTotal = try reader.readUInt64()
            let runningJobs = try reader.readUInt32()
            let queuedJobs = try reader.readUInt32()
            guard cpuLoad <= 1_000, memoryUsed <= memoryTotal else {
                throw GhostOSClientError.invalidFrame
            }
            nodes.append(
                ClusterNode(
                    id: nodeID,
                    health: health,
                    cpuLoadPermille: cpuLoad,
                    memoryUsedBytes: memoryUsed,
                    memoryTotalBytes: memoryTotal,
                    runningJobs: runningJobs,
                    queuedJobs: queuedJobs
                )
            )
        }
        return ClusterState(
            generation: generation,
            sampledAtMicroseconds: sampledAt,
            nodes: nodes
        )
    }

    public func topologyState() async throws -> TopologyState {
        let payload = try await call(method: .topologyState, body: Data())
        var reader = ByteReader(payload)
        let generation = try reader.readUInt64()
        let sampledAt = try reader.readUInt64()
        let linkCount = Int(try reader.readUInt16())
        try reader.skip(6)
        guard linkCount <= 64, reader.remaining == linkCount * 32 else {
            throw GhostOSClientError.invalidFrame
        }
        var links: [TopologyLink] = []
        links.reserveCapacity(linkCount)
        for index in 0..<linkCount {
            let from = try reader.readUInt32()
            let to = try reader.readUInt32()
            guard from > 0,
                  to > 0,
                  let transport = TopologyTransport(rawValue: try reader.readByte()),
                  let route = TopologyRoute(rawValue: try reader.readByte()),
                  let reachability = TopologyReachability(rawValue: try reader.readByte()) else {
                throw GhostOSClientError.invalidFrame
            }
            try reader.skip(1)
            let mtu = try reader.readUInt16()
            try reader.skip(2)
            let latency = try reader.readUInt64()
            let bandwidth = try reader.readUInt64()
            guard mtu >= 576, latency > 0, bandwidth > 0 else {
                throw GhostOSClientError.invalidFrame
            }
            links.append(
                TopologyLink(
                    id: "\(from)-\(to)-\(index)",
                    from: from,
                    to: to,
                    transport: transport,
                    route: route,
                    reachability: reachability,
                    latencyMicroseconds: latency,
                    bandwidthMbps: bandwidth,
                    mtu: mtu
                )
            )
        }
        return TopologyState(
            generation: generation,
            sampledAtMicroseconds: sampledAt,
            links: links
        )
    }

    public func clusterSummary() async throws -> ClusterSummary {
        let payload = try await call(method: .clusterSummary, body: Data())
        guard payload.count == 128 else {
            throw GhostOSClientError.invalidFrame
        }
        var reader = ByteReader(payload)
        let id = try reader.readUInt128()
        let lifecycle = try reader.readByte()
        let health = try reader.readByte()
        try reader.skip(2)
        let generation = try reader.readUInt64()
        let sampledAt = try reader.readUInt64()
        let leader = try reader.readUInt32()
        let coordinator = try reader.readUInt32()
        let memberCount = try reader.readUInt16()
        let healthyMembers = try reader.readUInt16()
        let votingMembers = try reader.readUInt16()
        let quorumRequired = try reader.readUInt16()
        let quorumAvailable = try reader.readUInt16()
        try reader.skip(2)
        let nameBytes = try reader.readData(count: 64)
        let name = String(data: nameBytes.prefix(while: { $0 != 0 }), encoding: .utf8)
        guard let lifecycle = ClusterLifecycle(rawValue: lifecycle),
              let health = ClusterHealth(rawValue: health),
              leader > 0,
              coordinator > 0,
              let name else {
            throw GhostOSClientError.invalidFrame
        }
        return ClusterSummary(
            id: id,
            name: name,
            lifecycle: lifecycle,
            health: health,
            generation: generation,
            sampledAtMicroseconds: sampledAt,
            leader: leader,
            coordinator: coordinator,
            memberCount: memberCount,
            healthyMembers: healthyMembers,
            votingMembers: votingMembers,
            quorumRequired: quorumRequired,
            quorumAvailable: quorumAvailable
        )
    }

    public func clusterHealth() async throws -> ClusterHealthSnapshot {
        let payload = try await call(method: .clusterHealth, body: Data())
        guard payload.count == 48 else {
            throw GhostOSClientError.invalidFrame
        }
        var reader = ByteReader(payload)
        let generation = try reader.readUInt64()
        let health = try reader.readByte()
        let quorum = try reader.readByte() != 0
        try reader.skip(6)
        let heartbeat = try reader.readUInt64()
        let missed = try reader.readUInt16()
        try reader.skip(2)
        let lastChange = try reader.readUInt64()
        let healthy = try reader.readUInt16()
        let degraded = try reader.readUInt16()
        let failed = try reader.readUInt16()
        guard let health = ClusterHealth(rawValue: health) else {
            throw GhostOSClientError.invalidFrame
        }
        return ClusterHealthSnapshot(
            generation: generation,
            health: health,
            quorum: quorum,
            heartbeatPeriodMicroseconds: heartbeat,
            missedHeartbeatLimit: missed,
            lastChangeMicroseconds: lastChange,
            healthyNodes: healthy,
            degradedNodes: degraded,
            failedNodes: failed
        )
    }

    public func clusterResources() async throws -> ClusterResources {
        let payload = try await call(method: .clusterResources, body: Data())
        guard payload.count == 96 else {
            throw GhostOSClientError.invalidFrame
        }
        var reader = ByteReader(payload)
        return ClusterResources(
            generation: try reader.readUInt64(),
            cpuCapacity: try reader.readUInt64(),
            cpuAvailable: try reader.readUInt64(),
            memoryCapacityBytes: try reader.readUInt64(),
            memoryAvailableBytes: try reader.readUInt64(),
            cxlCapacityBytes: try reader.readUInt64(),
            cxlAvailableBytes: try reader.readUInt64(),
            storageCapacityBytes: try reader.readUInt64(),
            storageAvailableBytes: try reader.readUInt64(),
            networkBandwidthMbps: try reader.readUInt64(),
            acceleratorCapacity: try reader.readUInt64(),
            acceleratorAvailable: try reader.readUInt64()
        )
    }

    public func clusterInvitations() async throws -> [ClusterInvitation] {
        let payload = try await call(method: .clusterInvitations, body: Data())
        var reader = ByteReader(payload)
        guard payload.count >= 24 else {
            throw GhostOSClientError.invalidFrame
        }
        _ = try reader.readUInt64()
        try reader.skip(8)
        let count = Int(try reader.readUInt16())
        try reader.skip(6)
        guard count <= 32, reader.remaining == count * 32 else {
            throw GhostOSClientError.invalidFrame
        }
        var invitations: [ClusterInvitation] = []
        for _ in 0..<count {
            let id = try reader.readUInt64()
            let node = try reader.readUInt32()
            let state = try reader.readByte()
            try reader.skip(3)
            _ = try reader.readUInt64()
            let expiresAt = try reader.readUInt64()
            _ = try reader.readUInt32()
            guard id > 0, node > 0 else {
                throw GhostOSClientError.invalidFrame
            }
            invitations.append(ClusterInvitation(id: id, node: node, state: state, expiresAtMicroseconds: expiresAt))
        }
        return invitations
    }

    public func clusterAudit() async throws -> [ClusterAuditEvent] {
        let payload = try await call(method: .clusterAudit, body: Data())
        var reader = ByteReader(payload)
        guard payload.count >= 24 else {
            throw GhostOSClientError.invalidFrame
        }
        _ = try reader.readUInt64()
        try reader.skip(8)
        let count = Int(try reader.readUInt16())
        try reader.skip(6)
        guard count <= 32, reader.remaining == count * 48 else {
            throw GhostOSClientError.invalidFrame
        }
        var events: [ClusterAuditEvent] = []
        for _ in 0..<count {
            let sequence = try reader.readUInt64()
            let timestamp = try reader.readUInt64()
            try reader.skip(16)
            let actor = try reader.readUInt32()
            let operation = try reader.readUInt16()
            let status = try reader.readUInt16()
            let target = try reader.readUInt64()
            guard sequence > 0, actor > 0 else {
                throw GhostOSClientError.invalidFrame
            }
            events.append(ClusterAuditEvent(id: sequence, timestampMicroseconds: timestamp, actor: actor, operation: operation, status: status, target: target))
        }
        return events
    }

    public func submitJob(
        command: String,
        priority: UInt8,
        timeoutMicroseconds: UInt64
    ) async throws -> JobReceipt {
        let commandBytes = Data(command.utf8)
        guard !commandBytes.isEmpty,
              commandBytes.count <= 512,
              priority > 0,
              timeoutMicroseconds > 0 else {
            throw GhostOSClientError.invalidInput
        }
        var writer = ByteWriter()
        writer.append(priority)
        writer.padding(7)
        writer.append(timeoutMicroseconds)
        writer.append(UInt16(commandBytes.count))
        writer.append(commandBytes)
        let payload = try await call(method: .submitJob, body: writer.data)
        var reader = ByteReader(payload)
        guard payload.count == 16 else {
            throw GhostOSClientError.invalidFrame
        }
        let jobID = try reader.readUInt64()
        let acceptedAt = try reader.readUInt64()
        guard jobID > 0 else {
            throw GhostOSClientError.invalidFrame
        }
        return JobReceipt(
            jobID: jobID,
            acceptedAtMicroseconds: acceptedAt
        )
    }

    public func delegateCapability(
        _ delegation: CapabilityDelegation
    ) async throws -> GhostOSCapability {
        guard delegation.resource > 0,
              delegation.subjectNode > 0,
              !delegation.rights.isEmpty,
              !delegation.transports.isEmpty,
              delegation.validForMicroseconds > 0 else {
            throw GhostOSClientError.invalidInput
        }
        var writer = ByteWriter()
        writer.append(delegation.resource)
        writer.append(delegation.subjectNode)
        writer.append(delegation.rights.rawValue)
        writer.append(delegation.transports.rawValue)
        writer.padding(1)
        writer.append(delegation.validForMicroseconds)
        let payload = try await call(
            method: .delegateCapability,
            body: writer.data
        )
        return try GhostOSCapability(bytes: payload)
    }

    private func call(method: Method, body: Data) async throws -> Data {
        let requestID = nextRequestID
        nextRequestID = nextRequestID &+ 1
        if nextRequestID == 0 {
            nextRequestID = 1
        }
        var payload = Data()
        if let authority {
            payload.append(authority.bytes)
        }
        payload.append(body)
        var request = ByteWriter()
        request.append(GhostOSABI.magic)
        request.append(Self.protocolVersion)
        request.append(method.rawValue)
        request.append(authority == nil ? 0 : Self.capabilityFlag)
        request.append(requestID)
        request.append(UInt32(payload.count))
        request.append(UInt16(0))
        request.padding(2)
        request.append(payload)
        let response = try await transport.roundTrip(request.data)
        return try decodeResponse(
            response,
            method: method,
            requestID: requestID
        )
    }

    private func decodeResponse(
        _ response: Data,
        method: Method,
        requestID: UInt64
    ) throws -> Data {
        guard response.count >= Self.headerBytes else {
            throw GhostOSClientError.invalidFrame
        }
        var reader = ByteReader(response)
        guard try reader.readData(count: 4) == GhostOSABI.magic else {
            throw GhostOSClientError.invalidFrame
        }
        let responseVersion = UInt16(try reader.readByte())
        guard responseVersion == UInt16(Self.protocolVersion) else {
            try GhostOSCompatibility.validate(responseVersion)
            throw GhostOSClientError.abiMismatch
        }
        guard try reader.readByte() == method.rawValue,
              try reader.readUInt16() == 0,
              try reader.readUInt64() == requestID else {
            throw GhostOSClientError.mismatchedResponse
        }
        let payloadBytes = Int(try reader.readUInt32())
        let status = try reader.readUInt16()
        try reader.skip(2)
        guard payloadBytes == reader.remaining else {
            throw GhostOSClientError.invalidFrame
        }
        let payload = try reader.readData(count: payloadBytes)
        guard status == 0 else {
            if payload.count == 80 {
                lastDiagnostics = try GhostOSPerformanceDiagnostics(data: payload)
            }
            throw GhostOSClientError.operatorFailure(
                GhostOSOperatorFailure.rpcStatus(status, requestID: requestID)
            )
        }
        guard payload.count >= 80 else {
            throw GhostOSClientError.invalidFrame
        }
        lastDiagnostics = try GhostOSPerformanceDiagnostics(data: Data(payload.suffix(80)))
        return Data(payload.dropLast(80))
    }
}
