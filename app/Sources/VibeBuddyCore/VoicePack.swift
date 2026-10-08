import Foundation

/// A pack the box plays its lines from: a Character pack (firmware-rs/core/src/character_pack.rs,
/// a 1024-byte header plus ADPCM lines in pools per occasion), or an older voice pack (a 256-byte
/// header plus five PCM clips). The app only needs the id and a few lines, for previews and display.
public struct VoicePack: Equatable, Sendable {
    public static let headerBytes = 256
    public static let characterHeaderBytes = 1024

    public var voiceID: String
    /// The voice pack's five clips; empty for a Character pack.
    public var clips: [Range<Int>]
    /// A Character pack's preview: the first line of each ordinary occasion that has one, with its
    /// sample count. Empty for a voice pack.
    public var previewLines: [(bytes: Range<Int>, samples: Int)]
    /// A Character pack's look (format version 2), if it carries one.
    public var look: Data?
    public var data: Data

    public var isCharacter: Bool { !previewLines.isEmpty }

    public static func == (lhs: VoicePack, rhs: VoicePack) -> Bool {
        lhs.data == rhs.data
    }

    public init?(data: Data) {
        func u16(_ offset: Int) -> Int { Int(data[offset]) | Int(data[offset + 1]) << 8 }
        func u32(_ offset: Int) -> Int { u16(offset) | u16(offset + 2) << 16 }
        guard data.count > VoicePack.headerBytes else { return nil }
        let idBytes = data[16..<48].prefix { $0 != 0 }
        guard let id = String(bytes: idBytes, encoding: .ascii), !id.isEmpty else { return nil }
        var clips: [Range<Int>] = []
        var previewLines: [(bytes: Range<Int>, samples: Int)] = []
        var look: Data?
        switch data.prefix(4) {
        case Data("VBVP".utf8):
            for index in 0..<5 {
                let offset = u32(48 + index * 4)
                let length = u32(68 + index * 4)
                guard offset >= VoicePack.headerBytes, offset + length <= data.count, length > 0 else { return nil }
                clips.append(offset..<(offset + length))
            }
        case Data("VBCP".utf8):
            guard data.count > VoicePack.characterHeaderBytes, data[52] == 1 else { return nil }
            let occasions = Int(data[53])
            let lines = u16(54)
            // The five ordinary occasions come first in the table.
            for occasion in 0..<min(5, occasions) {
                let first = u16(56 + occasion * 4)
                guard u16(58 + occasion * 4) > 0, first < lines else { continue }
                let offset = u32(128 + first * 8)
                let samples = u32(132 + first * 8)
                let length = (samples + 1) / 2
                guard offset >= VoicePack.characterHeaderBytes, offset + length <= data.count, samples > 0 else { return nil }
                previewLines.append((offset..<(offset + length), samples))
            }
            guard !previewLines.isEmpty else { return nil }
            if u32(4) == 2, u32(1012) > 0 {
                let offset = u32(1008), length = u32(1012)
                guard offset >= VoicePack.characterHeaderBytes, offset + length <= data.count else { return nil }
                look = data.subdata(in: offset..<(offset + length))
            }
        default:
            return nil
        }
        self.voiceID = id
        self.clips = clips
        self.previewLines = previewLines
        self.look = look
        self.data = data
    }

    /// The five clips, or a Character's preview lines, joined with 300 ms of silence between them,
    /// as 24 kHz, 16-bit stereo PCM; this is what a preview plays.
    public func previewPCM() -> Data {
        let pieces = isCharacter
            ? previewLines.map { ADPCM.decode(data.subdata(in: $0.bytes), samples: $0.samples) }
            : clips.map { data.subdata(in: $0) }
        var pcm = Data()
        let gap = Data(count: 24_000 * 2 * 2 * 3 / 10)
        for (index, piece) in pieces.enumerated() {
            pcm.append(piece)
            if index + 1 < pieces.count { pcm.append(gap) }
        }
        return pcm
    }
}

/// The box's line decoding (firmware-rs/core/src/adpcm.rs), so a preview sounds like the box:
/// 16 kHz mono IMA ADPCM in, 24 kHz stereo 16-bit PCM out, by linear interpolation.
enum ADPCM {
    private static let steps: [Int] = [
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130,
        143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166,
        1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845,
        8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
    ]
    private static let indexChange: [Int] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8]

    static func decode(_ bytes: Data, samples: Int) -> Data {
        var predictor = 0
        var index = 0
        var input: [Int] = []
        input.reserveCapacity(samples)
        for byte in bytes {
            for nibble in [Int(byte & 15), Int(byte >> 4)] where input.count < samples {
                let step = steps[index]
                var diff = step >> 3
                if nibble & 4 != 0 { diff += step }
                if nibble & 2 != 0 { diff += step >> 1 }
                if nibble & 1 != 0 { diff += step >> 2 }
                predictor = min(32767, max(-32768, nibble & 8 != 0 ? predictor - diff : predictor + diff))
                index = min(88, max(0, index + indexChange[nibble]))
                input.append(predictor)
            }
        }
        let outputs = (samples * 3 + 1) / 2
        var pcm = Data(capacity: outputs * 4)
        for k in 0..<outputs {
            let at = k * 2
            let position = at / 3
            let current = input[min(position, input.count - 1)]
            let next = input[min(position + 1, input.count - 1)]
            let sample = Int16(current + (next - current) * (at % 3) / 3)
            withUnsafeBytes(of: sample.littleEndian) { pcm.append(contentsOf: $0); pcm.append(contentsOf: $0) }
        }
        return pcm
    }
}

