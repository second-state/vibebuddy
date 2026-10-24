#if DEBUG
import AppKit
import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// Debug builds only: every korekore page, drawn in the real window off screen with a sample status, into PNGs to set
/// beside the design canvas. `VIBEBUDDY_KOREKORE_SNAPSHOTS=<folder> swift run --package-path app VibeBuddy`. No
/// daemon is started, and the conversation is canned, so it can run beside the installed app without showing what the
/// user said. (The Agents page still reads this Mac's hook configs and terminal permissions, as it always does.)
@MainActor
enum KorekoreSnapshots {
    static let sample = #"""
    {"daemon":{"build":"0.5.0 cc16829","revision":"firmware-v0.5.0-61-gcc16829","app_version":"0.5.0"},
     "device":{"connected":true,"port":"/dev/cu.usbmodem8301","bridge":false,"mode":"duty","firmware_version":"0.5.0",
       "firmware_build":"cc16829","voice":"mei","volume":50,"box_key":"k","computer_key":"me","paired":true,
       "paired_computers":[{"key":"me","name":"Michael's Mac Studio"},{"key":"t","name":"ThinkPad · Omarchy"}],
       "wifi_network":"ByteTrade","wifi_address":"192.168.1.160","transfer":true},
     "today":{"done":7,"asks":3,"busy_seconds":15120},
     "hooks":{"codex":"2026-10-21T16:40:00+08:00","claude":"2026-10-21T17:03:00+08:00"},
     "config":{"notify_link":true,"check_updates":true,"relay":false},
     "updates":{"enabled":true,"last_check":"2026-10-21T13:51:00+08:00","unsupported_app":false},
     "activities":[{"agent":"claude-code","title":"CC:Add Chinese to the box","state":"working"},
                   {"agent":"codex","title":"CX:Fix voice reply","state":"working"},
                   {"agent":"github-actions","title":"CI:App tests","state":"working"}]}
    """#

    /// The canvas's sample exchanges; the daemon's history is never read here, since it is the user's own.
    static let conversation = [
        ResidentExchange(said: "Quick check: what time is it, and how many agents are working?",
                         replies: ["It's 10:09 a.m. in Singapore. Three agents are working."]),
        ResidentExchange(said: "新加坡明天天气怎么样", replies: ["明天上午多云转晴，最高约三十四度；午后可能有雷阵雨，出门记得带伞。"]),
        ResidentExchange(said: "我现在那几个 session 工作怎么样",
                         replies: ["两个：XFactor 的 GitHub Actions 正在跑，上次是失败；Qing 的 Claude Code 在等你回复。"]),
    ]

    /// Draws a window off screen, system buttons included, and saves it.
    private static func capture(_ window: NSWindow, as name: String, in folder: URL) {
        window.setFrameOrigin(NSPoint(x: -20_000, y: 0))
        window.orderFrontRegardless()
        RunLoop.main.run(until: Date().addingTimeInterval(1.5))
        let frame = window.contentView!.superview!
        let bitmap = frame.bitmapImageRepForCachingDisplay(in: frame.bounds)!
        frame.cacheDisplay(in: frame.bounds, to: bitmap)
        try? bitmap.representation(using: .png, properties: [:])!.write(to: folder.appendingPathComponent("\(name).png"))
        window.close()
    }

    /// The windows as they are with the running daemon's status, a few seconds after it first answers. The
    /// Conversation page is left out (it is the user's own), and nothing is sent to the box.
    static func live(to folder: URL, model: AppModel) {
        NSApplication.shared.setActivationPolicy(.accessory)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        Task { @MainActor in
            for _ in 0..<40 where model.status == nil { try? await Task.sleep(for: .milliseconds(250)) }
            try? await Task.sleep(for: .seconds(2))
            for section in KorekoreSection.allCases where section != .conversation {
                capture(KorekoreSettingsWindow(rootView: KorekoreSettingsRoot(model: model, initial: section)), as: "live-\(section.rawValue)", in: folder)
            }
            let panel = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 600), styleMask: [.borderless], backing: .buffered, defer: false)
            let actions = KorekoreMenuActions(settings: {}, checkForUpdates: {}, reportProblem: {}, quit: {}, restartDaemon: {})
            let hosting = NSHostingView(rootView: KorekorePanelRoot(model: model, actions: actions).padding(20).background(Color(korekoreRGB: 0xC9C3D8)))
            panel.setContentSize(hosting.fittingSize)
            panel.contentView = hosting
            capture(panel, as: "live-menu-panel", in: folder)
            let icon = KorekoreMenuIcon.image(for: BoxPicture.derive(status: model.status, daemonAlive: model.daemonAlive))
            if let tiff = icon.tiffRepresentation, let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) {
                try? png.write(to: folder.appendingPathComponent("live-menu-icon.png"))
            }
            for step in 0...6 {
                capture(KorekoreSettingsWindow.onboarding(rootView: KorekoreOnboarding(model: model, finish: {}, step: step, passive: true)),
                        as: "live-onboarding-\(step)", in: folder)
            }
            exit(0)
        }
    }

    static func write(to folder: URL, model: AppModel) {
        NSApplication.shared.setActivationPolicy(.prohibited)
        KorekoreAssets.registerFonts(in: URL(fileURLWithPath: "fonts/korekore"))
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let status = try! StatusCoding.decoder().decode(Status.self, from: Data(sample.utf8))
        model.showSample(status)
        for section in KorekoreSection.allCases {
            let window = KorekoreSettingsWindow(rootView: KorekoreSettingsRoot(model: model, initial: section, sampleConversation: conversation))
            window.setFrameOrigin(NSPoint(x: -20_000, y: 0))
            window.orderFrontRegardless()
            RunLoop.main.run(until: Date().addingTimeInterval(1.5))
            let frame = window.contentView!.superview!
            let bitmap = frame.bitmapImageRepForCachingDisplay(in: frame.bounds)!
            frame.cacheDisplay(in: frame.bounds, to: bitmap)
            try? bitmap.representation(using: .png, properties: [:])!.write(to: folder.appendingPathComponent("page-\(section.rawValue).png"))
            window.close()
        }
        for step in 0...6 {
            capture(KorekoreSettingsWindow.onboarding(rootView: KorekoreOnboarding(model: model, finish: {}, step: step, passive: true)),
                    as: "onboarding-\(step)", in: folder)
        }
        let update = String(localized: "The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.")
        let alerts: [(String, AnyView)] = [
            ("alert-take-over", AnyView(KorekoreAlert.preview(
                title: String(localized: "Found an old vibebuddyd background service"),
                message: String(localized: "korekore can remove it and take over, so the two don't fight over the box."),
                buttons: [String(localized: "Remove and take over"), String(localized: "Later")]))),
            ("alert-language", AnyView(KorekoreAlert.preview(
                title: String(localized: "Restart korekore to change the language?"),
                message: String(localized: "The box goes offline for a few seconds while the app restarts."),
                buttons: [String(localized: "Restart"), String(localized: "Not now")],
                checkbox: .init(label: String(localized: "Also switch the box's voice to \("Xiaohe")"), isOn: true)))),
            ("alert-flash-file", AnyView(KorekoreAlert.preview(
                title: String(localized: "Flash this firmware?"), message: update,
                buttons: [String(localized: "Flash"), String(localized: "Cancel")],
                facts: [(String(localized: "Firmware package"), "0.5.1"), (String(localized: "Box now"), "0.5.0")]))),
            ("alert-update", AnyView(KorekoreAlert.preview(
                title: String(localized: "Update the box firmware to \("0.5.1")?"), message: update,
                buttons: [String(localized: "Update"), String(localized: "Not now")]))),
            ("alert-foreign", AnyView(KorekoreAlert.preview(
                title: String(localized: "Flash the box with korekore?"),
                message: String(localized: "The box's current firmware and data will be erased and can't be recovered. It restarts on its own when done; over the UART port this takes a few minutes."),
                buttons: [String(localized: "Flash"), String(localized: "Cancel")], warning: true))),
        ]
        for (name, view) in alerts {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 480, height: 300), styleMask: [.borderless], backing: .buffered, defer: false)
            let hosting = NSHostingView(rootView: view.padding(10).background(KorekoreTheme.Colors.backdrop))
            window.setContentSize(hosting.fittingSize)
            window.contentView = hosting
            capture(window, as: name, in: folder)
        }
    }
}
#endif
