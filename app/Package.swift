// swift-tools-version:5.9
// Vibe Buddy 的 Mac 端 App：SwiftPM 构建，不依赖 Xcode 工程（本机只有命令行
// 工具；装了 Xcode 可直接打开这个 Package.swift）。装包见 scripts/build-app.sh。
import PackageDescription

let package = Package(
    name: "VibeBuddy",
    platforms: [.macOS(.v14)],
    targets: [
        // 纯逻辑：状态解码、菜单状态推导、Hook 配置合并、语音包解析。不碰 UI。
        .target(name: "VibeBuddyCore", path: "Sources/VibeBuddyCore"),
        .executableTarget(name: "VibeBuddy", dependencies: ["VibeBuddyCore"], path: "Sources/VibeBuddy"),
        // 命令行工具没有 XCTest，也没有 swift-testing；视图模型的缝用这个
        // 自检可执行目标守着：`swift run SelfTest`。
        .executableTarget(name: "SelfTest", dependencies: ["VibeBuddyCore"], path: "Sources/SelfTest"),
    ]
)
