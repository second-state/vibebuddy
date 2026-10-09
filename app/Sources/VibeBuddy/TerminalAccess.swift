import AppKit
import CoreServices

/// Automation access to the terminals whose tabs K2 can go to: the daemon drives them with AppleScript, and macOS
/// asks about that on VibeBuddy's behalf. Asking from the Agents tab first keeps the prompt from popping up the
/// first time the user presses K2, on the box, away from the Mac.
enum TerminalAccess {
    enum State {
        case allowed
        case notAsked
        case denied
        /// macOS only answers while the terminal is running.
        case unknown
    }

    struct Terminal: Identifiable {
        let bundleID: String
        let name: String
        var id: String { bundleID }

        var url: URL? { NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) }
        var running: Bool { !NSRunningApplication.runningApplications(withBundleIdentifier: bundleID).isEmpty }
    }

    /// The terminals the daemon knows how to reach a tab in; keep in step with `focus_terminal_tab`.
    static let supported = [
        Terminal(bundleID: "com.mitchellh.ghostty", name: "Ghostty"),
        Terminal(bundleID: "com.googlecode.iterm2", name: "iTerm2"),
        Terminal(bundleID: "com.apple.Terminal", name: "Terminal"),
    ]

    static var installed: [Terminal] { supported.filter { $0.url != nil } }

    /// With `ask`, shows the system prompt if the user hasn't answered yet and blocks until they do, so call it
    /// off the main thread.
    static func check(_ terminal: Terminal, ask: Bool) -> State {
        let target = NSAppleEventDescriptor(bundleIdentifier: terminal.bundleID)
        guard let desc = target.aeDesc else { return .unknown }
        switch AEDeterminePermissionToAutomateTarget(desc, typeWildCard, typeWildCard, ask) {
        case noErr: return .allowed
        case OSStatus(errAEEventWouldRequireUserConsent): return .notAsked
        case OSStatus(errAEEventNotPermitted): return .denied
        default: return .unknown
        }
    }
}
