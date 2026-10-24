import Foundation

/// The snapshot from the daemon's `GET /v1/status` and status stream; fields match daemon/src/status.rs.
public struct Status: Codable, Equatable {
    public var daemon: DaemonInfo
    public var device: DeviceState
    public var today: TodaySummary
    public var hooks: HooksSeen
    public var operation: DeviceOperation?
    public var config: DaemonConfig
    /// What the update manifest offers (ADR-0010); absent from older daemons.
    public var updates: UpdateStatus?
    /// The person's VibeBuddy account (ADR-0018); absent from older daemons, which can't sign in.
    public var account: AccountStatus?

    public init(daemon: DaemonInfo, device: DeviceState, today: TodaySummary, hooks: HooksSeen, operation: DeviceOperation?, config: DaemonConfig, updates: UpdateStatus? = nil, account: AccountStatus? = nil) {
        self.daemon = daemon
        self.device = device
        self.today = today
        self.hooks = hooks
        self.operation = operation
        self.config = config
        self.updates = updates
        self.account = account
    }
}

/// An account has an email or a GitHub user, never both.
public struct AccountStatus: Codable, Equatable {
    /// This build has a service to sign in to.
    public var available: Bool
    public var signedIn: Bool
    public var email: String?
    public var githubID: Int?
    /// Looked up on GitHub when signing in; nil if that lookup failed.
    public var githubLogin: String?
    /// Voice used in the current UTC day, when the daemon knows; absent from daemons before it.
    public var today: AccountUsage?
    /// Why the service refused voice today: `allowance` or `global_cap`.
    public var refused: String?
    /// The account's plan (ADR-0020), when the daemon knows.
    public var plan: AccountPlan?
    enum CodingKeys: String, CodingKey { case available, signedIn = "signed_in", email, githubID = "github_id", githubLogin = "github_login", today, refused, plan }
    public init(available: Bool, signedIn: Bool, email: String? = nil, githubID: Int? = nil, githubLogin: String? = nil, today: AccountUsage? = nil, refused: String? = nil, plan: AccountPlan? = nil) {
        self.available = available; self.signedIn = signedIn; self.email = email; self.githubID = githubID; self.githubLogin = githubLogin
        self.today = today; self.refused = refused; self.plan = plan
    }
}

/// `free`, or `paid` with the billing period's end and its monthly allowance, in characters of our voices.
public struct AccountPlan: Codable, Equatable {
    public var name: String
    public var periodEnd: Date?
    public var characters: Int?
    public var charactersLimit: Int?
    enum CodingKeys: String, CodingKey { case name, periodEnd = "period_end", characters, charactersLimit = "characters_limit" }
    public init(name: String, periodEnd: Date? = nil, characters: Int? = nil, charactersLimit: Int? = nil) {
        self.name = name; self.periodEnd = periodEnd; self.characters = characters; self.charactersLimit = charactersLimit
    }
    public var paid: Bool { name == "paid" }
}

public struct AccountUsage: Codable, Equatable {
    public var day: String
    public var commands: Int
    public var commandsLimit: Int
    enum CodingKeys: String, CodingKey { case day, commands, commandsLimit = "commands_limit" }
    public init(day: String, commands: Int, commandsLimit: Int) {
        self.day = day; self.commands = commands; self.commandsLimit = commandsLimit
    }
}

public struct DaemonInfo: Codable, Equatable {
    public var build: String
    /// The git description alone; nil from a daemon older than the field.
    public var revision: String?
    public var appVersion: String?
    enum CodingKeys: String, CodingKey { case build, revision, appVersion = "app_version" }
    public init(build: String, revision: String? = nil, appVersion: String?) {
        self.build = build; self.revision = revision; self.appVersion = appVersion
    }
    /// What the settings window shows: the revision, or the whole build line from an older daemon.
    public var shortBuild: String { revision ?? build }
}

/// An environment variable that narrows the daemon's search to one port or device.
public struct SerialPin: Codable, Equatable {
    public var variable: String
    public var value: String
    public init(variable: String, value: String) { self.variable = variable; self.value = value }
}

/// A device that could be the box, offered when several are plugged in and none is the remembered one.
public struct BoxCandidate: Codable, Equatable, Identifiable {
    public var port: String
    public var usbSerial: String?
    public var id: String { port }
    enum CodingKeys: String, CodingKey { case port, usbSerial = "usb_serial" }
    public init(port: String, usbSerial: String?) { self.port = port; self.usbSerial = usbSerial }
}

/// The box is linked to another computer (ADR-0011): `reason` is "usb" (on its cable, can't be taken) or "linked".
public struct BoxBusy: Codable, Equatable {
    public var owner: String
    public var reason: String
    public init(owner: String, reason: String) { self.owner = owner; self.reason = reason }
}

/// A network the box heard on its last scan.
public struct SeenNetwork: Codable, Equatable, Identifiable {
    public var ssid: String
    public var rssi: Int
    public var id: String { ssid }
    public init(ssid: String, rssi: Int) { self.ssid = ssid; self.rssi = rssi }
}

