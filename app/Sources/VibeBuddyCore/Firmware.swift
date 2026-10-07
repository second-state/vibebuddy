import Foundation

/// Firmware is compared by hash only: the box reports its build ID as "hash date time", and so does the bundled copy.
/// A different hash means offering "Update to bundled version", without judging which is newer (docs/app.md, firmware upgrades).
public enum Firmware {
    public static func hash(of build: String?) -> String? {
        guard let build else { return nil }
        let first = build.split(separator: " ").first.map(String.init) ?? ""
        return first.isEmpty ? nil : first
    }

    /// How a firmware is shown: version first, then the build ID; firmware older than ADR-0010 has only the build.
    public static func label(version: String?, build: String?) -> String? {
        switch (version, build) {
        case let (version?, build?): return "\(version) · \(build)"
        case let (version?, nil): return version
        case let (nil, build): return build
        }
    }

    /// An update is available only when a bundled version exists and differs from the box's; no nagging before the box reports its build.
    public static func updateAvailable(device: String?, bundled: String?) -> Bool {
        guard let deviceHash = hash(of: device), let bundledHash = hash(of: bundled) else { return false }
        return deviceHash != bundledHash
    }

    /// The box is connected but runs other firmware (a factory box, or Muse on a box that runs it). The daemon judges
    /// it from what the box prints (docs/architecture.md, decision 17); a reported build always means it is ours.
    public static func foreign(_ device: DeviceState) -> Bool {
        device.connected && device.foreignFirmware == true && device.firmwareBuild == nil
    }
}
