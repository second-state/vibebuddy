import Foundation

/// 固件只比哈希：盒子报的构建标识是"哈希 日期 时间"，App 附带的也是。
/// 哈希不同就该显示「更新到 App 附带版本」，不判断新旧（设计文档 Q24）。
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
}
