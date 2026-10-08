// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "WispScience",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "WispScience", targets: ["WispScience"]),
        .library(name: "WispProjectBrowser", targets: ["WispProjectBrowser"]),
    ],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm.git", exact: "1.19.0"),
        .package(url: "https://github.com/mgriebling/SwiftMath.git", exact: "1.7.3"),
    ],
    targets: [
        .target(name: "WispProjectBrowser"),
        .target(name: "WispProjectBrowserUI", dependencies: ["WispProjectBrowser", .product(name: "SwiftTerm", package: "SwiftTerm"), .product(name: "SwiftMath", package: "SwiftMath")], resources: [.process("Resources")]),
        .executableTarget(name: "WispScience", dependencies: ["WispProjectBrowserUI"]),
        .testTarget(name: "WispProjectBrowserTests", dependencies: ["WispProjectBrowser"]),
        .testTarget(name: "WispProjectBrowserUITests", dependencies: ["WispProjectBrowserUI"]),
    ]
)
