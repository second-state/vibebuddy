import Foundation

/// 语音包（firmware/main/agent_voice_pack.h）：256 字节包头加五段 PCM。
/// App 只需要读出音色 id 和各句的位置，用来试听和显示。
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

    /// 五句连起来的 PCM，中间留 300 ms 静音；试听就放它。
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

/// 候选音色的展示信息，与 voices/README.md 对应。
/// `language` is the language the five lines are spoken in, so the picker can put
/// voices matching the UI language first. An entry only shows up once its pack is
/// bundled (build-app.sh packs every voices/<id>/ that has PCM in it).
public struct VoiceCatalogEntry: Equatable, Identifiable {
    public var id: String
    public var name: String
    public var tag: String
    public var language: String
    public init(id: String, name: String, tag: String, language: String) {
        self.id = id; self.name = name; self.tag = tag; self.language = language
    }

    public static var all: [VoiceCatalogEntry] {
        [
            VoiceCatalogEntry(id: "wanwanxiaohe", name: String(localized: "Wanwan Xiaohe"), tag: String(localized: "Chinese · Taiwanese accent · Doubao · same voice as Xiaozhi"), language: "zh"),
            VoiceCatalogEntry(id: "xiaohe2", name: String(localized: "Xiaohe 2.0"), tag: String(localized: "Chinese · Mandarin · Doubao"), language: "zh"),
            VoiceCatalogEntry(id: "hsiaoyu", name: String(localized: "HsiaoYu"), tag: String(localized: "Chinese · Taiwanese accent · Microsoft"), language: "zh"),
            VoiceCatalogEntry(id: "hsiaochen", name: String(localized: "HsiaoChen"), tag: String(localized: "Chinese · Taiwanese accent · Microsoft"), language: "zh"),
            VoiceCatalogEntry(id: "xiaoxiao", name: String(localized: "Xiaoxiao"), tag: String(localized: "Chinese · Mandarin · Microsoft"), language: "zh"),
            VoiceCatalogEntry(id: "jenny", name: "Jenny", tag: String(localized: "English · US · Microsoft"), language: "en"),
            VoiceCatalogEntry(id: "guy", name: "Guy", tag: String(localized: "English · US · Microsoft"), language: "en"),
        ]
    }

    /// Voices in `language` first, catalog order otherwise preserved.
    public static func sorted(_ entries: [VoiceCatalogEntry], preferring language: String) -> [VoiceCatalogEntry] {
        entries.filter { $0.language == language } + entries.filter { $0.language != language }
    }
}
