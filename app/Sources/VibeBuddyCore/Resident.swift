import Foundation

/// `GET /v1/resident`: whether the daemon runs the experimental resident session (`VIBEBUDDY_RESIDENT_SESSION=1`).
public struct ResidentInfo: Decodable, Equatable {
    public var enabled: Bool
    public var threadId: String?
    /// Goes up when a voice command's turn ends or the thread changes, so the Conversation tab reads the history again
    /// only then. Kept in memory: it starts again at 0 when the daemon restarts. Nil from a daemon older than it.
    public var revision: UInt64?
    /// What voice commands are missing while the session is on but not ready (the user's Codex, installed and logged
    /// in, or the user's Claude Code with their Anthropic API key). Nil when ready, and from a daemon too old to say;
    /// a reason this App doesn't know is `.other`.
    public var problem: ResidentProblem?
    /// The Agent answering voice commands while ready: `codex` or `claude`. Nil when not ready, or from an older daemon.
    public var brain: String?
    /// The user's choice of that Agent; nil from a daemon older than the choice, or one this App doesn't know.
    public var brainSetting: ResidentBrainChoice?
    /// Whether the daemon holds an Anthropic API key; the key itself never comes back.
    public var anthropicKeySet: Bool?

    enum CodingKeys: String, CodingKey {
        case enabled
        case threadId = "thread_id"
        case revision
        case problem
        case brain
        case brainSetting = "brain_setting"
        case anthropicKeySet = "anthropic_key_set"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        enabled = try container.decode(Bool.self, forKey: .enabled)
        threadId = try container.decodeIfPresent(String.self, forKey: .threadId)
        revision = try container.decodeIfPresent(UInt64.self, forKey: .revision)
        problem = try container.decodeIfPresent(ResidentProblem.self, forKey: .problem)
        brain = try container.decodeIfPresent(String.self, forKey: .brain)
        brainSetting = (try? container.decodeIfPresent(String.self, forKey: .brainSetting)).flatMap { $0 }.flatMap(ResidentBrainChoice.init(rawValue:))
        anthropicKeySet = try container.decodeIfPresent(Bool.self, forKey: .anthropicKeySet)
    }
}

/// Which Agent answers voice commands: Codex when it is ready, else Claude Code when an Anthropic API key is set
/// (`auto`); or always the one picked.
public enum ResidentBrainChoice: String, CaseIterable, Equatable {
    case auto
    case codex
    case claude
}

/// Why the resident session can't take voice commands now: voice needs the user's Codex, or the user's Claude Code
/// with their own Anthropic API key.
public enum ResidentProblem: Equatable, Decodable {
    case codexMissing
    case codexLoggedOut
    case claudeMissing
    case anthropicKeyMissing
    case other(String)

    public init(from decoder: Decoder) throws {
        switch try decoder.singleValueContainer().decode(String.self) {
        case "codex_missing": self = .codexMissing
        case "codex_logged_out": self = .codexLoggedOut
        case "claude_missing": self = .claudeMissing
        case "anthropic_key_missing": self = .anthropicKeyMissing
        case let other: self = .other(other)
        }
    }
}

/// One voice command and the buddy's spoken replies to it, as `GET /v1/resident/history` reads them from the thread.
public struct ResidentExchange: Decodable, Equatable {
    public var said: String
    public var replies: [String]

    public init(said: String, replies: [String]) {
        self.said = said
        self.replies = replies
    }
}

/// `GET /v1/resident/history`: oldest first, so the newest exchange is the last one.
public struct ResidentHistory: Decodable, Equatable {
    public var threadId: String?
    public var exchanges: [ResidentExchange]

    enum CodingKeys: String, CodingKey {
        case threadId = "thread_id"
        case exchanges
    }
}
