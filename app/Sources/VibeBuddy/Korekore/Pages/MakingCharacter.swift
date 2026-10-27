import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// A Character the user made (ADR-0020), as a Character card: its look, its name, Use or Resume, and Delete. The same
/// logic as `MadeCharacterCard`, in the korekore look; the design has no card of its own for it.
struct KorekoreMadeCard: View {
    @ObservedObject var model: AppModel
    let made: MadeCharacter

    private var inUse: Bool { model.status?.device.voice == made.id }
    private var making: Bool { model.status?.making?.running == true }
    private var connected: Bool { model.status?.device.connected ?? false }

    var body: some View {
        KCard(padding: EdgeInsets(top: 9, leading: 9, bottom: 9, trailing: 9),
              outline: inUse ? Theme.Colors.korePink : Theme.Colors.ink,
              stroke: inUse ? Theme.Stroke.bold : Theme.Stroke.base) {
            VStack(alignment: .leading, spacing: 6) {
                ZStack(alignment: .bottom) {
                    Color.clear
                    if let frame = model.look(of: made).flatMap(LookBuilder.frames(of:))?.first {
                        Image(nsImage: LookImages.image(frame, scale: 1)).interpolation(.none).padding(.bottom, 2)
                    }
                }
                .frame(maxWidth: .infinity)
                .frame(height: 104)
                .background(Theme.Colors.boxCard, in: RoundedRectangle(cornerRadius: Theme.Radius.sm))
                .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.sm))
                HStack(spacing: 5) {
                    Text(verbatim: made.name).font(.rounded(16)).lineLimit(1)
                    Spacer(minLength: 4)
                    if inUse {
                        KTag(String(localized: "In use"), color: Theme.Colors.korePinkText)
                    } else if made.complete {
                        Button(String(localized: "Use")) { model.writeMadeCharacter(made) }.kButton()
                            .disabled(model.operationRunning || !connected)
                    } else if making && model.status?.making?.id == made.id {
                        KTag(String(localized: "Making…"), color: Theme.Colors.waitText)
                    } else {
                        Button(String(localized: "Resume")) { model.resumeCharacter(made.id) }.kButton()
                            .disabled(making)
                    }
                }
                Text(verbatim: detail).font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                    .lineLimit(2, reservesSpace: true)
                HStack {
                    Spacer()
                    Button(String(localized: "Delete…")) { confirmDelete() }.kButton()
                        .disabled(inUse || (making && model.status?.making?.id == made.id))
                        .help(inUse ? String(localized: "It's on the box: put another character on the box to delete this one.") : "")
                }
            }
        }
    }

    private var detail: String {
        let base = VoiceCatalogEntry.all.first { $0.id == made.base }?.name ?? made.base
        let voice = model.characterVoice(made.voice)?.name ?? made.voice
        return made.complete
            ? String(localized: "Made from \(base) · voice: \(voice)")
            : String(localized: "Not finished · made from \(base) · voice: \(voice)")
    }

    private func confirmDelete() {
        let answer = KorekoreAlert.run(title: String(localized: "Delete “\(made.name)”? This can't be undone."),
                                       message: String(localized: "Its lines made in its voice go with it; making it again would make them again."),
                                       buttons: [String(localized: "Delete"), String(localized: "Cancel")], warning: true)
        if answer.button == 0 { model.deleteMadeCharacter(made) }
    }
}

/// The Character being made, or the last one: its progress, why it stopped, or that it is ready for the box.
struct KorekoreMakingRow: View {
    @ObservedObject var model: AppModel
    let making: MakingStatus

    private var made: MadeCharacter? { model.madeCharacters.first { $0.id == making.id } }

    var body: some View {
        if making.running {
            KCard {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        KHeading(String(localized: "Making “\(making.name)”…"))
                        Spacer()
                        KTag("\(making.done) / \(making.total)", color: Theme.Colors.ink)
                    }
                    KBar(making.progress, fill: Theme.Colors.sun)
                }
            }
        } else if let why = making.explanation(provider: provider) {
            KCard(fill: Theme.Colors.cream, outline: Theme.Colors.korePinkText) {
                HStack(alignment: .top, spacing: 12) {
                    KNote(why, color: Theme.Colors.ink).frame(maxWidth: .infinity, alignment: .leading)
                    if made != nil {
                        Button(String(localized: "Resume")) { model.resumeCharacter(making.id) }.kButton(.sun)
                    }
                }
            }
        } else if let made, made.complete, model.status?.device.voice != made.id {
            KCard(outline: Theme.Colors.doneText) {
                HStack(spacing: 12) {
                    KTag(String(localized: "“\(made.name)” is made."), color: Theme.Colors.doneText)
                    Spacer()
                    Button(String(localized: "Write to box")) { model.writeMadeCharacter(made) }.kButton(.pink)
                        .disabled(model.operationRunning || !(model.status?.device.connected ?? false))
                }
            }
        }
    }

    private var provider: String {
        made.flatMap { model.characterVoice($0.voice) }?.providerName ?? String(localized: "the voice's provider")
    }
}

