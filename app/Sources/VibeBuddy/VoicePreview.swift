import AVFoundation
import Foundation
import VibeBuddyCore

/// 试听：语音包里的 PCM 是 24 kHz、16-bit、双声道，直接喂给 AVAudioEngine。
@MainActor
final class VoicePreview {
    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private(set) var playingVoice: String?
    var onFinish: (() -> Void)?

    init() {
        engine.attach(player)
    }

    func toggle(pack: VoicePack) {
        if playingVoice == pack.voiceID {
            stop()
            return
        }
        stop()
        let pcm = pack.previewPCM()
        let format = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 24_000, channels: 2, interleaved: true)!
        let frames = AVAudioFrameCount(pcm.count / 4)
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames) else { return }
        buffer.frameLength = frames
        pcm.withUnsafeBytes { raw in
            buffer.int16ChannelData![0].update(from: raw.bindMemory(to: Int16.self).baseAddress!, count: Int(frames) * 2)
        }
        engine.connect(player, to: engine.mainMixerNode, format: format)
        do {
            try engine.start()
        } catch {
            return
        }
        playingVoice = pack.voiceID
        player.scheduleBuffer(buffer, at: nil, options: []) { [weak self] in
            Task { @MainActor in
                guard let self, self.playingVoice == pack.voiceID else { return }
                self.playingVoice = nil
                self.onFinish?()
            }
        }
        player.play()
    }

    func stop() {
        player.stop()
        engine.stop()
        playingVoice = nil
        onFinish?()
    }
}
