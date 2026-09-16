import Foundation

/// 两个 Agent 的 Hook 配置：只增删 Vibe Buddy 自己的条目，不动别人的 Hook。
/// Claude Code 的用户级 settings.json 与 Codex 的 hooks.json 结构相同：
/// `hooks.<事件>: [ { hooks: [ { type: "command", command, timeout } ] } ]`。
public enum HookAgent: String, CaseIterable {
    case codex, claude

    public var events: [String] {
        switch self {
        case .codex: return ["UserPromptSubmit", "PermissionRequest", "PostToolUse", "Stop", "Interrupt", "SessionEnd"]
        case .claude: return ["UserPromptSubmit", "PermissionRequest", "PostToolUse", "Stop", "StopFailure", "SubagentStart", "SubagentStop", "SessionEnd"]
        }
    }

    public var displayName: String {
        switch self {
        case .codex: return "Codex"
        case .claude: return "Claude Code"
        }
    }
}

public enum HookConfig {
    /// 判断一条 command 是不是我们的：旧的 Python 脚本也算，升级时一并替换。
    public static func isOurs(_ command: String) -> Bool {
        command.contains("beacon-hook") || command.contains("codex-hook.py") || command.contains("claude-hook.py")
    }

    /// 我们要写进去的那条命令。
    public static func command(binary: String, agent: HookAgent) -> String {
        "\"\(binary)\" \(agent.rawValue)"
    }

    /// 把 Vibe Buddy 的 Hook 合并进配置。返回新配置。
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

    /// 去掉 Vibe Buddy 的条目；空掉的事件与空掉的 hooks 也一并删。
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

    /// 配置里是否已经装了指向这个二进制的 Hook（所有事件都在才算装好）。
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

    /// 写前给用户看的差异：逐事件说明加了什么、去了什么。
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
