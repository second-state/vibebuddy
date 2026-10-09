import AppKit
import VibeBuddyCore

/// The UI language picked in Settings. macOS reads the app's own `AppleLanguages` default at launch,
/// so a change takes effect after a restart; "System" removes the override.
enum AppLanguage: String, CaseIterable, Identifiable {
    case system
    case en
    case zhHans = "zh-Hans"

    var id: String { rawValue }

    private static let key = "AppleLanguages"

    static var current: AppLanguage {
        guard let bundleID = Bundle.main.bundleIdentifier,
              let own = UserDefaults.standard.persistentDomain(forName: bundleID)?[key] as? [String],
              let first = own.first else { return .system }
        return AppLanguage(rawValue: first) ?? .system
    }

    func save() {
        if self == .system {
            UserDefaults.standard.removeObject(forKey: Self.key)
        } else {
            UserDefaults.standard.set([rawValue], forKey: Self.key)
        }
    }

    /// The language the UI will be shown in once this choice applies.
    var resolved: VoiceLanguage {
        switch self {
        case .en: return .en
        case .zhHans: return .zh
        case .system:
            // The system-wide list, not this process's, which already has the app's override in front.
            let system = CFPreferencesCopyValue(Self.key as CFString, kCFPreferencesAnyApplication,
                                                kCFPreferencesCurrentUser, kCFPreferencesAnyHost) as? [String] ?? []
            let match = Bundle.preferredLocalizations(from: Bundle.main.localizations, forPreferences: system).first ?? "en"
            return match.hasPrefix("zh") ? .zh : .en
        }
    }
}

enum AppRelaunch {
    /// Passed to the relaunched app so it opens Settings again, where the user left it.
    static let showSettingsArgument = "--show-settings"

    /// Quit, and open this bundle again once this process is gone (a second copy started earlier would hand off to us and quit).
    @MainActor
    static func relaunch() {
        openOnceGone(Bundle.main.bundlePath, arguments: [showSettingsArgument])
        NSApp.terminate(nil)
    }

    /// Opens the bundle at `path` once this process has exited; the caller then quits.
    static func openOnceGone(_ path: String, arguments: [String] = []) {
        let waiter = Process()
        waiter.executableURL = URL(fileURLWithPath: "/bin/sh")
        let args = arguments.isEmpty ? "" : " --args " + arguments.joined(separator: " ")
        waiter.arguments = ["-c", "while kill -0 \(getpid()) 2>/dev/null; do sleep 0.2; done; /usr/bin/open \"$0\"\(args)", path]
        try? waiter.run()
    }
}
