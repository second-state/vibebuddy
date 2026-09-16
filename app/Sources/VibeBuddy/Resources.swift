import Foundation
import VibeBuddyCore

/// App 包里的东西在哪：两个 helper、固件三件套、语音包。
enum Resources {
    /// 语义版本，随心跳报给设备。
    static var bundleVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
    }

    /// 构建号（git 描述），只在界面上显示。
    static var bundleBuild: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "dev"
    }

    static var displayVersion: String { "\(bundleVersion) (\(bundleBuild))" }

    static var macOSDirectory: URL {
        Bundle.main.executableURL!.deletingLastPathComponent()
    }

    static var daemonBinary: URL { macOSDirectory.appendingPathComponent("beacond") }
    static var hookBinary: URL { macOSDirectory.appendingPathComponent("beacon-hook") }

    static var resourcesDirectory: URL {
        Bundle.main.resourceURL ?? macOSDirectory
    }

    static var firmwareDirectory: URL { resourcesDirectory.appendingPathComponent("firmware") }

    /// 附带固件的构建标识（打包脚本写的），没有附带固件时为 nil。
    static var bundledFirmwareBuild: String? {
        let url = firmwareDirectory.appendingPathComponent("build.txt")
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    static var firmwareFiles: (bootloader: URL, partitionTable: URL, app: URL)? {
        let bootloader = firmwareDirectory.appendingPathComponent("bootloader.bin")
        let table = firmwareDirectory.appendingPathComponent("partition-table.bin")
        let app = firmwareDirectory.appendingPathComponent("agent-beacon-fw.bin")
        let manager = FileManager.default
        guard [bootloader, table, app].allSatisfy({ manager.fileExists(atPath: $0.path) }) else { return nil }
        return (bootloader, table, app)
    }

    static func voicePackURL(_ id: String) -> URL {
        resourcesDirectory.appendingPathComponent("voices").appendingPathComponent("\(id).bin")
    }

    static func voicePack(_ id: String) -> VoicePack? {
        guard let data = try? Data(contentsOf: voicePackURL(id)) else { return nil }
        return VoicePack(data: data)
    }

    /// Application Support 下的固定位置：Hook 二进制、配置、日志都在附近。
    static var applicationSupport: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AgentBeacon")
    }

    static var installedHookBinary: URL { applicationSupport.appendingPathComponent("bin/beacon-hook") }
    static var logsDirectory: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/AgentBeacon")
    }
}
