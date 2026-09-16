import Foundation
import VibeBuddyCore

/// 接入：把 vibebuddy-hook 复制到 Application Support，再把 Hook 条目合并进
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

    /// 配置文件最近一次被写的时间，和 daemon 报的最近一次事件时间比，就知道
    /// Codex 有没有在跑这份配置（VibeBuddyCore.HookConfig.codexTrustHint）。
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

    /// 把包里的二进制复制到固定位置；每次启动都做，App 升级后 Hook 也跟着新。
    static func deployBinary() throws {
        let destination = Resources.installedHookBinary
        try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
        // 改名前的二进制跟着旧目录搬过来了，没人再引用它。
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

    /// 改名后旧路径的条目没法再跑：是我们自己写的，就直接换成新路径，不用再问。
    /// 但要告诉调用方改写了谁：Codex 对改过的 hook 一律停用到人重新信任为止，
    /// 这一步 App 替不了，不说一声用户只会发现盒子对 Codex 没了反应。
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
