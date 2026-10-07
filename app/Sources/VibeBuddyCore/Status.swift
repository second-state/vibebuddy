import Foundation

/// The snapshot from the daemon's `GET /v1/status` and status stream; fields match daemon/src/status.rs.
public struct Status: Codable, Equatable {
    public var daemon: DaemonInfo
    public var device: DeviceState
    public var today: TodaySummary
    public var hooks: HooksSeen
    public var operation: DeviceOperation?
    public var config: DaemonConfig

    public init(daemon: DaemonInfo, device: DeviceState, today: TodaySummary, hooks: HooksSeen, operation: DeviceOperation?, config: DaemonConfig) {
        self.daemon = daemon
        self.device = device
        self.today = today
        self.hooks = hooks
        self.operation = operation
        self.config = config
    }
}

public struct DaemonInfo: Codable, Equatable {
    public var build: String
    public var appVersion: String?
    enum CodingKeys: String, CodingKey { case build, appVersion = "app_version" }
    public init(build: String, appVersion: String?) { self.build = build; self.appVersion = appVersion }
}

public struct DeviceState: Codable, Equatable {
    public var connected: Bool
    public var port: String?
    public var bridge: Bool
    public var mode: String?
    public var firmwareBuild: String?
    /// The firmware version, such as "0.2.2"; firmware older than ADR-0010 and older daemons report none.
    public var firmwareVersion: String?
    public var voice: String?
    /// Speaker volume (20 to 100) as reported by the box; the app is just a remote control.
    public var volume: Int?
    /// Connected but running other firmware, as the daemon judged it; absent from older daemons.
    public var foreignFirmware: Bool?
    enum CodingKeys: String, CodingKey { case connected, port, bridge, mode, firmwareBuild = "firmware_build", firmwareVersion = "firmware_version", voice, volume, foreignFirmware = "foreign_firmware" }
    public init(connected: Bool, port: String? = nil, bridge: Bool = false, mode: String? = nil, firmwareBuild: String? = nil, firmwareVersion: String? = nil, voice: String? = nil, volume: Int? = nil, foreignFirmware: Bool? = nil) {
        self.connected = connected; self.port = port; self.bridge = bridge; self.mode = mode; self.firmwareBuild = firmwareBuild; self.firmwareVersion = firmwareVersion; self.voice = voice; self.volume = volume
        self.foreignFirmware = foreignFirmware
    }
}

public struct TodaySummary: Codable, Equatable {
    public var done: Int
    public var asks: Int
    public var busySeconds: Int
    enum CodingKeys: String, CodingKey { case done, asks, busySeconds = "busy_seconds" }
    public init(done: Int, asks: Int, busySeconds: Int) { self.done = done; self.asks = asks; self.busySeconds = busySeconds }
}

public struct HooksSeen: Codable, Equatable {
    public var codex: Date?
    public var claude: Date?
    public init(codex: Date? = nil, claude: Date? = nil) { self.codex = codex; self.claude = claude }
}

/// Named DeviceOperation rather than Operation, which clashes with Foundation's.
public struct DeviceOperation: Codable, Equatable {
    public enum Kind: String, Codable { case voicePack = "voice_pack", firmware }
    public enum State: String, Codable { case running, done, failed, replug }
    public var kind: Kind
    public var state: State
    public var progress: Double
    public var message: String
    public init(kind: Kind, state: State, progress: Double, message: String) { self.kind = kind; self.state = state; self.progress = progress; self.message = message }
}

public struct DaemonConfig: Codable, Equatable {
    public var voice: String?
    public var notifyLink: Bool
    enum CodingKeys: String, CodingKey { case voice, notifyLink = "notify_link" }
    public init(voice: String? = nil, notifyLink: Bool = true) { self.voice = voice; self.notifyLink = notifyLink }
}

public enum StatusCoding {
    /// The daemon writes chrono's RFC 3339 with microseconds and a UTC offset; Foundation's ISO8601
    /// decoder needs fractional seconds enabled explicitly, so try both forms.
    public static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        decoder.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            if let date = fractional.date(from: text) ?? plain.date(from: text) { return date }
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "bad timestamp: \(text)"))
        }
        return decoder
    }

    public static func encoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }
}
