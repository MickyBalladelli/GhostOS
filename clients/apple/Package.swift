// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "GhostOSControl",
    platforms: [
        .iOS(.v17),
        .macOS(.v14)
    ],
    products: [
        .library(name: "GhostOSClient", targets: ["GhostOSClient"]),
        .library(name: "GhostOSControlUI", targets: ["GhostOSControlUI"]),
        .executable(name: "GhostOSControl", targets: ["GhostOSControlApp"])
    ],
    targets: [
        .target(name: "GhostOSClient"),
        .target(
            name: "GhostOSControlUI",
            dependencies: ["GhostOSClient"]
        ),
        .executableTarget(
            name: "GhostOSControlApp",
            dependencies: ["GhostOSClient", "GhostOSControlUI"]
        )
    ]
)
