import AppKit
import CoreServices

/// Automation access to Ghostty, which K2 needs to go to a session's own tab: the daemon drives Ghostty with
/// AppleScript, and macOS asks about that on Vibe Buddy's behalf. Asking from the Agents tab first keeps the
/// prompt from popping up the first time the user presses K2, on the box, away from the Mac.
enum TerminalAccess {
    enum State {
        case allowed
        case notAsked
        case denied
        /// macOS only answers while Ghostty is running.
        case unknown
    }

    static let ghosttyBundleID = "com.mitchellh.ghostty"

    static var ghosttyURL: URL? { NSWorkspace.shared.urlForApplication(withBundleIdentifier: ghosttyBundleID) }

    static var ghosttyRunning: Bool { !NSRunningApplication.runningApplications(withBundleIdentifier: ghosttyBundleID).isEmpty }

    /// With `ask`, shows the system prompt if the user hasn't answered yet and blocks until they do, so call it
    /// off the main thread.
    static func check(ask: Bool) -> State {
        let target = NSAppleEventDescriptor(bundleIdentifier: ghosttyBundleID)
        guard let desc = target.aeDesc else { return .unknown }
        switch AEDeterminePermissionToAutomateTarget(desc, typeWildCard, typeWildCard, ask) {
        case noErr: return .allowed
        case OSStatus(errAEEventWouldRequireUserConsent): return .notAsked
        case OSStatus(errAEEventNotPermitted): return .denied
        default: return .unknown
        }
    }
}
