import Foundation

/// Hook config for both agents: only adds or removes VibeBuddy's own entries, never anyone else's hooks.
/// Claude Code's user-level settings.json and Codex's hooks.json share one shape:
/// `hooks.<event>: [ { hooks: [ { type: "command", command, timeout } ] } ]`.
public enum HookAgent: String, CaseIterable {
    case codex, claude, opencode, copilot, pi

    /// The events merged into a shared config; empty for agents whose whole file VibeBuddy owns.
    public var events: [String] {
        switch self {
        case .codex: return ["UserPromptSubmit", "PermissionRequest", "PostToolUse", "Stop", "Interrupt", "SessionEnd"]
        case .claude: return ["UserPromptSubmit", "PermissionRequest", "PostToolUse", "Stop", "StopFailure", "SubagentStart", "SubagentStop", "SessionEnd"]
        case .opencode, .copilot, .pi: return []
        }
    }

    /// OpenCode, Copilot and Pi get a file of VibeBuddy's own in their config directory, instead of entries merged
    /// into the user's: connecting writes it, removing deletes it. The hook binary makes its contents.
    public var ownsFile: Bool { events.isEmpty }

    public var displayName: String {
        switch self {
        case .codex: return "Codex"
        case .claude: return "Claude Code"
        case .opencode: return "OpenCode"
        case .copilot: return "GitHub Copilot CLI"
        case .pi: return "Pi"
        }
    }
}

/// For every hook in hooks.json, Codex keeps a trusted_hash under `[hooks.state]` in config.toml.
/// Once the file changes, the changed entries are marked "pending review" and silently stop running until someone
/// re-trusts them in Codex (/hooks in the terminal, Settings › Hooks in the app). The app doesn't recompute that hash (it's Codex's internal format
/// and changes with its versions); it only looks at one observable fact: has any Codex event arrived since the config was written?
public enum CodexTrustHint: Equatable {
    /// No event received yet: maybe it isn't trusted, maybe Codex just hasn't been used today. No verdict.
    case waitingFirstEvent
    /// The config changed after the last event and Codex hasn't called it since: almost certainly not trusted.
    case changedSinceLastEvent(Date)
    /// An event arrived after the config was written, so Codex is running it.
    case trusted
}

public enum HookConfig {
    /// The change shown before writing a file VibeBuddy owns: the whole file, added or removed.
    public static func describeOwnedFile(from before: String?, to after: String?) -> [String] {
        guard before != after else { return [] }
        let removed = (before ?? "").split(separator: "\n", omittingEmptySubsequences: false).filter { !$0.isEmpty }.map { "- \($0)" }
        let added = (after ?? "").split(separator: "\n", omittingEmptySubsequences: false).filter { !$0.isEmpty }.map { "+ \($0)" }
        return removed + added
    }

    public static func codexTrustHint(configModifiedAt: Date?, lastEvent: Date?) -> CodexTrustHint {
        guard let lastEvent else { return .waitingFirstEvent }
        if let configModifiedAt, configModifiedAt > lastEvent { return .changedSinceLastEvent(configModifiedAt) }
        return .trusted
    }

    /// Whether a command is ours: the old Python scripts count too, and get replaced on upgrade.
    public static func isOurs(_ command: String) -> Bool {
        // The old name beacon-hook and the two earlier Python scripts count too, and get replaced on upgrade.
        command.contains("vibebuddy-hook") || command.contains("beacon-hook")
            || command.contains("codex-hook.py") || command.contains("claude-hook.py")
    }

    /// The command we write.
    public static func command(binary: String, agent: HookAgent) -> String {
        "\"\(binary)\" \(agent.rawValue)"
    }

    /// Merges VibeBuddy's hooks into the config and returns the new config.
    public static func install(into root: [String: Any], agent: HookAgent, binary: String) -> [String: Any] {
        var root = removed(from: root)
        var hooks = root["hooks"] as? [String: Any] ?? [:]
        let entry: [String: Any] = ["hooks": [["type": "command", "command": command(binary: binary, agent: agent), "timeout": 2]]]
        for event in agent.events {
            var groups = hooks[event] as? [[String: Any]] ?? []
            groups.append(entry)
            hooks[event] = groups
        }
        root["hooks"] = hooks
        return root
    }

    /// Removes VibeBuddy's entries, along with any events and hooks objects left empty.
    public static func removed(from root: [String: Any]) -> [String: Any] {
        var root = root
        guard var hooks = root["hooks"] as? [String: Any] else { return root }
        for (event, value) in hooks {
            guard let groups = value as? [[String: Any]] else { continue }
            let kept = groups.compactMap { group -> [String: Any]? in
                var group = group
                let commands = (group["hooks"] as? [[String: Any]] ?? []).filter { hook in
                    !isOurs(hook["command"] as? String ?? "")
                }
                if commands.isEmpty { return nil }
                group["hooks"] = commands
                return group
            }
            if kept.isEmpty { hooks.removeValue(forKey: event) } else { hooks[event] = kept }
        }
        if hooks.isEmpty { root.removeValue(forKey: "hooks") } else { root["hooks"] = hooks }
        return root
    }

