// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "s2timeline",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "s2timeline",
            swiftSettings: [.swiftLanguageMode(.v5)]
        )
    ]
)
