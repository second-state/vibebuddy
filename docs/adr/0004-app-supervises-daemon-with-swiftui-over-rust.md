---
status: accepted
---

# The App supervises the daemon: a SwiftUI interface over a Rust helper

Until now the Mac side had only `vibebuddyd` kept resident by a LaunchAgent, configured through environment variables and connected by hand-editing JSON. We decided to build a native SwiftUI menu bar App that bundles `vibebuddyd` as a helper, and that launches it, restarts it and quits it along with the App; the LaunchAgent is retired. The daemon and the hooks stay in Rust, and the two communicate over local HTTP plus SSE.

## Rejected options

Keep the LaunchAgent and make the App just a settings window: two lifecycles, upgrades replace binaries in two places, and crash restarts and login startup are each managed separately. Tauri: more Rust reuse, but the menu bar, Login Item and settings window appearance would all need workarounds, which runs against "like it came with the system".

## Consequences

The repository gains another language, Swift. The plan was to check in an Xcode project; during implementation on 2026-09-16 it became a SwiftPM package (`app/Package.swift`) plus a packaging script: this machine has only the command line tools, so `xcodebuild` isn't available, while `swift build` can compile SwiftUI and AppKit. With Xcode installed you can open Package.swift directly, and the decision itself (native SwiftUI plus a Rust helper) is unchanged. Quitting the App takes the box offline, by design. The repository name, the bundle id `com.vibebuddy.app` and the Application Support path keep their old names.
