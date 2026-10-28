import Foundation

/// Making a Character (ADR-0020, docs/characters.md): the user starts from a preset, edits its lines and picks a voice;
/// the daemon makes the line audio (`POST /v1/characters`), and the App packs it with a look.

/// A voice a made Character can speak with, from `GET /v1/voices`: a row of characters/voices.tsv, or one in the
/// user's own account (`own`), which only their own key speaks and which has no language of its own.
public struct CharacterVoice: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    /// `elevenlabs` or `volc`.
    public var provider: String
    public var voice: String
    public var language: String
    public var gender: String
    public var name: String
    public var own: Bool
    public init(id: String, provider: String, voice: String, language: String, gender: String, name: String, own: Bool = false) {
        self.id = id; self.provider = provider; self.voice = voice; self.language = language; self.gender = gender; self.name = name; self.own = own
    }

    private enum CodingKeys: String, CodingKey { case id, provider, voice, language, gender, name, own }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(id: try container.decode(String.self, forKey: .id), provider: try container.decode(String.self, forKey: .provider),
                  voice: try container.decode(String.self, forKey: .voice), language: try container.decode(String.self, forKey: .language),
                  gender: try container.decode(String.self, forKey: .gender), name: try container.decode(String.self, forKey: .name),
                  own: try container.decodeIfPresent(Bool.self, forKey: .own) ?? false)
    }

    public var voiceLanguage: VoiceLanguage? { VoiceLanguage(rawValue: language) }

    /// The Character whose preview is said in this voice: voices.tsv lists each preset's voice under the preset's id,
    /// and Jessica's is the robot's, the built-in lines.
    public var previewID: String { id == "jessica" ? "robot" : id }

    /// The provider as the UI names it.
    public var providerName: String {
        provider == "volc" ? String(localized: "Volcano Engine") : "ElevenLabs"
    }

    /// Voices in `language` first, then the rest of ours, then the user's own; the daemon's order otherwise kept.
    public static func sorted(_ voices: [CharacterVoice], preferring language: VoiceLanguage?) -> [CharacterVoice] {
        let ours = voices.filter { !$0.own }
        return ours.filter { $0.voiceLanguage == language } + ours.filter { $0.voiceLanguage != language } + voices.filter(\.own)
    }
}

public struct CharacterLine: Codable, Equatable, Sendable {
    public var occasion: String
    public var text: String
    public init(occasion: String, text: String) { self.occasion = occasion; self.text = text }
}

/// `POST /v1/characters`, and a made Character's manifest.json, which has no `id`.
public struct CharacterRequest: Codable, Equatable, Sendable {
    /// A Character made before, to make again (resume); nil for a new one.
    public var id: String?
    public var name: String
    /// The preset it started from.
    public var base: String
    /// An id from `GET /v1/voices`.
    public var voice: String
    public var lines: [CharacterLine]
    public init(id: String? = nil, name: String, base: String, voice: String, lines: [CharacterLine]) {
        self.id = id; self.name = name; self.base = base; self.voice = voice; self.lines = lines
    }
}

/// A Character made on this computer, from `GET /v1/characters`.
public struct MadeCharacter: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var base: String
    public var voice: String
    /// How many lines were asked for.
    public var lines: Int
    /// Its packs are made, so it can be written to the box.
    public var complete: Bool
    public init(id: String, name: String, base: String, voice: String, lines: Int, complete: Bool) {
        self.id = id; self.name = name; self.base = base; self.voice = voice; self.lines = lines; self.complete = complete
    }
}

/// `making` in the status: the Character being made, or the last one, since the daemon started.
public struct MakingStatus: Codable, Equatable, Sendable {
    public var id: String
    public var name: String
    /// `making`, `done` or `failed`; kept as text so a state added later doesn't break the whole status.
    public var state: String
    /// Pieces of audio made, of all there are.
    public var done: Int
    public var total: Int
    /// Why a failed job stopped: `plan`, `monthly`, `global_cap`, `signed_out`, `own_key`, `internal`, or nil with `error`.
    public var reason: String?
    public var error: String?
    public init(id: String, name: String, state: String, done: Int, total: Int, reason: String? = nil, error: String? = nil) {
        self.id = id; self.name = name; self.state = state; self.done = done; self.total = total; self.reason = reason; self.error = error
    }

    public var running: Bool { state == "making" }
    public var failed: Bool { state == "failed" }
    public var progress: Double { total > 0 ? Double(done) / Double(total) : 0 }

    /// Why it stopped, said for the user; `provider` names the voice's provider. Nil unless it failed.
    public func explanation(provider: String) -> String? {
        guard failed else { return nil }
        switch reason {
        case "plan": return String(localized: "Making a Character with VibeBuddy's voices needs the Paid plan, or your own key for \(provider).")
        case "monthly": return String(localized: "This month's Characters on the Paid plan are used up. The lines made so far are kept: resume next month, or with your own key for \(provider).")
        case "global_cap": return String(localized: "VibeBuddy's voices are paused for today. The lines made so far are kept: resume tomorrow.")
        case "signed_out": return String(localized: "Sign in on the Account tab, or set your own key for \(provider) there, then resume.")
        case "own_key": return String(localized: "\(provider) refused your own key. Set another one on the Account tab, then resume.")
        case "internal": return String(localized: "Making the Character stopped unexpectedly. Resume to try again.")
        default: return String(localized: "Making the Character failed: \(error ?? "")")
        }
    }
}

