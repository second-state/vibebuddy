import Foundation

/// Voice pack (firmware/main/agent_voice_pack.h): a 256-byte header plus five PCM clips.
/// The app only needs the voice id and each clip's location, for previews and display.
public struct VoicePack: Equatable {
    public static let headerBytes = 256

    public var voiceID: String
    public var clips: [Range<Int>]
    public var data: Data

    public init?(data: Data) {
        guard data.count > VoicePack.headerBytes, data.prefix(4) == Data("VBVP".utf8) else { return nil }
        func u32(_ offset: Int) -> Int {
            Int(data[offset]) | Int(data[offset + 1]) << 8 | Int(data[offset + 2]) << 16 | Int(data[offset + 3]) << 24
        }
        let idBytes = data[16..<48].prefix { $0 != 0 }
        guard let id = String(bytes: idBytes, encoding: .ascii), !id.isEmpty else { return nil }
        var clips: [Range<Int>] = []
        for index in 0..<5 {
            let offset = u32(48 + index * 4)
            let length = u32(68 + index * 4)
            guard offset >= VoicePack.headerBytes, offset + length <= data.count, length > 0 else { return nil }
            clips.append(offset..<(offset + length))
        }
        self.voiceID = id
        self.clips = clips
        self.data = data
    }

    /// The five clips joined with 300 ms of silence between them; this is what a preview plays.
    public func previewPCM() -> Data {
        var pcm = Data()
        let gap = Data(count: 24_000 * 2 * 2 * 3 / 10)
        for (index, clip) in clips.enumerated() {
            pcm.append(data.subdata(in: clip))
            if index + 1 < clips.count { pcm.append(gap) }
        }
        return pcm
    }
}

/// Display info for the candidate voices, matching voices/README.md.
/// `language` is the language the five lines are spoken in, so the picker can put
/// voices matching the UI language first. An entry only shows up once its pack is
/// bundled (build-app.sh packs every voices/<id>/ that has PCM in it).
/// The language a voice speaks its lines in, and the language the UI is shown in.
public enum VoiceLanguage: String, Equatable, Sendable {
    case zh, en
}

public struct VoiceCatalogEntry: Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var tag: String
    public var language: VoiceLanguage
    public init(id: String, name: String, tag: String, language: VoiceLanguage) {
        self.id = id; self.name = name; self.tag = tag; self.language = language
    }

    /// Built once: names and tags are localized for the language the process started in.
    public static let all: [VoiceCatalogEntry] = [
        VoiceCatalogEntry(id: "wanwanxiaohe", name: String(localized: "Wanwan Xiaohe"), tag: String(localized: "Chinese · Taiwanese accent · Doubao · same voice as Xiaozhi"), language: .zh),
        VoiceCatalogEntry(id: "xiaohe2", name: String(localized: "Xiaohe 2.0"), tag: String(localized: "Chinese · Mandarin · Doubao"), language: .zh),
        VoiceCatalogEntry(id: "jessica", name: "Jessica", tag: String(localized: "English · US · ElevenLabs"), language: .en),
        VoiceCatalogEntry(id: "chris", name: "Chris", tag: String(localized: "English · US · ElevenLabs"), language: .en),
    ]

    /// Voices in `language` first, catalog order otherwise preserved.
    public static func sorted(_ entries: [VoiceCatalogEntry], preferring language: VoiceLanguage) -> [VoiceCatalogEntry] {
        entries.filter { $0.language == language } + entries.filter { $0.language != language }
    }
}
