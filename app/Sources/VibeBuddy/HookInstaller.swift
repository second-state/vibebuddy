import Foundation
import VibeBuddyCore

/// Hook setup: copies vibebuddy-hook to Application Support, then merges the hook entries into
/// the user-level config. Only our own entries are added or removed (VibeBuddyCore.HookConfig).
struct HookInstaller {
    struct Plan {
        let agent: HookAgent
        let configURL: URL
        let before: [String: Any]
        let after: [String: Any]
        var diff: [String] { HookConfig.describeChange(from: before, to: after, agent: agent) }
    }

    static func configURL(for agent: HookAgent) -> URL {
        let home = FileManager.default.homeDirectoryForCurrentUser
        switch agent {
        case .codex: return home.appendingPathComponent(".codex/hooks.json")
        case .claude: return home.appendingPathComponent(".claude/settings.json")
        }
    }

    /// Whether this agent is installed on this machine: checks for its user directory.
    static func isPresent(_ agent: HookAgent) -> Bool {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let directory = agent == .codex ? ".codex" : ".claude"
        return FileManager.default.fileExists(atPath: home.appendingPathComponent(directory).path)
    }

    /// When the config file was last written; compared with the last event time the daemon reports, it tells
    /// whether Codex is running this config (VibeBuddyCore.HookConfig.codexTrustHint).
    static func configModifiedAt(_ agent: HookAgent) -> Date? {
        (try? FileManager.default.attributesOfItem(atPath: configURL(for: agent).path))?[.modificationDate] as? Date
    }

    static func readConfig(_ url: URL) -> [String: Any] {
        guard let data = try? Data(contentsOf: url),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return object
    }

    static func isInstalled(_ agent: HookAgent) -> Bool {
        HookConfig.isInstalled(in: readConfig(configURL(for: agent)), agent: agent, binary: Resources.installedHookBinary.path)
    }

    /// Copies the bundled binary to its fixed location; done on every launch so the hook updates with the app.
    static func deployBinary() throws {
        let destination = Resources.installedHookBinary
        try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
        // The pre-rename binary came along with the old directory, and nothing references it any more.
        let legacy = destination.deletingLastPathComponent().appendingPathComponent("beacon-hook")
        if FileManager.default.fileExists(atPath: legacy.path) { try? FileManager.default.removeItem(at: legacy) }
        let source = Resources.hookBinary
        guard FileManager.default.fileExists(atPath: source.path) else { return }
        if FileManager.default.fileExists(atPath: destination.path) {
            try FileManager.default.removeItem(at: destination)
        }
        try FileManager.default.copyItem(at: source, to: destination)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: destination.path)
    }

    /// After the rename, entries with the old path can't run: we wrote them, so switch them to the new path without asking.
    /// But tell the caller which ones were rewritten: Codex disables any changed hook until someone re-trusts it,
    /// which the app can't do for them; without a heads-up the user just finds the box ignoring Codex.
    @discardableResult
    static func migrateLegacyCommands() -> [HookAgent] {
        var rewritten: [HookAgent] = []
        for agent in HookAgent.allCases {
            let url = configURL(for: agent)
            let before = readConfig(url)
            guard let hooks = before["hooks"] as? [String: Any] else { continue }
            let commands = hooks.values.flatMap { value in
                (value as? [[String: Any]] ?? []).flatMap { group in
                    (group["hooks"] as? [[String: Any]] ?? []).compactMap { $0["command"] as? String }
                }
            }
            let wanted = HookConfig.command(binary: Resources.installedHookBinary.path, agent: agent)
            let stale = commands.contains { HookConfig.isOurs($0) && $0 != wanted }
            guard stale else { continue }
            if (try? apply(installPlan(for: agent))) != nil { rewritten.append(agent) }
        }
        return rewritten
    }

    static func installPlan(for agent: HookAgent) -> Plan {
        let url = configURL(for: agent)
        let before = readConfig(url)
        let after = HookConfig.install(into: before, agent: agent, binary: Resources.installedHookBinary.path)
        return Plan(agent: agent, configURL: url, before: before, after: after)
    }

    static func removePlan(for agent: HookAgent) -> Plan {
        let url = configURL(for: agent)
        let before = readConfig(url)
        return Plan(agent: agent, configURL: url, before: before, after: HookConfig.removed(from: before))
    }

    /// Keeps a .bak before writing, and writes via a temp file swap.
    static func apply(_ plan: Plan) throws {
        let manager = FileManager.default
        try manager.createDirectory(at: plan.configURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        if manager.fileExists(atPath: plan.configURL.path) {
            let backup = plan.configURL.appendingPathExtension("bak")
            if manager.fileExists(atPath: backup.path) { try manager.removeItem(at: backup) }
            try manager.copyItem(at: plan.configURL, to: backup)
        }
        // Don't escape slashes: JSONSerialization writes / as \/ by default, which leaves paths looking chewed.
        let data = try JSONSerialization.data(withJSONObject: plan.after, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        try data.write(to: plan.configURL, options: .atomic)
    }
}
