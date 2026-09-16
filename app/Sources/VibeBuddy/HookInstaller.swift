import Foundation
import VibeBuddyCore

/// 接入：把 beacon-hook 复制到 Application Support，再把 Hook 条目合并进
/// 用户级配置。只增删自己的条目（VibeBuddyCore.HookConfig）。
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

    /// 这个 Agent 装在这台机器上吗：看它的用户目录在不在。
    static func isPresent(_ agent: HookAgent) -> Bool {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let directory = agent == .codex ? ".codex" : ".claude"
        return FileManager.default.fileExists(atPath: home.appendingPathComponent(directory).path)
    }

    static func readConfig(_ url: URL) -> [String: Any] {
        guard let data = try? Data(contentsOf: url),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return object
    }

    static func isInstalled(_ agent: HookAgent) -> Bool {
        HookConfig.isInstalled(in: readConfig(configURL(for: agent)), agent: agent, binary: Resources.installedHookBinary.path)
    }

    /// 把包里的二进制复制到固定位置；每次启动都做，App 升级后 Hook 也跟着新。
    static func deployBinary() throws {
        let destination = Resources.installedHookBinary
        try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
        let source = Resources.hookBinary
        guard FileManager.default.fileExists(atPath: source.path) else { return }
        if FileManager.default.fileExists(atPath: destination.path) {
            try FileManager.default.removeItem(at: destination)
        }
        try FileManager.default.copyItem(at: source, to: destination)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: destination.path)
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

    /// 写前留一份 .bak，写入用临时文件替换。
    static func apply(_ plan: Plan) throws {
        let manager = FileManager.default
        try manager.createDirectory(at: plan.configURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        if manager.fileExists(atPath: plan.configURL.path) {
            let backup = plan.configURL.appendingPathExtension("bak")
            if manager.fileExists(atPath: backup.path) { try manager.removeItem(at: backup) }
            try manager.copyItem(at: plan.configURL, to: backup)
        }
        // 不转义斜杠：JSONSerialization 默认把 / 写成 \/，路径会难看得像被咬过。
        let data = try JSONSerialization.data(withJSONObject: plan.after, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        try data.write(to: plan.configURL, options: .atomic)
    }
}
