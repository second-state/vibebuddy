// swift-tools-version:5.9
// Vibe Buddy's Mac app: built with SwiftPM, no Xcode project (this machine only has the command
// line tools; with Xcode installed, open this Package.swift directly). Bundling: scripts/build-app.sh.
import PackageDescription

let package = Package(
    name: "VibeBuddy",
    platforms: [.macOS(.v14)],
    targets: [
        // Pure logic: status decoding, menu state derivation, hook config merging, voice pack parsing. No UI.
        .target(name: "VibeBuddyCore", path: "Sources/VibeBuddyCore"),
        .executableTarget(name: "VibeBuddy", dependencies: ["VibeBuddyCore"], path: "Sources/VibeBuddy"),
        // The command line tools have neither XCTest nor swift-testing; the view-model seams are
        // guarded by this self-test executable: `swift run SelfTest`.
        .executableTarget(name: "SelfTest", dependencies: ["VibeBuddyCore"], path: "Sources/SelfTest"),
    ]
)
