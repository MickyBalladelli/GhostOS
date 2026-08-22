import Foundation

public enum GhostOSCompatibility {
    /// Stable user-space SDK contract version. This is independent of the
    /// SYRP wire version and the Swift package's toolchain version.
    public static let sdkApiVersion: UInt16 = 1
    public static let currentVersion: UInt16 = sdkApiVersion
    public static let minimumVersion: UInt16 = 1

    public static func validate(_ offered: UInt16) throws {
        guard offered >= minimumVersion else {
            throw GhostOSClientError.incompatibleApiVersion(
                code: "GHOSTOS-COMPAT-001",
                offered: offered,
                minimum: minimumVersion,
                maximum: currentVersion
            )
        }
        guard offered <= currentVersion else {
            throw GhostOSClientError.incompatibleApiVersion(
                code: "GHOSTOS-COMPAT-002",
                offered: offered,
                minimum: minimumVersion,
                maximum: currentVersion
            )
        }
    }

    @available(*, deprecated, message: "Use GhostOSClient.apiVersion")
    public static let legacyVersion: UInt16 = 1
}
