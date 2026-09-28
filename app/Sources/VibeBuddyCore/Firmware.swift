import Foundation

/// Firmware is compared by hash only: the box reports its build ID as "hash date time", and so does the bundled copy.
/// A different hash means offering "Update to bundled version", without judging which is newer (docs/app.md, firmware upgrades).
public enum Firmware {
    public static func hash(of build: String?) -> String? {
        guard let build else { return nil }
        let first = build.split(separator: " ").first.map(String.init) ?? ""
        return first.isEmpty ? nil : first
    }

    /// An update is available only when a bundled version exists and differs from the box's; no nagging before the box reports its build.
    public static func updateAvailable(device: String?, bundled: String?) -> Bool {
        guard let deviceHash = hash(of: device), let bundledHash = hash(of: bundled) else { return false }
        return deviceHash != bundledHash
    }

    /// If the serial port has been open this long without a build ID, assume the box isn't running Vibe Buddy firmware (a factory box).
    /// Our firmware reports within a second of the daemon's hello, so the grace period is 5 seconds.
    public static let silenceGrace: TimeInterval = 5

    public static func foreign(connected: Bool, device: String?, connectedFor: TimeInterval) -> Bool {
        connected && device == nil && connectedFor >= silenceGrace
    }
}
