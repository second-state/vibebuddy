import AppKit
import VibeBuddyCore

/// The UI language picked in Settings. macOS reads the app's own `AppleLanguages` default at launch,
/// so a change takes effect after a restart; "System" removes the override.
enum AppLanguage: String, CaseIterable, Identifiable {
    case system
    case en
    case zhHans = "zh-Hans"

    var id: String { rawValue }

    /// Where the voice asked for during a language change waits for the restarted app (see AppModel).
    static let pendingVoiceKey = "pendingVoice"

    static var current: AppLanguage {
        guard let bundleID = Bundle.main.bundleIdentifier,
              let own = UserDefaults.standard.persistentDomain(forName: bundleID)?["AppleLanguages"] as? [String],
              let first = own.first else { return .system }
        return AppLanguage(rawValue: first) ?? .system
    }

    func save() {
        if self == .system {
            UserDefaults.standard.removeObject(forKey: "AppleLanguages")
        } else {
            UserDefaults.standard.set([rawValue], forKey: "AppleLanguages")
        }
    }

    /// The language the UI will be shown in once this choice applies.
    var resolved: VoiceLanguage {
        switch self {
        case .en: return .en
        case .zhHans: return .zh
        case .system:
            // The system-wide list, not this process's, which already has the app's override in front.
            let system = CFPreferencesCopyValue("AppleLanguages" as CFString, kCFPreferencesAnyApplication,
                                                kCFPreferencesCurrentUser, kCFPreferencesAnyHost) as? [String] ?? []
            let match = Bundle.preferredLocalizations(from: Bundle.main.localizations, forPreferences: system).first ?? "en"
            return match.hasPrefix("zh") ? .zh : .en
        }
    }
}

enum AppRelaunch {
    /// Quit, and open this bundle again once this process is gone (a second copy started earlier would hand off to us and quit).
    @MainActor
    static func relaunch() {
        let waiter = Process()
        waiter.executableURL = URL(fileURLWithPath: "/bin/sh")
        waiter.arguments = ["-c", "while kill -0 \(getpid()) 2>/dev/null; do sleep 0.2; done; /usr/bin/open \"$0\"", Bundle.main.bundlePath]
        try? waiter.run()
        NSApp.terminate(nil)
    }
}
