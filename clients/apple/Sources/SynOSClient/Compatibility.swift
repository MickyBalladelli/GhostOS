import Foundation

public enum SynOSCompatibility {
    public static let currentVersion: UInt16 = 1
    public static let minimumVersion: UInt16 = 1

    public static func validate(_ offered: UInt16) throws {
        guard offered >= minimumVersion else {
            throw SynOSClientError.incompatibleApiVersion(
                code: "SYNOS-COMPAT-001",
                offered: offered,
                minimum: minimumVersion,
                maximum: currentVersion
            )
        }
        guard offered <= currentVersion else {
            throw SynOSClientError.incompatibleApiVersion(
                code: "SYNOS-COMPAT-002",
                offered: offered,
                minimum: minimumVersion,
                maximum: currentVersion
            )
        }
    }

    @available(*, deprecated, message: "Use SynOSClient.apiVersion")
    public static let legacyVersion: UInt16 = 1
}
