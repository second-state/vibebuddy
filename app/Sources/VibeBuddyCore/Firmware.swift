import Foundation

/// 固件只比哈希：盒子报的构建标识是"哈希 日期 时间"，App 附带的也是。
/// 哈希不同就该显示「更新到 App 附带版本」，不判断新旧（docs/app.md「固件升级」）。
public enum Firmware {
    public static func hash(of build: String?) -> String? {
        guard let build else { return nil }
        let first = build.split(separator: " ").first.map(String.init) ?? ""
        return first.isEmpty ? nil : first
    }

    /// 附带版本存在且与盒子不同时才可更新；盒子还没报构建号时不催。
    public static func updateAvailable(device: String?, bundled: String?) -> Bool {
        guard let deviceHash = hash(of: device), let bundledHash = hash(of: bundled) else { return false }
        return deviceHash != bundledHash
    }

    /// 串口开了这么久还没报构建号，就当盒子跑的不是 Vibe Buddy 固件（出厂机）。
    /// 我们的固件在 daemon 发 hello 后一秒内就会报，宽限取 5 秒。
    public static let silenceGrace: TimeInterval = 5

    public static func foreign(connected: Bool, device: String?, connectedFor: TimeInterval) -> Bool {
        connected && device == nil && connectedFor >= silenceGrace
    }
}