/// A computer the box is paired with (ADR-0012).
public struct PairedComputer: Codable, Equatable, Identifiable {
    public var key: String
    public var name: String
    public var id: String { key }
    public init(key: String, name: String) { self.key = key; self.name = name }
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
    /// Absent from older daemons, like the two below.
    public var pin: SerialPin?
    public var candidates: [BoxCandidate]?
    /// Our firmware built for other hardware (the breadboard devkit); released firmware isn't offered to it. Absent from older daemons.
    public var unsupportedBoard: String?
    /// The box's key; nil for firmware that can't pair. Absent from older daemons, like the two below.
    public var boxKey: String?
    /// This computer's key, to tell it apart in `pairedComputers`.
    public var computerKey: String?
    public var pairedComputers: [PairedComputer]?
    /// Linked over the local network; `port` is then the box's address. Absent from older daemons.
    public var network: Bool?
    /// Linked through the Relay; `port` is then the Relay's address. Absent from older daemons, like busy.
    public var relay: Bool?
    public var busy: BoxBusy?
    /// The Wi-Fi network the box is set to join, and the address it got there. Absent from older daemons.
    public var wifiNetwork: String?
    public var wifiAddress: String?
    /// Why the box couldn't join the network, until it does. Absent from older daemons.
    public var wifiError: String?
    /// What the box heard on its last scan, strongest first. Absent from older daemons.
    public var wifiSeen: [SeenNetwork]?
    /// The firmware takes updates over any link (ADR-0016): no cable, no download mode. Absent from older daemons.
    public var transfer: Bool?
    enum CodingKeys: String, CodingKey { case transfer, wifiNetwork = "wifi_network", wifiAddress = "wifi_address", wifiError = "wifi_error", wifiSeen = "wifi_seen", relay, busy, connected, port, bridge, network, mode, firmwareBuild = "firmware_build", firmwareVersion = "firmware_version", voice, volume, foreignFirmware = "foreign_firmware", pin, candidates, unsupportedBoard = "unsupported_board", boxKey = "box_key", computerKey = "computer_key", pairedComputers = "paired_computers" }
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
    /// Absent from daemons older than OpenCode and Copilot support.
    public var opencode: Date?
    public var copilot: Date?
    /// Absent from daemons older than Pi support.
    public var pi: Date?
    public init(codex: Date? = nil, claude: Date? = nil, opencode: Date? = nil, copilot: Date? = nil, pi: Date? = nil) {
        self.codex = codex; self.claude = claude; self.opencode = opencode; self.copilot = copilot; self.pi = pi
    }

    public func lastEvent(_ agent: HookAgent) -> Date? {
        switch agent {
        case .codex: return codex
        case .claude: return claude
        case .opencode: return opencode
        case .copilot: return copilot
        case .pi: return pi
        }
    }
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
    /// nil follows the build: on in releases CI makes, off when built from source.
    public var checkUpdates: Bool?
    /// Reach the box through the Relay when nothing closer does (ADR-0012); nil, from an older daemon, means on.
    public var relay: Bool?
    enum CodingKeys: String, CodingKey { case voice, notifyLink = "notify_link", checkUpdates = "check_updates", relay }
    public init(voice: String? = nil, notifyLink: Bool = true, checkUpdates: Bool? = nil, relay: Bool? = nil) {
        self.voice = voice; self.notifyLink = notifyLink; self.checkUpdates = checkUpdates; self.relay = relay
    }
}

/// The daemon's view of updates, from the signed update manifest.
public struct UpdateStatus: Codable, Equatable {
    /// Checks run: switched on, and the build knows where to check.
    public var enabled: Bool
    public var lastCheck: Date?
    public var error: String?
    /// A newer App for this platform.
    public var app: AppOffer?
    /// This App is older than the oldest one still supported.
    public var unsupportedApp: Bool
    /// The newest firmware this App can run.
    public var firmware: FirmwareOffer?
    enum CodingKeys: String, CodingKey { case enabled, lastCheck = "last_check", error, app, unsupportedApp = "unsupported_app", firmware }
    public init(enabled: Bool, lastCheck: Date? = nil, error: String? = nil, app: AppOffer? = nil, unsupportedApp: Bool = false, firmware: FirmwareOffer? = nil) {
        self.enabled = enabled; self.lastCheck = lastCheck; self.error = error; self.app = app; self.unsupportedApp = unsupportedApp; self.firmware = firmware
    }
}

public struct AppOffer: Codable, Equatable {
    public var version: String
    public var url: String
    public var notes: [String: String]
    public init(version: String, url: String, notes: [String: String]) { self.version = version; self.url = url; self.notes = notes }
}

public struct FirmwareOffer: Codable, Equatable {
    public var version: String
    public var notes: [String: String]
    /// The unpacked images once the daemon has downloaded and checked them.
    public var directory: String?
    /// The connected box runs something older, unversioned or a dirty build; never set for a downgrade.
    public var newerThanBox: Bool
    enum CodingKeys: String, CodingKey { case version, notes, directory, newerThanBox = "newer_than_box" }
    public init(version: String, notes: [String: String], directory: String? = nil, newerThanBox: Bool) {
        self.version = version; self.notes = notes; self.directory = directory; self.newerThanBox = newerThanBox
    }
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
