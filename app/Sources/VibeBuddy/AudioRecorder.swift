import AVFoundation
import SwiftUI
import UniformTypeIdentifiers
import VibeBuddyCore

/// The XGGF recorder: an explicit, bounded recording from the device's microphone, followed here,
/// then played or saved on this computer. The daemon offers it only when this computer granted
/// audio, and nothing is kept anywhere by default. Laid out neutrally so the Device tab's form and
/// the korekore page's card can both carry it.
struct AudioRecorder: View {
    @ObservedObject var model: AppModel

    @State private var rate = 16000
    @State private var seconds: Double = 5
    @State private var recording = false
    @State private var frames: Int?
    @State private var lost = 0
    @State private var result: Data?
    @State private var problem: String?
    /// Kept so the sound keeps playing; AVAudioPlayer stops the moment it is released.
    @State private var player: AVAudioPlayer?

    private var expected: Int { rate / 50 * Int(seconds) }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if recording {
                LabeledContent("Status", value: progress)
                Button("Stop") {
                    Task { try? await model.client.stopAudio() }
                }
            } else {
                LabeledContent("Sample rate") {
                    Picker("Sample rate", selection: $rate) {
                        Text(verbatim: "8 kHz").tag(8000)
                        Text(verbatim: "16 kHz").tag(16000)
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .frame(width: 160)
                }
                LabeledContent(String(localized: "Length: \(Int(seconds)) s")) {
                    Slider(value: $seconds, in: 1...60, step: 1).frame(width: 220)
                }
                Button("Record") { start() }
            }
            if result != nil {
                HStack {
                    Button("Play") { play() }
                    Button("Save…") { save() }
                    Button("Discard", role: .destructive) { discard() }
                }
            }
            if let problem {
                Text(problem).font(.caption).foregroundStyle(.red)
            }
        }
        // Follows the recording until it completes or ends; leaving the page cancels it.
        .task(id: recording) { await follow() }
    }

    private var progress: String {
        guard let frames else { return String(localized: "Starting…") }
        let base = String(localized: "\(frames) of \(expected) frames")
        return lost > 0 ? "\(base)\(String(localized: ", \(lost) lost"))" : base
    }

    private func start() {
        problem = nil
        result = nil
        frames = nil
        Task {
            do {
                try await model.client.startAudio(sampleRate: rate, durationMs: Int(seconds) * 1000)
                recording = true
            } catch {
                problem = (error as? DaemonError)?.message ?? error.localizedDescription
            }
        }
    }

    private func follow() async {
        guard recording else { return }
        while !Task.isCancelled {
            do {
                switch try await model.client.pollAudio() {
                case .done(let wav):
                    result = wav
                    recording = false
                    frames = nil
                    return
                case .recording(let arrived, let missing):
                    frames = arrived
                    lost = missing
                case .ended(let reason):
                    recording = false
                    frames = nil
                    // A nil reason, or "no completed recording", just means nothing is armed.
                    if let reason, !reason.hasPrefix("no completed recording") {
                        problem = reason
                    }
                    return
                }
            } catch {
                recording = false
                problem = (error as? DaemonError)?.message ?? error.localizedDescription
                return
            }
            try? await Task.sleep(for: .milliseconds(500))
        }
    }

    private func play() {
        guard let result else { return }
        do {
            player = try AVAudioPlayer(data: result)
            player?.play()
        } catch {
            problem = error.localizedDescription
        }
    }

    private func save() {
        guard let result else { return }
        let panel = NSSavePanel()
        panel.title = String(localized: "Save the recording")
        panel.allowedContentTypes = [.wav]
        panel.nameFieldStringValue = "recording.wav"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try result.write(to: url)
        } catch {
            problem = error.localizedDescription
        }
    }

    private func discard() {
        player?.stop()
        player = nil
        result = nil
        frames = nil
        problem = nil
        Task { try? await model.client.discardAudio() }
    }
}
