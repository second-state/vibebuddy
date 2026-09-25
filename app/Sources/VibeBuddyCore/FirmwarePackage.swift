import Foundation

/// A flashable firmware: the three images plus a build ID. The copy bundled with the app and one the user picks from a file
/// both look like this; file names match the bundling script and the zip CI produces.
public struct FirmwarePackage: Equatable {
    public let bootloader: URL
    public let partitionTable: URL
    public let app: URL
    /// "hash date time", the same format the box footer reports; without build.txt, just the version string in the image.
    public let build: String

    public static let bootloaderName = "bootloader.bin"
    public static let partitionTableName = "partition-table.bin"
    public static let appName = "vibebuddy-fw.bin"
    public static let buildName = "build.txt"

    public enum Failure: Error, LocalizedError, Equatable {
        case missing(String)
        case notAnImage(String)

        public var errorDescription: String? {
            switch self {
            case .missing(let name): return String(localized: "The firmware package has no \(name)")
            case .notAnImage(let name): return String(localized: "\(name) is not an ESP32-S3 image")
            }
        }
    }

    /// Finds the three images in a directory (including subdirectories, since zips often add a folder), checks magic numbers, reads the build ID.
    public static func inspect(directory: URL) throws -> FirmwarePackage {
        let files = try locate(in: directory)
        guard let bootloader = files[bootloaderName] else { throw Failure.missing(bootloaderName) }
        guard let table = files[partitionTableName] else { throw Failure.missing(partitionTableName) }
        guard let app = files[appName] else { throw Failure.missing(appName) }

        // An ESP image header starts with byte 0xE9; each partition table entry starts with 0xAA 0x50;
        // the app image has esp_app_desc's magic 0xABCD5432 (little endian) at offset 0x20.
        guard try prefix(of: bootloader, count: 1) == Data([0xE9]) else { throw Failure.notAnImage(bootloaderName) }
        guard try prefix(of: table, count: 2) == Data([0xAA, 0x50]) else { throw Failure.notAnImage(partitionTableName) }
        let header = try prefix(of: app, count: 0x50)
        guard header.count == 0x50, header[0x20..<0x24] == Data([0x32, 0x54, 0xCD, 0xAB]) else { throw Failure.notAnImage(appName) }

        let build: String
        if let stamp = files[buildName], let text = try? String(contentsOf: stamp, encoding: .utf8),
           !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            build = text.trimmingCharacters(in: .whitespacesAndNewlines)
        } else {
            let version = header[0x30..<0x50].prefix { $0 != 0 }
            build = String(decoding: version, as: UTF8.self)
        }
        return FirmwarePackage(bootloader: bootloader, partitionTable: table, app: app, build: build)
    }

    private static func locate(in directory: URL) throws -> [String: URL] {
        let wanted: Set<String> = [bootloaderName, partitionTableName, appName, buildName]
        var found: [String: URL] = [:]
        guard let walker = FileManager.default.enumerator(at: directory, includingPropertiesForKeys: [.isRegularFileKey]) else {
            throw Failure.missing(appName)
        }
        for case let url as URL in walker {
            let name = url.lastPathComponent
            guard wanted.contains(name), found[name] == nil,
                  (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true else { continue }
            found[name] = url
        }
        return found
    }

    private static func prefix(of url: URL, count: Int) throws -> Data {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        return try handle.read(upToCount: count) ?? Data()
    }
}