/// Display info for the Characters the app ships, matching characters/ (docs/characters.md).
/// `language` is the language the five lines are spoken in, so the picker can put
/// voices matching the UI language first. An entry only shows up once its pack is
/// bundled (build-app.sh packs every voices/<id>/ that has PCM in it).
/// The language a voice speaks its lines in, and the language the UI is shown in.
public enum VoiceLanguage: String, Equatable, Sendable {
    case zh, en
}

/// The forms of address the buddy can call the user by, matching characters/addresses.tsv. Each is
/// synthesized in advance for every Character of its language, so the list is fixed.
public struct FormOfAddress: Equatable, Identifiable, Sendable {
    public var id: String
    /// The words spoken, shown as they are: they are the same in every UI language.
    public var words: String
    public var language: VoiceLanguage

    public static let all: [FormOfAddress] = [
        FormOfAddress(id: "laoban", words: "老板", language: .zh),
        FormOfAddress(id: "dalao", words: "大佬", language: .zh),
        FormOfAddress(id: "ge", words: "哥", language: .zh),
        FormOfAddress(id: "jie", words: "姐", language: .zh),
        FormOfAddress(id: "qin", words: "亲", language: .zh),
        FormOfAddress(id: "boss", words: "boss", language: .en),
        FormOfAddress(id: "captain", words: "captain", language: .en),
        FormOfAddress(id: "buddy", words: "buddy", language: .en),
    ]
}

public struct VoiceCatalogEntry: Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var tag: String
    /// Who the Character is, in one sentence, so the user can pick between them.
    public var summary: String
    public var language: VoiceLanguage
    public init(id: String, name: String, tag: String, summary: String, language: VoiceLanguage) {
        self.id = id; self.name = name; self.tag = tag; self.summary = summary; self.language = language
    }

    /// Built once: names, tags and summaries are localized for the language the process started in.
    public static let all: [VoiceCatalogEntry] = [
        VoiceCatalogEntry(id: "wanwanxiaohe", name: String(localized: "Xiaohe"), tag: String(localized: "Chinese · Taiwanese accent"),
                          summary: String(localized: "A sweet, lively friend from Taiwan who's always rooting for you."), language: .zh),
        VoiceCatalogEntry(id: "ada", name: "Ada", tag: String(localized: "English · British accent"),
                          summary: String(localized: "A witty Londoner. Understated praise, gently bossy about breaks."), language: .en),
        VoiceCatalogEntry(id: "hank", name: "Hank", tag: String(localized: "English"),
                          summary: String(localized: "A greybeard engineer who's seen every outage. Few words, dry praise."), language: .en),
        VoiceCatalogEntry(id: "luna", name: "Luna", tag: String(localized: "English"),
                          summary: String(localized: "Late-night lofi calm. Never rushes you."), language: .en),
        VoiceCatalogEntry(id: "kai", name: "Kai", tag: String(localized: "English · California accent"),
                          summary: String(localized: "A laid-back San Diego surfer and developer. Nothing stresses him out."), language: .en),
        VoiceCatalogEntry(id: "mei", name: "Mei", tag: String(localized: "English · Bay Area"),
                          summary: String(localized: "An upbeat Bay Area engineer who cheers you on, sometimes in Chinese."), language: .en),
    ]

    /// The language a box voice speaks, from the voice id the box reports; "builtin" is Jessica (English).
    /// nil for an id this catalog doesn't know.
    public static func language(ofVoice id: String) -> VoiceLanguage? {
        id == "builtin" ? .en : all.first { $0.id == id }?.language
    }

    /// The voice to offer when the UI switches to `language`: nil when the box already speaks it,
    /// its voice is unknown, or no voice in that language is bundled. Otherwise the first bundled
    /// voice in catalog order.
    public static func switchSuggestion(boxVoice: String, to language: VoiceLanguage, bundled: [VoiceCatalogEntry]) -> VoiceCatalogEntry? {
        guard let current = self.language(ofVoice: boxVoice), current != language else { return nil }
        return all.first { entry in entry.language == language && bundled.contains { $0.id == entry.id } }
    }

    /// Voices in `language` first, catalog order otherwise preserved.
    public static func sorted(_ entries: [VoiceCatalogEntry], preferring language: VoiceLanguage) -> [VoiceCatalogEntry] {
        entries.filter { $0.language == language } + entries.filter { $0.language != language }
    }
}
