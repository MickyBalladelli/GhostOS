// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "SynOSControl",
    platforms: [
        .iOS(.v17),
        .macOS(.v14)
    ],
    products: [
        .library(name: "SynOSClient", targets: ["SynOSClient"]),
        .library(name: "SynOSControlUI", targets: ["SynOSControlUI"]),
        .executable(name: "SynOSControl", targets: ["SynOSControlApp"])
    ],
    targets: [
        .target(name: "SynOSClient"),
        .target(
            name: "SynOSControlUI",
            dependencies: ["SynOSClient"]
        ),
        .executableTarget(
            name: "SynOSControlApp",
            dependencies: ["SynOSClient", "SynOSControlUI"]
        )
    ]
)
