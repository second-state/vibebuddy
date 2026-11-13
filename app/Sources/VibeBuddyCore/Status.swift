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
    /// Every Activity, newest first, as the box's task cards show them; absent from older daemons.
    public var activities: [ActivitySummary]?

    /// The person's VibeBuddy account (ADR-0018); absent from older daemons, which can't sign in.
    public var account: AccountStatus?
    /// Which of the user's own keys for spoken replies are set (ADR-0023); absent from older daemons.
    public var voiceKeys: VoiceKeysStatus?
    /// The Character being made, or the last one made since the daemon started (ADR-0023); absent when there is none,
    /// and from older daemons.
    public var making: MakingStatus?
    /// Every VibePlay paired with this computer or found on the network (ADR-0025); absent from older daemons.
    public var vibeplays: [VibePlay]?

    enum CodingKeys: String, CodingKey { case daemon, device, today, hooks, operation, config, updates, account, voiceKeys = "voice_keys", making, activities, vibeplays }

    public init(daemon: DaemonInfo, device: DeviceState, today: TodaySummary, hooks: HooksSeen, operation: DeviceOperation?, config: DaemonConfig, updates: UpdateStatus? = nil, account: AccountStatus? = nil, voiceKeys: VoiceKeysStatus? = nil, making: MakingStatus? = nil, activities: [ActivitySummary]? = nil, vibeplays: [VibePlay]? = nil) {
        self.daemon = daemon
        self.device = device
        self.today = today
        self.hooks = hooks
        self.operation = operation
        self.config = config
        self.updates = updates
        self.activities = activities
        self.account = account
        self.voiceKeys = voiceKeys
        self.making = making
        self.vibeplays = vibeplays
    }
}

/// A VibePlay, the game handheld that shows the buddy beside the box (ADR-0025, docs/vibeplay.md).
public struct VibePlay: Codable, Equatable, Identifiable {
    public var key: String
    public var name: String
    /// Paired with this computer.
    public var paired: Bool
    public var connected: Bool
    public var address: String?
    public var id: String { key }

    public init(key: String, name: String, paired: Bool, connected: Bool, address: String? = nil) {
        self.key = key
        self.name = name
        self.paired = paired
        self.connected = connected
        self.address = address
    }
}

/// One Activity as the daemon summarises it (daemon/src/activity.rs `ActivitySummary`).
public struct ActivitySummary: Codable, Equatable, Sendable {
    /// `codex`, `claude-code`, `github-actions`, or the agent a CLI hook names (`opencode`, `copilot`, `pi`).
    public var agent: String
    /// The task card's title, with the prefix that tells agents apart (`CC:`, `CX:`…).
    public var title: String
    public var project: String?
    /// `working` or `needs_input`.
    public var state: String

    public init(agent: String, title: String, project: String? = nil, state: String) {
        self.agent = agent
        self.title = title
        self.project = project
        self.state = state
    }
}

/// The daemon says only whether each key is set, and whether its provider refused it; a key never comes back.
public struct VoiceKeysStatus: Codable, Equatable {
    public struct Key: Codable, Equatable {
        public var set: Bool
        public var refused: Bool
        public init(set: Bool, refused: Bool = false) { self.set = set; self.refused = refused }
    }
    public var elevenlabs: Key
    public var volc: Key
    /// For speech recognition; nil from a daemon before it.
    public var groq: Key?
    public init(elevenlabs: Key, volc: Key, groq: Key? = nil) { self.elevenlabs = elevenlabs; self.volc = volc; self.groq = groq }

    /// Volcano Engine is no longer offered: only a user who already set a key for it sees its key row and voices,
    /// and can keep using or clear them.
    public var showsVolc: Bool { volc.set }