    /// Whether the config already has hooks pointing at this binary (installed only if every event is present).
    public static func isInstalled(in root: [String: Any], agent: HookAgent, binary: String) -> Bool {
        guard let hooks = root["hooks"] as? [String: Any] else { return false }
        let wanted = command(binary: binary, agent: agent)
        return agent.events.allSatisfy { event in
            let groups = hooks[event] as? [[String: Any]] ?? []
            return groups.contains { group in
                (group["hooks"] as? [[String: Any]] ?? []).contains { ($0["command"] as? String) == wanted }
            }
        }
    }

    /// Whether Codex trusts none of our entries. Codex skips a hook it hasn't trusted without a word, so this is the only
    /// way to tell "not trusted" from "not used yet". It keeps each trusted hook under
    /// `[hooks.state."<hooks.json>:<event>:<group>:<handler>"]` in `config.toml`, with a `trusted_hash`. Only whether one is
    /// there counts: the hash is Codex's own format. False when there is nothing of ours to trust.
    public static func codexTrustsNone(hooks root: [String: Any], config: String, hooksPath: String, binary: String) -> Bool {
        let (bypass, trusted) = trustedHookKeys(in: config)
        guard !bypass, let hooks = root["hooks"] as? [String: Any] else { return false }
        let wanted = command(binary: binary, agent: .codex)
        var ours: [String] = []
        for (event, value) in hooks {
            for (group, entry) in (value as? [[String: Any]] ?? []).enumerated() {
                for (handler, hook) in (entry["hooks"] as? [[String: Any]] ?? []).enumerated()
                    where (hook["command"] as? String) == wanted {
                    ours.append("\(hooksPath):\(snakeCase(event)):\(group):\(handler)")
                }
            }
        }
        return !ours.isEmpty && !ours.contains { trusted.contains($0) }
    }

    /// The hook keys `config.toml` has a `trusted_hash` for, and whether `bypass_hook_trust` turns the check off.
    static func trustedHookKeys(in config: String) -> (bypass: Bool, keys: Set<String>) {
        var keys = Set<String>()
        var bypass = false
        var section: String?
        var inTable = false
        for raw in config.split(whereSeparator: \.isNewline) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("[") {
                inTable = true
                let name = line.trimmingCharacters(in: CharacterSet(charactersIn: "[] "))
                section = name.hasPrefix("hooks.state.") ? tomlKey(String(name.dropFirst("hooks.state.".count))) : nil
            } else if let equals = line.firstIndex(of: "=") {
                let name = line[..<equals].trimmingCharacters(in: .whitespaces)
                let value = line[line.index(after: equals)...].trimmingCharacters(in: .whitespaces)
                if name == "trusted_hash", let section { keys.insert(section) }
                if name == "bypass_hook_trust", !inTable { bypass = value == "true" }
            }
        }
        return (bypass, keys)
    }

    /// A quoted TOML key: `"…"` with `\\` and `\"` escapes, or `'…'` as it is.
    static func tomlKey(_ quoted: String) -> String? {
        if quoted.hasPrefix("'"), quoted.hasSuffix("'"), quoted.count >= 2 { return String(quoted.dropFirst().dropLast()) }
        guard quoted.hasPrefix("\""), quoted.hasSuffix("\""), quoted.count >= 2 else { return nil }
        var key = ""
        var escaped = false
        for character in quoted.dropFirst().dropLast() {
            if escaped { key.append(character); escaped = false } else if character == "\\" { escaped = true } else { key.append(character) }
        }
        return key
    }

    /// `UserPromptSubmit` as Codex writes it in a hook key: `user_prompt_submit`.
    static func snakeCase(_ event: String) -> String {
        var out = ""
        for (index, character) in event.enumerated() {
            if character.isUppercase, index > 0 { out.append("_") }
            out.append(contentsOf: character.lowercased())
        }
        return out
    }

    /// The diff shown to the user before writing: what gets added and removed, event by event.
    public static func describeChange(from before: [String: Any], to after: [String: Any], agent: HookAgent) -> [String] {
        var lines: [String] = []
        let beforeHooks = before["hooks"] as? [String: Any] ?? [:]
        let afterHooks = after["hooks"] as? [String: Any] ?? [:]
        for event in Set(beforeHooks.keys).union(afterHooks.keys).sorted() {
            let earlier = commands(in: beforeHooks[event])
            let later = commands(in: afterHooks[event])
            for added in later where !earlier.contains(added) { lines.append("+ \(event): \(added)") }
            for gone in earlier where !later.contains(gone) { lines.append("- \(event): \(gone)") }
        }
        return lines
    }

    private static func commands(in value: Any?) -> [String] {
        (value as? [[String: Any]] ?? []).flatMap { group in
            (group["hooks"] as? [[String: Any]] ?? []).compactMap { $0["command"] as? String }
        }
    }
}
