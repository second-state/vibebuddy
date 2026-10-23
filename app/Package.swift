// swift-tools-version:5.9
// VibeBuddy's Mac app: built with SwiftPM, no Xcode project (this machine only has the command
// line tools; with Xcode installed, open this Package.swift directly). Bundling: scripts/build-app.sh.
import PackageDescription

let package = Package(
    name: "VibeBuddy",
    platforms: [.macOS(.v14)],
    dependencies: [
        // The App's self-update (ADR-0010): checks the appcast, verifies the EdDSA signature, swaps the bundle.
        .package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.10.0"),
    ],
    targets: [
        // Pure logic: status decoding, menu state derivation, hook config merging, voice pack parsing. No UI.
        .target(name: "VibeBuddyCore", path: "Sources/VibeBuddyCore"),
        // The korekore UI's views (ADR-0021), fed plain values so KorekorePreview can draw them without the app.
        .target(name: "KorekoreUI", dependencies: ["VibeBuddyCore"], path: "Sources/KorekoreUI"),
        .executableTarget(
            name: "VibeBuddy",
            dependencies: ["VibeBuddyCore", "KorekoreUI", .product(name: "Sparkle", package: "Sparkle")],
            path: "Sources/VibeBuddy",
            // build-app.sh puts Sparkle.framework in Contents/Frameworks.
            linkerSettings: [.unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"])]
        ),
        // The command line tools have neither XCTest nor swift-testing; the view-model seams are
        // guarded by this self-test executable: `swift run SelfTest`.
        .executableTarget(name: "SelfTest", dependencies: ["VibeBuddyCore"], path: "Sources/SelfTest"),
        // Draws the korekore views with sample data into PNGs, to set beside the design canvas:
        // `swift run --package-path app KorekorePreview <dir>`.
        .executableTarget(name: "KorekorePreview", dependencies: ["KorekoreUI"], path: "Sources/KorekorePreview"),
    ]
)
