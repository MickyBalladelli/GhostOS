import Foundation

public protocol SynOSTransport: Sendable {
    func roundTrip(_ request: Data) async throws -> Data
}

public struct HTTPTransport: SynOSTransport {
    public let endpoint: URL

    public init(endpoint: URL) {
        self.endpoint = endpoint
    }

    public func roundTrip(_ request: Data) async throws -> Data {
        var urlRequest = URLRequest(url: endpoint)
        urlRequest.httpMethod = "POST"
        urlRequest.httpBody = request
        urlRequest.setValue(
            "application/vnd.synos.rpc",
            forHTTPHeaderField: "Content-Type"
        )
        let (data, response) = try await URLSession.shared.data(for: urlRequest)
        guard let httpResponse = response as? HTTPURLResponse,
              (200..<300).contains(httpResponse.statusCode) else {
            if let failure = try? JSONDecoder().decode(
                SynOSOperatorFailure.self,
                from: data
            ) {
                throw SynOSClientError.operatorFailure(failure)
            }
            throw SynOSClientError.transportRejected
        }
        return data
    }
}