/// Making a Character of the user's own: bring a look, pick a voice, and take a preset's lines to change (occasions stay,
/// each keeps a line). With only a look, nothing is made: the drawings go to the box with the preset lending its
/// voice and lines, as `CustomCharacterCard` does. With new lines or another voice, the daemon makes the line audio, as
/// `MakeCharacterSheet` does.
struct KorekoreMakeSheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    private struct Draft: Identifiable {
        let id = UUID()
        var occasion: String
        var text: String
    }

    @State private var base = Resources.bundledVoices.first?.id ?? ""
    @State private var name = ""
    @State private var voice = ""
    @State private var lines: [Draft] = []
    /// The preset's lines as they came, to tell whether any changed.
    @State private var original: [CharacterLine] = []
    /// The preset's occasions, in its order.
    @State private var occasions: [String] = []
    /// The user's own look; nil wears the base's.
    @State private var look: Data?
    @State private var problem: String?
    @State private var starting = false

    private var baseName: String { VoiceCatalogEntry.all.first { $0.id == base }?.name ?? base }
    private var chosenVoice: CharacterVoice? { model.characterVoice(voice) }
    private var edited: [CharacterLine] { lines.map { CharacterLine(occasion: $0.occasion, text: $0.text) } }
    private var draftProblem: String? { lines.isEmpty ? nil : CharacterDraft.problem(name: name, lines: edited, occasions: occasions) }
    /// Only the look is new: nothing to make, the preset lends its voice and lines.
    private var lookOnly: Bool { look != nil && edited == original && voice == base }
    private var readiness: MakingReadiness? {
        chosenVoice.map { MakingReadiness.of($0, keys: model.status?.voiceKeys, account: model.status?.account) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Make your own").font(.rounded(18))
            KNote(String(localized: "Bring your own drawings, pick a voice and say what it says. A character's own voice with its lines as they are is free: they are already made. Any other voice, or lines you change, VibeBuddy makes in that voice, which needs the Paid plan or your own key."))
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    KRows {
                        KRow(String(localized: "Name")) {
                            KField(String(localized: "Name"), text: $name).frame(width: 240)
                        }
                    }
                    KHeading(String(localized: "Look"))
                    KRows {
                        HStack(alignment: .top, spacing: 14) {
                            if let frame = (look ?? Resources.voicePack(base)?.look).flatMap(LookBuilder.frames(of:))?.first {
                                Image(nsImage: LookImages.image(frame, scale: 1)).interpolation(.none)
                                    .padding(4)
                                    .background(Theme.Colors.boxCard, in: RoundedRectangle(cornerRadius: Theme.Radius.sm))
                            }
                            VStack(alignment: .leading, spacing: 10) {
                                KNote(String(localized: "One to four PNG or JPEG images of the figure on a plain white or transparent background, in the order normal, eyes closed, happy, sad. Any size: VibeBuddy cuts out the figure and makes it 48 × 64 pixels in 15 colors. No drawings yet? Copy a prompt into an image tool, such as ChatGPT or Gemini, with your character in place of the brackets."))
                                HStack(spacing: 8) {
                                    Button(String(localized: "Choose images…")) { chooseLook() }.kButton()
                                    Button(String(localized: "Copy a prompt")) {
                                        NSPasteboard.general.clearContents()
                                        NSPasteboard.general.setString(CustomCharacterCard.prompt, forType: .string)
                                    }
                                    .kButton()
                                    if look != nil {
                                        Button(String(localized: "Use \(baseName)'s look")) { look = nil }.kButton()
                                    }
                                }
                            }
                        }
                        .padding(.vertical, 12)
                        .padding(.horizontal, 12)
                    }
                    KHeading(String(localized: "Voice"))
                    KRows {
                        // In one order whatever the lines: the voice comes first, so the lines don't move it.
                        let options = model.characterVoices
                        ForEach(Array(options.enumerated()), id: \.element.id) { index, option in
                            if index > 0 { KDivider() }
                            voiceRow(option)
                        }
                    }
                    if !lookOnly, let readiness, let chosenVoice {
                        KNote(readiness.explanation(provider: chosenVoice.providerName),
                              color: readiness.canStart ? Theme.Colors.muted : Theme.Colors.korePinkText)
                    }
                    KHeading(String(localized: "Lines"))
                    KRows {
                        KRow(String(localized: "Lines from")) {
                            KMenu(selection: base, options: Resources.bundledVoices.map { ($0.id, $0.name) }) { base = $0 }
                        }
                    }
                    ForEach(occasions, id: \.self) { occasion in
                        KHeading(CharacterOccasion.name(occasion))
                        KRows {
                            let rows = lines.filter { $0.occasion == occasion }
                            ForEach(Array(rows.enumerated()), id: \.element.id) { index, line in
                                if index > 0 { KDivider() }
                                HStack(spacing: 8) {
                                    KField(CharacterOccasion.name(occasion), text: text(of: line.id))
                                    Button { lines.removeAll { $0.id == line.id } } label: { Text(verbatim: "−").frame(width: 14) }
                                        .kButton()
                                        .disabled(!CharacterDraft.canRemove(lines.firstIndex { $0.id == line.id } ?? -1, from: edited))
                                        .help(String(localized: "Remove this line"))
                                }
                                .padding(.vertical, 8)
                                .padding(.horizontal, 12)
                            }
                        }
                    }
                    KNote(String(localized: "{address} is where the buddy says what it calls you; each line with it is made once with each form of address."))
                }
                .padding(.bottom, Theme.Sticker.sm + 4)
            }
            .scrollIndicators(.never)
            if let shown = problem ?? (lookOnly ? nil : draftProblem) {
                KTag(shown, color: Theme.Colors.korePinkText)
            }
            HStack {
                Spacer()
                Button(String(localized: "Cancel")) { dismiss() }.kButton().keyboardShortcut(.cancelAction)
                if lookOnly {
                    Button(String(localized: "Use this look")) { useLook() }.kButton(.pink)
                        .keyboardShortcut(.defaultAction)
                        .disabled(model.operationRunning || !(model.status?.device.connected ?? false))
                } else {
                    Button(String(localized: "Make")) { make() }.kButton(.sun)
                        .keyboardShortcut(.defaultAction)
                        .disabled(starting || lines.isEmpty || draftProblem != nil || !(readiness?.canStart ?? false) || model.status?.making?.running == true)
                }
            }
        }
        .foregroundStyle(Theme.Colors.ink)
        .padding(20)
        .frame(width: 640, height: 680)
        .background(Theme.Colors.cream)
        .onAppear { if model.characterVoices.isEmpty { model.refreshMadeCharacters() } }
        .onDisappear { if model.previewingVoice != nil { model.preview.stop() } }
        .task(id: base) { await load() }
    }

    /// A row per voice, so each has its play button: the preview of the preset it comes from.
    private func voiceRow(_ option: CharacterVoice) -> some View {
        let chosen = voice == option.id
        return HStack(spacing: 10) {
            Button { voice = option.id } label: {
                HStack(spacing: 8) {
                    Circle()
                        .fill(chosen ? Theme.Colors.korePink : Theme.Colors.paper)
                        .overlay(Circle().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                        .frame(width: 16, height: 16)
                    Text(verbatim: option.name).font(.rounded(14))
                    KTag(option.providerName)
                    if option.id == base && edited == original {
                        KTag(String(localized: "Free · already made"), color: Theme.Colors.doneText)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(chosen ? .isSelected : [])
            Spacer()
            if Resources.voicePack(option.previewID) != nil {
                let playing = model.previewingVoice == option.previewID
                Button { model.togglePreview(option.previewID) } label: {
                    Image(systemName: playing ? "stop.fill" : "play.fill").font(.system(size: 9, weight: .heavy)).frame(width: 14)
                }
                .kButton()
                .disabled(model.operationRunning)
                .help(playing ? String(localized: "Stop") : String(localized: "Hear this voice"))
            }
        }
        .padding(.vertical, 8)
        .padding(.horizontal, 12)
    }

    private func text(of id: UUID) -> Binding<String> {
        Binding(get: { lines.first { $0.id == id }?.text ?? "" },
                set: { new in if let index = lines.firstIndex(where: { $0.id == id }) { lines[index].text = new } })
    }

    /// The base's lines; picking another base starts them over. The voice starts as the base's and stays as the user picks it.
    private func load() async {
        if voice.isEmpty { voice = base }
        do {
            let fetched = try await model.client.presetLines(base)
            lines = fetched.map { Draft(occasion: $0.occasion, text: $0.text) }
            original = fetched
            occasions = fetched.reduce(into: []) { seen, line in if !seen.contains(line.occasion) { seen.append(line.occasion) } }
            problem = nil
        } catch {
            lines = []
            original = []
            occasions = []
            problem = String(localized: "Couldn't read the lines of \(baseName): \(error.localizedDescription)")
        }
    }

    private func chooseLook() {
        LookImages.chooseLook { result in
            switch result {
            case .success(let built):
                look = built
                problem = nil
            case .failure(let failure):
                problem = failure.message
            case nil:
                break
            }
        }
    }

    /// The drawings go to the box as the user's own Character, the preset lending its voice and lines.
    private func useLook() {
        guard let look, model.writeCustomCharacter(look: look, lender: base) else {
            problem = String(localized: "That character can't lend its voice; pick another.")
            return
        }
        dismiss()
    }

    private func make() {
        starting = true
        let request = CharacterRequest(name: name.trimmingCharacters(in: .whitespacesAndNewlines), base: base, voice: voice, lines: edited)
        Task {
            problem = await model.makeCharacter(request, look: look)
            starting = false
            if problem == nil { dismiss() }
        }
    }
}
