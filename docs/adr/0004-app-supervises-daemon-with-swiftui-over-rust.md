---
status: accepted
---

# App 看管 daemon：SwiftUI 界面加 Rust helper

Mac 端此前只有 LaunchAgent 常驻的 `vibebuddyd`，配置靠环境变量，接入靠手改 JSON。我们决定做一个原生 SwiftUI 菜单栏 App，把 `vibebuddyd` 作为 helper 打包在里面并由 App 拉起、重启、随 App 退出；LaunchAgent 退役。daemon 与 Hook 仍是 Rust，两者通过本机 HTTP 加 SSE 通信。

## 被拒绝的方案

保留 LaunchAgent、App 只是设置窗：两个生命周期，升级要换两处二进制，崩溃重启与登录启动各管各的。Tauri：Rust 复用多，但菜单栏、Login Item、设置窗外观都要绕，与"像系统自带的那样"相悖。

## 后果

仓库多一门 Swift。原定 Xcode 工程入库，2026-09-16 实施时改为 SwiftPM 包（`app/Package.swift`）加装包脚本：本机只有命令行工具，`xcodebuild` 不可用，而 `swift build` 能编 SwiftUI 与 AppKit；装了 Xcode 可直接打开 Package.swift，决定本身（原生 SwiftUI 加 Rust helper）不变。退出 App 盒子就离线，是有意为之。仓库名、bundle id `com.vibebuddy.app`、Application Support 路径沿用旧名。
