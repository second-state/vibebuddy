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
    }
}
#endif
