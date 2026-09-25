import Foundation
import VibeBuddyCore

/// Where things live in the app bundle: the two helpers, the three firmware images, the voice packs.
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

    static var macOSDirectory: URL {
        Bundle.main.executableURL!.deletingLastPathComponent()
    }

    static var daemonBinary: URL { macOSDirectory.appendingPathComponent("vibebuddyd") }
    static var hookBinary: URL { macOSDirectory.appendingPathComponent("vibebuddy-hook") }

    static var resourcesDirectory: URL {
        Bundle.main.resourceURL ?? macOSDirectory
    }

    static var firmwareDirectory: URL { resourcesDirectory.appendingPathComponent("firmware") }

    /// Build ID of the bundled firmware (written by the bundling script); nil when no firmware is bundled.
    static var bundledFirmwareBuild: String? {
        let url = firmwareDirectory.appendingPathComponent("build.txt")
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    static var firmwareFiles: (bootloader: URL, partitionTable: URL, app: URL)? {
        let bootloader = firmwareDirectory.appendingPathComponent("bootloader.bin")
        let table = firmwareDirectory.appendingPathComponent("partition-table.bin")
        let app = firmwareDirectory.appendingPathComponent("vibebuddy-fw.bin")
        let manager = FileManager.default
        guard [bootloader, table, app].allSatisfy({ manager.fileExists(atPath: $0.path) }) else { return nil }
        return (bootloader, table, app)
    }

    static func voicePackURL(_ id: String) -> URL {
        resourcesDirectory.appendingPathComponent("voices").appendingPathComponent("\(id).bin")
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
