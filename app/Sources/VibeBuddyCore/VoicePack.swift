import Foundation

/// 语音包（firmware/main/agent_voice_pack.h）：256 字节包头加五段 PCM。
/// App 只需要读出音色 id 和各句的位置，用来试听和显示。
public struct VoicePack: Equatable {
    public static let headerBytes = 256
    public static let clipNames = ["需要你确认", "任务完成", "任务遇到问题", "专注结束，休息一下", "休息结束"]

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

/// 五个候选音色的展示信息，与 voices/README.md 对应。
public struct VoiceCatalogEntry: Equatable, Identifiable {
    public var id: String
    public var name: String
    public var tag: String
    public init(id: String, name: String, tag: String) { self.id = id; self.name = name; self.tag = tag }

    public static let all: [VoiceCatalogEntry] = [
        VoiceCatalogEntry(id: "wanwanxiaohe", name: "湾湾小何", tag: "台湾口音 · 豆包语音 · 小智同款"),
        VoiceCatalogEntry(id: "xiaohe2", name: "小何 2.0", tag: "普通话 · 豆包语音"),
        VoiceCatalogEntry(id: "hsiaoyu", name: "晓雨", tag: "台湾口音 · 微软"),
        VoiceCatalogEntry(id: "hsiaochen", name: "晓臻", tag: "台湾口音 · 微软"),
        VoiceCatalogEntry(id: "xiaoxiao", name: "晓晓", tag: "普通话 · 微软"),
    ]
}
