import Foundation

public enum SynOSClientError: Error, Equatable, LocalizedError, Sendable {
    case invalidCapability
    case invalidFrame
    case invalidInput
    case mismatchedResponse
    case remoteStatus(UInt16)
    case transportRejected

    public var errorDescription: String? {
        switch self {
        case .invalidCapability:
            "Capability token is not a SynOS v1 token."
        case .invalidFrame:
            "Gateway returned an invalid RPC frame."
        case .invalidInput:
            "Request fields are not valid."
        case .mismatchedResponse:
            "Gateway response does not match the request."
        case let .remoteStatus(status):
            "Gateway rejected the request with status \(status)."
        case .transportRejected:
            "Gateway transport rejected the request."
        }
    }
}

public actor SynOSClient {
    private enum Method: UInt8 {
        case clusterState = 1
        case submitJob = 2
        case delegateCapability = 3
    }

    private static let headerBytes = 24
    private static let protocolVersion: UInt8 = 1
    private static let capabilityFlag: UInt16 = 1

    private let transport: any SynOSTransport
    private var authority: SynOSCapability?
    private var nextRequestID: UInt64 = 1

    public init(
        transport: any SynOSTransport,
        authority: SynOSCapability? = nil
    ) {
        self.transport = transport
        self.authority = authority
    }

    public func setAuthority(_ authority: SynOSCapability?) {
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
            throw SynOSClientError.invalidFrame
        }
        var nodes: [ClusterNode] = []
        nodes.reserveCapacity(nodeCount)
        for _ in 0..<nodeCount {
            let nodeID = try reader.readUInt32()
            guard nodeID > 0,
                  let health = NodeHealth(rawValue: try reader.readByte()) else {
                throw SynOSClientError.invalidFrame
            }
            try reader.skip(1)
            let cpuLoad = try reader.readUInt16()
            let memoryUsed = try reader.readUInt64()
            let memoryTotal = try reader.readUInt64()
            let runningJobs = try reader.readUInt32()
            let queuedJobs = try reader.readUInt32()
            guard cpuLoad <= 1_000, memoryUsed <= memoryTotal else {
                throw SynOSClientError.invalidFrame
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
            throw SynOSClientError.invalidInput
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
            throw SynOSClientError.invalidFrame
        }
        let jobID = try reader.readUInt64()
        let acceptedAt = try reader.readUInt64()
        guard jobID > 0 else {
            throw SynOSClientError.invalidFrame
        }
        return JobReceipt(
            jobID: jobID,
            acceptedAtMicroseconds: acceptedAt
        )
    }

    public func delegateCapability(
        _ delegation: CapabilityDelegation
    ) async throws -> SynOSCapability {
        guard delegation.resource > 0,
              delegation.subjectNode > 0,
              !delegation.rights.isEmpty,
              !delegation.transports.isEmpty,
              delegation.validForMicroseconds > 0 else {
            throw SynOSClientError.invalidInput
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
        return try SynOSCapability(bytes: payload)
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
        request.append(Data([0x53, 0x59, 0x52, 0x50]))
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
            throw SynOSClientError.invalidFrame
        }
        var reader = ByteReader(response)
        guard try reader.readData(count: 4) == Data([0x53, 0x59, 0x52, 0x50]),
              try reader.readByte() == Self.protocolVersion,
              try reader.readByte() == method.rawValue,
              try reader.readUInt16() == 0,
              try reader.readUInt64() == requestID else {
            throw SynOSClientError.mismatchedResponse
        }
        let payloadBytes = Int(try reader.readUInt32())
        let status = try reader.readUInt16()
        try reader.skip(2)
        guard payloadBytes == reader.remaining else {
            throw SynOSClientError.invalidFrame
        }
        guard status == 0 else {
            throw SynOSClientError.remoteStatus(status)
        }
        return try reader.readData(count: payloadBytes)
    }
}
