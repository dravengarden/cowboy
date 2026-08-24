// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "CowboyInstaller",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "CowboyInstaller", targets: ["CowboyInstaller"]),
        .library(name: "CowboyInstallerCore", targets: ["CowboyInstallerCore"]),
    ],
    targets: [
        .target(name: "CowboyInstallerCore"),
        .executableTarget(
            name: "CowboyInstaller",
            dependencies: ["CowboyInstallerCore"]
        ),
        .testTarget(
            name: "CowboyInstallerCoreTests",
            dependencies: ["CowboyInstallerCore"]
        ),
    ]
)