/// Whether a Character can be made in a voice, before starting: the daemon only finds out about the plan once the
/// voice service answers, so the App says what it can tell.
public enum MakingReadiness: Equatable, Sendable {
    /// The user's own key for the voice's provider: nothing else is needed.
    case ownKey
    /// Signed in without an own key: VibeBuddy's voices, which need the Paid plan.
    case paidPlan
    /// Not signed in and no own key: nothing can make it yet.
    case signIn
    /// No service to sign in to in this build, and no own key.
    case ownKeyOnly

    public var canStart: Bool { self == .ownKey || self == .paidPlan }

    public static func of(_ voice: CharacterVoice, keys: VoiceKeysStatus?, account: AccountStatus?) -> MakingReadiness {
        let key = voice.provider == "volc" ? keys?.volc : keys?.elevenlabs
        if let key, key.set, !key.refused { return .ownKey }
        // A voice in the user's own account: VibeBuddy's voice service can't speak it.
        if voice.own { return .ownKeyOnly }
        if account?.signedIn == true { return .paidPlan }
        return account?.available == true ? .signIn : .ownKeyOnly
    }

    public func explanation(provider: String) -> String {
        switch self {
        case .ownKey: return String(localized: "Made with your own key for \(provider), billed to your account there.")
        case .paidPlan: return String(localized: "Without your own key for \(provider), making a Character with VibeBuddy's voices needs the Paid plan.")
        case .signIn: return String(localized: "Sign in on the Account tab for the Paid plan, or set your own key for \(provider) there.")
        case .ownKeyOnly: return String(localized: "Set your own key for \(provider) on the Account tab first.")
        }
    }
}

/// The lines being edited before making: occasions stay as the preset has them, and each keeps at least one line.
public enum CharacterDraft {
    /// What a pack with a look has room for, and the daemon's limits.
    public static let maxLines = 110
    public static let maxText = 1000
    public static let maxName = 64

    /// Whether line `index` can go: not when it is the last of its occasion.
    public static func canRemove(_ index: Int, from lines: [CharacterLine]) -> Bool {
        lines.indices.contains(index) && lines.filter { $0.occasion == lines[index].occasion }.count > 1
    }

    /// What stops these from being made, said for the user; nil when they can be. `occasions` are the preset's.
    public static func problem(name: String, lines: [CharacterLine], occasions: [String]) -> String? {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty || trimmed.count > maxName { return String(localized: "Give the Character a name of up to 64 characters.") }
        if lines.count > maxLines { return String(localized: "A Character has at most 110 lines.") }
        for occasion in occasions where !lines.contains(where: { $0.occasion == occasion }) {
            return String(localized: "Keep at least one line for “\(CharacterOccasion.name(occasion))”.")
        }
        for line in lines {
            let text = line.text.trimmingCharacters(in: .whitespaces)
            if text.isEmpty { return String(localized: "A line for “\(CharacterOccasion.name(line.occasion))” is empty.") }
            if line.text.utf16.count > maxText || line.text.contains(where: { $0.isNewline || $0 == "\t" }) {
                return String(localized: "Each line is one row of at most 1000 characters.")
            }
            if line.text.replacingOccurrences(of: "{address}", with: "").contains(where: { $0 == "{" || $0 == "}" }) {
                return String(localized: "Only {address} goes in braces: it is where the form of address is said.")
            }
        }
        return nil
    }
}

/// The occasions a line is said on (docs/characters.md), as the line editor names them.
public enum CharacterOccasion {
    public static func name(_ id: String) -> String {
        switch id {
        case "input_required": return String(localized: "Needs input")
        case "done": return String(localized: "Done")
        case "failed": return String(localized: "Failed")
        case "focus_done": return String(localized: "Focus ended")
        case "break_done": return String(localized: "Break ended")
        case "first_done": return String(localized: "First done of the day")
        case "milestone": return String(localized: "Milestone")
        case "late_night_done": return String(localized: "Done late at night")
        case "late_night_input": return String(localized: "Needs input late at night")
        case "greeting_morning": return String(localized: "Morning greeting")
        case "greeting_afternoon": return String(localized: "Afternoon greeting")
        case "greeting_evening": return String(localized: "Evening greeting")
        case "long_session": return String(localized: "Long session")
        case "welcome_back": return String(localized: "Welcome back")
        default: return id
        }
    }
}

extension VoicePack {
    /// A made Character as the box gets it: the daemon's pack of its lines (`pack.bin`, no look), with the lines of a
    /// form of address (`address/<form>.bin`) in place of the plain ones, wearing `look`. The pack always carries the
    /// made Character's `id`, which the daemon gives a spoken reply its voice by. Nil if `pack` isn't a Character pack.
    public static func made(_ pack: Data, variant: Data?, look: Data?, id: String) -> Data? {
        guard var made = VoicePack(data: pack) else { return nil }
        if let variant = variant.flatMap(VoicePack.init(data:)), let data = made.withAddress(variant), let addressed = VoicePack(data: data) {
            made = addressed
        }
        guard let pools = made.pools else { return nil }
        return build(id: id, pools: pools, look: look)
    }
}