    /// The voices to offer: Volcano Engine's only to a user who still has a key for it, and `kept`, a voice already
    /// chosen, whatever its provider.
    public static func offered(_ voices: [CharacterVoice], keys: VoiceKeysStatus?, kept: String? = nil) -> [CharacterVoice] {
        voices.filter { $0.provider != "volc" || keys?.showsVolc == true || $0.id == kept }
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
    /// The account's plan (ADR-0023), when the daemon knows.
    public var plan: AccountPlan?
    /// The voice credit left, in millionths of a US dollar (ADR-0024); nil before the account first uses our voice
    /// service, when the service couldn't read it, and from older daemons.
    public var voiceCreditMicroUSD: Int?
    /// The one switch for the Paid plan: the service's answer, which the daemon reads with the plan. While it is false,
    /// the App doesn't offer the Paid plan: no Upgrade, no Free plan row, no Relay switch or refusals, and no making a
    /// Character with VibeBuddy's voices. False when signed out, and from older daemons and services.
    public var upgradeOffered: Bool
    enum CodingKeys: String, CodingKey {
        case available, signedIn = "signed_in", email, githubID = "github_id", githubLogin = "github_login", plan, voiceCreditMicroUSD = "voice_credit_micro_usd"
        case upgradeOffered = "upgrade_offered"
    }
    public init(available: Bool, signedIn: Bool, email: String? = nil, githubID: Int? = nil, githubLogin: String? = nil, plan: AccountPlan? = nil, voiceCreditMicroUSD: Int? = nil,
                upgradeOffered: Bool = false) {
        self.available = available; self.signedIn = signedIn; self.email = email; self.githubID = githubID; self.githubLogin = githubLogin
        self.plan = plan; self.voiceCreditMicroUSD = voiceCreditMicroUSD; self.upgradeOffered = upgradeOffered
    }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.init(available: try c.decode(Bool.self, forKey: .available), signedIn: try c.decode(Bool.self, forKey: .signedIn),
                  email: try c.decodeIfPresent(String.self, forKey: .email), githubID: try c.decodeIfPresent(Int.self, forKey: .githubID),
                  githubLogin: try c.decodeIfPresent(String.self, forKey: .githubLogin), plan: try c.decodeIfPresent(AccountPlan.self, forKey: .plan),
                  voiceCreditMicroUSD: try c.decodeIfPresent(Int.self, forKey: .voiceCreditMicroUSD),
                  upgradeOffered: try c.decodeIfPresent(Bool.self, forKey: .upgradeOffered) ?? false)
    }

    public enum PlanButton: Equatable { case upgrade, managePlan }

    /// The account card's plan button: Manage plan on Paid, Upgrade on Free while the Paid plan is offered, none
    /// otherwise.
    public var planButton: PlanButton? {
        guard signedIn, let plan else { return nil }
        if plan.paid { return .managePlan }
        return upgradeOffered ? .upgrade : nil
    }

    /// True when the plan row has a button: Manage plan on Paid, Upgrade on Free while the Paid plan is offered.
    public var showsPlanRow: Bool { planButton != nil }

    /// The Relay switch on General: while the Paid plan is offered, and always for a Paid account, so stopping sales
    /// doesn't take it from someone who pays for the Relay.
    public var showsRelaySwitch: Bool { upgradeOffered || (signedIn && plan?.paid == true) }

    /// The voice credit left in dollars, "$0.93", for the account card; nil when the daemon doesn't know it. Cut down
    /// to the cent, so it never shows more than is left.
    public var voiceCredit: String? {
        guard let micro = voiceCreditMicroUSD else { return nil }
        let cents = max(micro, 0) / 10_000
        return "$\(cents / 100)." + String(format: "%02d", cents % 100)
    }
}

/// `free`, or `paid` with the billing period's end.
public struct AccountPlan: Codable, Equatable {
    public var name: String
    public var periodEnd: Date?
    enum CodingKeys: String, CodingKey { case name, periodEnd = "period_end" }
    public init(name: String, periodEnd: Date? = nil) {
        self.name = name; self.periodEnd = periodEnd
    }
    public var paid: Bool { name == "paid" }
}

/// The daemon's answer to `POST /v1/account/upgrade` or `/v1/account/manage-plan` (docs/api.md).
public enum PlanPageAnswer: Equatable {
    /// The Stripe page to open in the browser.
    case open(URL)
    /// The service's 409 reason, `already_paid` or `no_subscription`: the plan isn't what the App showed, and the
    /// daemon has read the account again.
    case conflict(String)
    /// 404: Stripe isn't set up for this service, the Paid plan isn't offered, or the daemon is too old for the route.
    case notOffered
    /// 401: the status then shows signed out.
    case notSignedIn
    /// Anything else, with the daemon's message when it gave one.
    case failed(String?)

