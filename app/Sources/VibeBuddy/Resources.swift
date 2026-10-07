import Foundation
import VibeBuddyCore

/// Where things live in the app bundle: the two helpers and the voice packs. Firmware isn't bundled; the daemon
/// downloads it (ADR-0010).
enum Resources {
    /// Semantic version, reported to the device with the heartbeat.
    static var bundleVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
    }

    /// Build number (git description), shown in the UI only.
    static var bundleBuild: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "dev"
    }

    static var displayVersion: String { "\(bundleVersion) (\(bundleBuild))" }

    /// Where every release lives, for downloading firmware by hand when the daemon can't.
    static let releasesPage = URL(string: "https://github.com/second-state/vibebuddy/releases")!

    static var macOSDirectory: URL {
        Bundle.main.executableURL!.deletingLastPathComponent()
    }

    static var daemonBinary: URL { macOSDirectory.appendingPathComponent("vibebuddyd") }
    static var hookBinary: URL { macOSDirectory.appendingPathComponent("vibebuddy-hook") }

    static var resourcesDirectory: URL {
        Bundle.main.resourceURL ?? macOSDirectory
    }

    static func voicePackURL(_ id: String) -> URL {
        resourcesDirectory.appendingPathComponent("voices").appendingPathComponent("\(id).bin")
    }

    /// The lines of Character `id` said with a form of address (characters/<id>/address/<form>.bin).
    static func addressPack(_ id: String, form: String) -> VoicePack? {
        let url = resourcesDirectory.appendingPathComponent("voices").appendingPathComponent("\(id).\(form).bin")
        return (try? Data(contentsOf: url)).flatMap(VoicePack.init(data:))
    }

    static func voicePack(_ id: String) -> VoicePack? {
        guard let data = try? Data(contentsOf: voicePackURL(id)) else { return nil }
        return VoicePack(data: data)
    }

    /// The language the UI is actually shown in ("zh" or "en"), which follows the
    /// system's preferred languages against the lproj folders the bundle ships.
    static let uiLanguage: VoiceLanguage =
        (Bundle.main.preferredLocalizations.first ?? "en").hasPrefix("zh") ? .zh : .en

    /// Catalog voices whose pack is in this build, UI-language voices first. The bundle
    /// can't change while the app runs, so this is worked out once (a file-existence
    /// check per voice) rather than on every SwiftUI render.
    static let bundledVoices: [VoiceCatalogEntry] = {
        let bundled = VoiceCatalogEntry.all.filter { FileManager.default.fileExists(atPath: voicePackURL($0.id).path) }
        return VoiceCatalogEntry.sorted(bundled, preferring: uiLanguage)
    }()

    /// Fixed location under Application Support: the hook binary, config and logs all live nearby.
    static var applicationSupport: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/VibeBuddy")
    }

    static var installedHookBinary: URL { applicationSupport.appendingPathComponent("bin/vibebuddy-hook") }
    static var logsDirectory: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/VibeBuddy")
    }

    /// Directories from before the rename (the AgentBeacon era). First launch moves them to the new location.
    static var legacyDirectories: [(from: URL, to: URL)] {
        let home = FileManager.default.homeDirectoryForCurrentUser
        return [
            (home.appendingPathComponent("Library/Application Support/AgentBeacon"), applicationSupport),
            (home.appendingPathComponent("Library/Logs/AgentBeacon"), logsDirectory),
        ]
    }

    static func migrateLegacyDirectories() {
        let manager = FileManager.default
        for (from, to) in legacyDirectories where manager.fileExists(atPath: from.path) && !manager.fileExists(atPath: to.path) {
            try? manager.moveItem(at: from, to: to)
        }
    }
}
