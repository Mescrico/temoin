// swift-tools-version:5.9
import PackageDescription

// Platform-independent core: age sealer, envelope, enrollment links.
// On Apple platforms it uses CryptoKit; on Linux (CI / Docker tests) swift-crypto,
// which exposes the same API.
let package = Package(
    name: "TemoinCore",
    platforms: [.iOS(.v16), .macOS(.v13)],
    products: [.library(name: "TemoinCore", targets: ["TemoinCore"])],
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", "3.0.0" ..< "4.0.0"),
    ],
    targets: [
        .target(
            name: "TemoinCore",
            dependencies: [.product(name: "Crypto", package: "swift-crypto", condition: .when(platforms: [.linux]))]
        ),
        .testTarget(name: "TemoinCoreTests", dependencies: ["TemoinCore"]),
    ]
)