    public init(code: Int, data: Data) {
        let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        switch code {
        case 200:
            if let text = body?["url"] as? String, let url = URL(string: text) { self = .open(url) } else { self = .failed(nil) }
        case 409: self = .conflict(body?["reason"] as? String ?? "")
        case 404: self = .notOffered
        case 401: self = .notSignedIn
        default: self = .failed(body?["message"] as? String)
        }
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
    /// Which set of screens the box draws, `robot` or `korekore` (ADR-0021); nil from firmware that predates the choice, and from older daemons.
    public var screens: String?
    /// The decoration the box draws over its screens, `off`, `thanksgiving` or `christmas` (ADR-0022); nil from firmware that predates decorations, and from older daemons.
    public var decoration: String?
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
    /// The Wi-Fi network the box is on or trying now, and the address it got there. Absent from older daemons.
    public var wifiNetwork: String?
    /// Every network the box knows, most recently joined first (ADR-0023). Absent from older daemons.
    public var wifiNetworks: [String]?
    public var wifiAddress: String?
    /// Why the box couldn't join the network, until it does. Absent from older daemons.
    public var wifiError: String?
    /// What the box heard on its last scan, strongest first. Absent from older daemons.
    public var wifiSeen: [SeenNetwork]?
    /// The firmware takes updates over any link (ADR-0016): no cable, no download mode. Absent from older daemons.
    public var transfer: Bool?
    /// The last image sent over the link didn't confirm itself, and the box went back (`FIRMWARE ROLLED BACK`).
    public var rolledBack: Bool?
    /// Why the Relay won't carry the link (ADR-0023): `signed_out`, `plan` or `unavailable`. Absent from older daemons.
    public var relayRefused: String?
    /// The Relay this computer uses, and the one the box last said it uses (docs/relay-protocol.md, "Which Relay").
    /// Absent from older daemons; the box's also until the box has said.
    public var relayServer: String?
    public var boxRelay: String?
    enum CodingKeys: String, CodingKey { case transfer, rolledBack = "rolled_back", relayRefused = "relay_refused", relayServer = "relay_server", boxRelay = "box_relay", wifiNetwork = "wifi_network", wifiNetworks = "wifi_networks", wifiAddress = "wifi_address", wifiError = "wifi_error", wifiSeen = "wifi_seen", relay, busy, connected, port, bridge, network, mode, firmwareBuild = "firmware_build", firmwareVersion = "firmware_version", voice, volume, screens, decoration, foreignFirmware = "foreign_firmware", pin, candidates, unsupportedBoard = "unsupported_board", boxKey = "box_key", computerKey = "computer_key", pairedComputers = "paired_computers" }
    public init(connected: Bool, port: String? = nil, bridge: Bool = false, mode: String? = nil, firmwareBuild: String? = nil, firmwareVersion: String? = nil, voice: String? = nil, volume: Int? = nil, foreignFirmware: Bool? = nil) {
        self.connected = connected; self.port = port; self.bridge = bridge; self.mode = mode; self.firmwareBuild = firmwareBuild; self.firmwareVersion = firmwareVersion; self.voice = voice; self.volume = volume
        self.foreignFirmware = foreignFirmware
    }
}

/// Which Relay an address names: ours in production, ours in staging, or another, such as one under `wrangler dev`.
public enum RelayKind: Equatable {
    case production
    case staging
    case other(String)

    public init(_ address: String) {
        let host = URL(string: address.trimmingCharacters(in: .whitespaces))?.host ?? address
        switch host {
        case "relay.korekore.ai": self = .production
        case "relay-staging.korekore.ai": self = .staging
        default: self = .other(host)
        }
    }
}

extension DeviceState {
    /// The box uses another Relay than this computer, so the two can't meet through it. Unknown until the box has said.
    public var relayMismatch: Bool {
        guard let relayServer, let boxRelay else { return false }
        return RelayKind(relayServer) != RelayKind(boxRelay)
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
