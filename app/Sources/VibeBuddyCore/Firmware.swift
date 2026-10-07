import Foundation

/// Which firmware to offer is the daemon's call (it reads the update manifest, ADR-0010); the app only shows it.
public enum Firmware {
    /// How a firmware is shown: version first, then the build ID; firmware older than ADR-0010 has only the build.
    public static func label(version: String?, build: String?) -> String? {
        switch (version, build) {
        case let (version?, build?): return "\(version) · \(build)"
        case let (version?, nil): return version
        case let (nil, build): return build
        }
    }

    /// The images of the offered firmware, once downloaded; what flashing needs.
    public static func files(of offer: FirmwareOffer?) -> (bootloader: URL, partitionTable: URL, app: URL)? {
        guard let directory = offer?.directory.map({ URL(fileURLWithPath: $0, isDirectory: true) }) else { return nil }
        return (directory.appendingPathComponent(FirmwarePackage.bootloaderName),
                directory.appendingPathComponent(FirmwarePackage.partitionTableName),
                directory.appendingPathComponent(FirmwarePackage.appName))
    }

    /// An update is offered only for a box that runs something older, and only once the firmware is on disk.
    public static func updateAvailable(_ updates: UpdateStatus?) -> Bool {
        guard let offer = updates?.firmware else { return false }
        return offer.newerThanBox && offer.directory != nil
    }

    /// Release notes in the UI's language, English otherwise.
    public static func notes(_ notes: [String: String], chinese: Bool) -> String? {
        (chinese ? notes["zh-Hans"] : nil) ?? notes["en"]
    }

    /// The box is connected but runs other firmware (a factory box, or Muse on a box that runs it). The daemon judges
    /// it from what the box prints (docs/architecture.md, decision 17); a reported build always means it is ours.
    public static func foreign(_ device: DeviceState) -> Bool {
        device.connected && device.foreignFirmware == true && device.firmwareBuild == nil
    }
}
