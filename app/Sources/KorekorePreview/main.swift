// Draws the korekore views with the canvas's sample data into PNGs (2×), to set beside the design's boards.
// `swift run --package-path app KorekorePreview <output dir> [fonts dir]`; the fonts default to fonts/korekore.
import AppKit
import KorekoreUI
import SwiftUI
import VibeBuddyCore

let arguments = CommandLine.arguments
guard arguments.count >= 2 else {
    print("usage: KorekorePreview <output dir> [fonts dir]")
    exit(2)
}
let output = URL(fileURLWithPath: arguments[1])
KorekoreAssets.registerFonts(in: URL(fileURLWithPath: arguments.count > 2 ? arguments[2] : "fonts/korekore"))
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)

func picture(_ activities: [ActivitySummary], connected: Bool = true, mode: String = "duty") -> BoxPicture {
    let json = """
    {"daemon":{"build":"0.5.0"},"device":{"connected":\(connected),"bridge":false,"mode":"\(mode)","firmware_version":"0.5.0"},
     "today":{"done":7,"asks":3,"busy_seconds":15120},"hooks":{},"config":{"notify_link":true}}
    """
    var status = try! StatusCoding.decoder().decode(Status.self, from: Data(json.utf8))
    status.activities = activities
    return BoxPicture.derive(status: status, daemonAlive: true)
}

// The canvas's sidebar sample: three cards, all working.
let working = picture([
    ActivitySummary(agent: "claude-code", title: "CC:Add Chinese to the box", project: "vibebuddy", state: "working"),
    ActivitySummary(agent: "codex", title: "CX:Fix voice reply", project: "vibebuddy", state: "working"),
    ActivitySummary(agent: "github-actions", title: "CI:App tests", project: "vibebuddy", state: "working"),
])
let asking = picture([
    ActivitySummary(agent: "codex", title: "CX:Review changes", project: "vibebuddy", state: "needs_input"),
    ActivitySummary(agent: "claude-code", title: "CC:Add Chinese to the box", project: "vibebuddy", state: "working"),
])
let quiet = picture([])
let unlinked = picture([], connected: false)
let versions = KorekoreVersions(app: "0.5.0", firmware: "0.5.0")
let noActions = KorekoreMenuActions(settings: {}, checkForUpdates: {}, reportProblem: {}, quit: {}, restartDaemon: {})

@MainActor func save<V: View>(_ name: String, _ view: V) {
    let renderer = ImageRenderer(content: view.environment(\.colorScheme, .light))
    renderer.scale = 2
    guard let image = renderer.cgImage else {
        print("could not render \(name)")
        return
    }
    let data = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])!
    try! data.write(to: output.appendingPathComponent("\(name).png"))
    print("wrote \(name).png")
}

/// A board as the canvas frames it: 960 × 820 on the mockup backdrop, the window inset 24.
@MainActor func window(_ picture: BoxPicture, _ section: KorekoreSection) -> some View {
    KorekoreSettings(picture: picture, versions: versions, sections: KorekoreSection.allCases,
                     selection: .constant(section)) { _ in KorekorePagePending() }
        .padding(24)
        .frame(width: 960, height: 820)
        .background(KorekoreTheme.Colors.backdrop)
}

MainActor.assumeIsolated {
    save("window-character", window(working, .character))
    save("window-device-asking", window(asking, .device))
    for (name, sample) in [("working", working), ("asking", asking), ("quiet", quiet), ("no-link", unlinked)] {
        save("minibox-\(name)", MiniBox(picture: sample).padding(12).background(KorekoreTheme.Colors.paper))
    }
    save("menu-panel", KorekoreMenuPanel(picture: working, actions: noActions)
        .padding(24).background(Color(korekoreRGB: 0xC9C3D8)))
    save("menu-panel-no-link", KorekoreMenuPanel(picture: unlinked, actions: noActions)
        .padding(24).background(Color(korekoreRGB: 0xC9C3D8)))
    save("menu-icons", HStack(spacing: 16) {
        ForEach(Array([working, asking, unlinked].enumerated()), id: \.offset) { _, sample in
            Image(nsImage: KorekoreMenuIcon.image(for: sample)).interpolation(.none)
        }
    }.padding(8).background(Color.white.opacity(0.75)))
}

// The real window, system buttons included, drawn off screen: checks the frame and where the buttons land.
MainActor.assumeIsolated {
    NSApplication.shared.setActivationPolicy(.prohibited)
    let window = KorekoreSettingsWindow(rootView: KorekoreSettings(
        picture: working, versions: versions, sections: KorekoreSection.allCases, selection: .constant(.character)
    ) { _ in KorekorePagePending() })
    window.setFrameOrigin(NSPoint(x: -20_000, y: 0))
    window.orderFrontRegardless()
    RunLoop.main.run(until: Date().addingTimeInterval(1))
    let frame = window.contentView!.superview!
    let bitmap = frame.bitmapImageRepForCachingDisplay(in: frame.bounds)!
    frame.cacheDisplay(in: frame.bounds, to: bitmap)
    try! bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent("window-real.png"))
    print("wrote window-real.png")
    window.close()
}
