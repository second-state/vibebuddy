import Foundation
import VibeBuddyCore

/// Hook setup: copies vibebuddy-hook to Application Support, then merges the hook entries into
/// the user-level config. Only our own entries are added or removed (VibeBuddyCore.HookConfig). OpenCode and
/// Copilot get a whole file of Vibe Buddy's own instead, made by the hook binary.
struct HookInstaller {
    struct Plan {
        let agent: HookAgent
        let configURL: URL
        let diff: [String]
        /// What the file becomes; nil deletes it.
        let contents: Data?
    }

    /// The agent's own directory, which exists once it has run here. The hook binary looks in the same places
    /// (`cli_agents.rs`); `XDG_CONFIG_HOME` and `COPILOT_HOME` aren't seen here, set as they are in shells.
    private static func directory(_ agent: HookAgent) -> URL {
        let home = FileManager.default.homeDirectoryForCurrentUser
        switch agent {
        case .codex: return home.appendingPathComponent(".codex")
        case .claude: return home.appendingPathComponent(".claude")
        case .opencode: return home.appendingPathComponent(".config/opencode")
        case .copilot: return home.appendingPathComponent(".copilot")
        }
    }

    static func configURL(for agent: HookAgent) -> URL {
        switch agent {
        case .codex: return directory(agent).appendingPathComponent("hooks.json")
        case .claude: return directory(agent).appendingPathComponent("settings.json")
        case .opencode: return directory(agent).appendingPathComponent("plugins/vibebuddy.js")
        case .copilot: return directory(agent).appendingPathComponent("hooks/vibebuddy.json")
        }
    }

    /// Whether this agent is installed on this machine: checks for its user directory.
    static func isPresent(_ agent: HookAgent) -> Bool {
        FileManager.default.fileExists(atPath: directory(agent).path)
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
        if agent.ownsFile {
            return (try? String(contentsOf: configURL(for: agent), encoding: .utf8))?.contains(Resources.installedHookBinary.path) ?? false
        }
        return HookConfig.isInstalled(in: readConfig(configURL(for: agent)), agent: agent, binary: Resources.installedHookBinary.path)
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
        for agent in HookAgent.allCases where !agent.ownsFile {
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
        if agent.ownsFile {
            // Without the hook to make the file there is nothing to write; never let that read as "delete it".
            guard let contents = ownedFileContents(agent) else { return Plan(agent: agent, configURL: url, diff: [], contents: nil) }
            return ownedFilePlan(agent, url: url, after: contents)
        }
        let before = readConfig(url)
        let after = HookConfig.install(into: before, agent: agent, binary: Resources.installedHookBinary.path)
        return mergePlan(agent, url: url, before: before, after: after)
    }

    static func removePlan(for agent: HookAgent) -> Plan {
        let url = configURL(for: agent)
        if agent.ownsFile { return ownedFilePlan(agent, url: url, after: nil) }
        let before = readConfig(url)
        return mergePlan(agent, url: url, before: before, after: HookConfig.removed(from: before))
    }

    private static func mergePlan(_ agent: HookAgent, url: URL, before: [String: Any], after: [String: Any]) -> Plan {
        // Don't escape slashes: JSONSerialization writes / as \/ by default, which leaves paths looking chewed.
        let data = try? JSONSerialization.data(withJSONObject: after, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        return Plan(agent: agent, configURL: url, diff: HookConfig.describeChange(from: before, to: after, agent: agent), contents: data)
    }

    private static func ownedFilePlan(_ agent: HookAgent, url: URL, after: String?) -> Plan {
        let before = try? String(contentsOf: url, encoding: .utf8)
        return Plan(agent: agent, configURL: url, diff: HookConfig.describeOwnedFile(from: before, to: after), contents: after.map { Data($0.utf8) })
    }

    /// Asks the bundled hook for the file, pointing at the copy in Application Support, which the file will run.
    private static func ownedFileContents(_ agent: HookAgent) -> String? {
        let process = Process()
        process.executableURL = Resources.hookBinary
        process.arguments = ["agent-file", agent.rawValue, Resources.installedHookBinary.path]
        let output = Pipe()
        process.standardOutput = output
        guard (try? process.run()) != nil else { return nil }
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Keeps a .bak before writing, and writes via a temp file swap. A file Vibe Buddy owns is simply deleted.
    static func apply(_ plan: Plan) throws {
        let manager = FileManager.default
        guard let contents = plan.contents else {
            if manager.fileExists(atPath: plan.configURL.path) { try manager.removeItem(at: plan.configURL) }
            return
        }
        try manager.createDirectory(at: plan.configURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        if !plan.agent.ownsFile, manager.fileExists(atPath: plan.configURL.path) {
            let backup = plan.configURL.appendingPathExtension("bak")
            if manager.fileExists(atPath: backup.path) { try manager.removeItem(at: backup) }
            try manager.copyItem(at: plan.configURL, to: backup)
        }
        try contents.write(to: plan.configURL, options: .atomic)
    }
}
