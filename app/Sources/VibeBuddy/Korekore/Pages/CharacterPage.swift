import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// The Character page (canvas: "Mac · Character (sidebar)"; the write and Make-your-own states on "App · Device &
/// firmware states" and "App · Agents, Conversation, Character states"). What it does is the current Character
/// tab's (`VoicesView`, `RobotCard`, `CustomCharacterCard`, `AddressRow`), copied, in the korekore look.
struct KorekoreCharacterPage: View {
    @ObservedObject var model: AppModel
    @State private var makingCharacter = false

    private var connected: Bool { model.status?.device.connected ?? false }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                KorekoreVolumeCard(model: model)
                if let operation = model.operation, operation.kind == .voicePack {
                    writeProgress(operation)
                }
                if let making = model.status?.making {
                    KorekoreMakingRow(model: model, making: making)
                }
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12, alignment: .top), count: 3), spacing: 12) {
                    koreCard
                    robotCard
                    ForEach(Resources.bundledVoices) { entry in
                        voiceCard(entry)
                    }
                    ForEach(model.madeCharacters) { made in
                        KorekoreMadeCard(model: model, made: made)
                    }
                    makeYourOwn
                }
                .padding(.bottom, Theme.Sticker.sm)
                // Drawings of the user's own, kept or on the box: they can change them, the Character lending its voice, or
                // delete them.
                if model.status?.device.voice == CustomCharacterCard.id || model.savedCustomCharacter != nil {
                    KorekoreCustomLook(model: model)
                }
            }
            .padding(.bottom, 8)
        }
        .scrollIndicators(.never)
        .onAppear { model.refreshMadeCharacters() }
        .sheet(isPresented: $makingCharacter) { KorekoreMakeSheet(model: model) }
    }

    private func writeProgress(_ operation: DeviceOperation) -> some View {
        Group {
            switch operation.state {
            case .running:
                KCard {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            KHeading(String(localized: "Writing the character to the box…"))
                            Spacer()
                            KTag(operation.progress.formatted(.percent.precision(.fractionLength(0))), color: Theme.Colors.ink)
                        }
                        KBar(operation.progress)
                    }
                }
            case .failed:
                KCard(fill: Theme.Colors.cream) {
                    KNote(String(localized: "The character didn't finish writing, so the box keeps its built-in voice for now. Write it again when you're ready."))
                }
            case .done, .replug:
                EmptyView()
            }
        }
    }

    /// A Character card: the look on a dark stage 104 px high, the name, ▷ and Use, one line about who it is. The
    /// one the box wears has a 3 px `kore-pink` outline and IN USE.
    private func card<Stage: View>(name: String, summary: String, inUse: Bool, previewID: String?, use: (() -> Void)?,
                                   stageFill: Color = Theme.Colors.boxCard, @ViewBuilder stage: () -> Stage) -> some View {
        KCard(padding: EdgeInsets(top: 9, leading: 9, bottom: 9, trailing: 9),
              outline: inUse ? Theme.Colors.korePink : Theme.Colors.ink,
              stroke: inUse ? Theme.Stroke.bold : Theme.Stroke.base) {
            VStack(alignment: .leading, spacing: 6) {
                ZStack(alignment: .bottom) {
                    Color.clear
                    stage()
                }
                    .frame(maxWidth: .infinity)
                    .frame(height: 104)
                    .background(stageFill, in: RoundedRectangle(cornerRadius: Theme.Radius.sm))
                    .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.sm))
                HStack(spacing: 5) {
                    Text(verbatim: name).font(.rounded(16)).lineLimit(1)
                    Spacer(minLength: 4)
                    if inUse {
                        KTag(String(localized: "In use"), color: Theme.Colors.korePinkText)
                    } else {
                        if let previewID {
                            Button { model.togglePreview(previewID) } label: {
                                Image(systemName: model.previewingVoice == previewID ? "stop.fill" : "play.fill")
                                    .font(.system(size: 9, weight: .heavy))
                                    .frame(width: 24, height: 22)
                                    .foregroundStyle(Theme.Colors.ink)
                                    .background(Theme.Colors.paper, in: Capsule())
                                    .overlay(Capsule().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                            }
                            .buttonStyle(.plain)
                            .disabled(model.operationRunning)
                            .accessibilityLabel(String(localized: "Preview \(name)"))
                        }
                        if let use {
                            Button(String(localized: "Use"), action: use).kButton()
                                .disabled(model.operationRunning || !connected)
                        }
                    }
                }
                Text(verbatim: summary).font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                    .lineLimit(2, reservesSpace: true)
            }
        }
    }

    /// Kore becomes a Character of its own in a later change (ADR-0021); until then the card says so.
    private var koreCard: some View {
        KCard(padding: EdgeInsets(top: 9, leading: 9, bottom: 9, trailing: 9)) {
            VStack(alignment: .leading, spacing: 6) {
                Sprite(name: "kore-normal", scale: 4)
                    .frame(maxWidth: .infinity)
                    .frame(height: 104)
                    .background(Theme.Colors.ink, in: RoundedRectangle(cornerRadius: Theme.Radius.sm))
                HStack {
                    Text(verbatim: "Kore").font(.rounded(16))
                    Spacer()
                    KTag(String(localized: "Coming soon"))
                }
                Text("The box itself. “Kore kore!”").font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                    .lineLimit(2, reservesSpace: true)
            }
        }
    }

    private var robotCard: some View {
        card(name: "VibeBuddy", summary: String(localized: "The original robot, drawn by the box itself"),
             inUse: model.wearsRobot && !model.robotBorrowsVoice, previewID: AppModel.robotID, use: model.writeRobot) {
            if let face = Resources.robotFace {
                Image(nsImage: face).interpolation(.none).padding(.bottom, 8)
            }
        }
    }

    private func voiceCard(_ entry: VoiceCatalogEntry) -> some View {
        card(name: entry.name, summary: entry.summary, inUse: model.status?.device.voice == entry.id,
             previewID: entry.id, use: { model.writeVoice(entry.id) }) {
            if let frame = Resources.voicePack(entry.id)?.look.flatMap(LookBuilder.frames(of:))?.first {
                Image(nsImage: LookImages.image(frame, scale: 1)).interpolation(.none).padding(.bottom, 2)
            }
        }
    }

    /// One way in to a Character of the user's own: their drawings, their lines, a voice they pick. Drawings alone go
    /// straight to the box; new lines or another voice are made first (KorekoreMakeSheet).
    private var makeYourOwn: some View {
        Button { makingCharacter = true } label: {
            dashed(String(localized: "Make your own"), String(localized: "Your drawings, your lines, a voice you pick"))
        }
        .buttonStyle(.plain)
        .disabled(model.status == nil)
    }

    /// The dashed card that starts something new.
    private func dashed(_ title: String, _ detail: String) -> some View {
        VStack(spacing: 6) {
            Text(verbatim: "+").font(.pixel(32)).foregroundStyle(Theme.Colors.korePinkText)
            Text(verbatim: title).font(.rounded(14))
            Text(verbatim: detail).font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
        }
        .foregroundStyle(Theme.Colors.ink)
        .padding(9)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .overlay(RoundedRectangle(cornerRadius: Theme.Radius.md)
            .strokeBorder(Theme.Colors.ink, style: StrokeStyle(lineWidth: Theme.Stroke.base, dash: [6, 4])))
        .contentShape(Rectangle())
    }
}

/// Volume, and what the buddy calls you. The slider's value comes from the box's status and is sent on release; while
/// dragging, the status stream can't yank it back. Floor of 20: a saved zero would be a persistent mute.
struct KorekoreVolumeCard: View {
    @ObservedObject var model: AppModel
    @State private var level: Double = 65
    @State private var dragging = false

    private var connected: Bool { model.status?.device.connected ?? false }

    var body: some View {
        KRows {
            HStack(spacing: 14) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Text("Volume").font(.rounded(13))
                        Spacer()
                        Text(verbatim: "\(Int(level))").font(.pixel(13))
                    }
                    GeometryReader { geometry in
                        KBar((level - 20) / 80)
                            .contentShape(Rectangle())
                            .gesture(DragGesture(minimumDistance: 0)
                                .onChanged { drag in
                                    dragging = true
                                    let fraction = min(1, max(0, drag.location.x / geometry.size.width))
                                    // 20 to 100 in steps of 5, as the box takes it.
                                    level = 20 + (fraction * 80 / 5).rounded() * 5
                                }
                                .onEnded { _ in
                                    dragging = false
                                    model.setVolume(Int(level))
                                })
                    }
                    .frame(height: 12)
                }
                Button(String(localized: "Play on box")) { model.setVolume(Int(level), preview: true) }.kButton(.sun, size: 13)
            }
            .padding(.vertical, 10)
            .padding(.horizontal, 14)
            .disabled(!connected || model.operationRunning)
            KDivider()
            HStack(spacing: 10) {
                Text("What the buddy calls you").font(.rounded(13, weight: 700))
                Spacer()
                ForEach([VoiceLanguage.zh, .en], id: \.self) { language in
                    KMenu(selection: model.formOfAddress(language) ?? "",
                          options: [("", String(localized: "Nothing"))] + FormOfAddress.all.filter { $0.language == language }.map { ($0.id, $0.words) }) {
                        model.setFormOfAddress($0.isEmpty ? nil : $0, for: language)
                    }
                    .disabled(model.operationRunning)
                }
            }
            .padding(.vertical, 8)
            .padding(.horizontal, 14)
        }
        .onAppear(perform: sync)
        .onChange(of: model.status?.device.volume) { sync() }
    }

    private func sync() {
        guard !dragging, let volume = model.status?.device.volume else { return }
        level = Double(volume)
    }
}

/// A Character of the user's own: their drawings as the look, a preset Character's voice and lines (ADR-0009). The
/// same steps as `CustomCharacterCard`.
struct KorekoreCustomLook: View {
    @ObservedObject var model: AppModel
    @State private var frames: [RGBAImage] = []
    @State private var look: Data?
    @State private var lender = Resources.bundledVoices.first?.id ?? ""
    @State private var problem: String?

    private var inUse: Bool { model.status?.device.voice == CustomCharacterCard.id }

    var body: some View {
        KCard(stroke: Theme.Stroke.bold) {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    KHeading(String(localized: "Your own character"))
                    Spacer()
                    if inUse { KTag(String(localized: "In use"), color: Theme.Colors.korePinkText) }
                }
                HStack(alignment: .bottom, spacing: 14) {
                    ForEach(frames.indices, id: \.self) { index in
                        Image(nsImage: LookImages.image(frames[index], scale: 1)).interpolation(.none)
                    }
                }
                .frame(maxWidth: .infinity, minHeight: 80)
                .padding(12)
                .background(Theme.Colors.boxCard, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12)
                    .strokeBorder(Theme.Colors.ink, style: StrokeStyle(lineWidth: Theme.Stroke.base, dash: [6, 4])))
                KNote(String(localized: "One PNG or JPEG image: the figure four times in a 2 × 2 grid, on a plain white or transparent background. Top left normal, top right eyes closed, bottom left happy, bottom right sad. Any size: VibeBuddy cuts out each figure and makes it 48 × 64 pixels in 15 colors. No drawing yet? Copy a prompt into an image tool, such as ChatGPT or Gemini, with your character in place of the brackets."))
                HStack {
                    Button(String(localized: "Choose images…"), action: choose).kButton()
                    Button(String(localized: "Copy a prompt")) {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(CustomCharacterCard.prompt, forType: .string)
                    }
                    .kButton()
                }
                if !frames.isEmpty {
                    HStack {
                        KNote(String(localized: "Voice and lines from"))
                        Spacer()
                        KMenu(selection: lender, options: Resources.bundledVoices.map { ($0.id, $0.name) }) { lender = $0 }
                    }
                    HStack {
                        if model.savedCustomCharacter != nil {
                            Button(String(localized: "Delete…"), action: confirmDelete).kButton()
                        }
                        Spacer()
                        Button(String(localized: "Use this look"), action: use).kButton(.pink)
                            .disabled(model.operationRunning || !(model.status?.device.connected ?? false))
                    }
                }
                if let problem {
                    KTag(problem, color: Theme.Colors.korePinkText)
                }
            }
        }
        .onAppear {
            guard look == nil, let saved = model.savedCustomCharacter else { return }
            look = saved.look
            frames = LookBuilder.frames(of: saved.look) ?? []
            lender = saved.lender
        }
    }

    private func choose() {
        LookImages.chooseLook { result in
            switch result {
            case .success(let built):
                look = built
                frames = LookBuilder.frames(of: built) ?? []
                problem = nil
            case .failure(let failure):
                problem = failure.message
            case nil:
                break
            }
        }
    }

    private func confirmDelete() {
        let lenderName = VoiceCatalogEntry.all.first { $0.id == model.savedCustomCharacter?.lender }?.name ?? lender
        let answer = KorekoreAlert.run(title: String(localized: "Delete your own character? This can't be undone."),
                                       message: inUse ? String(localized: "The box goes back to \(lenderName).") : nil,
                                       buttons: [String(localized: "Delete"), String(localized: "Cancel")], warning: true)
        guard answer.button == 0 else { return }
        model.deleteCustomCharacter()
        look = nil
        frames = []
        problem = nil
    }

    private func use() {
        guard let look, model.writeCustomCharacter(look: look, lender: lender) else {
            problem = String(localized: "That character can't lend its voice; pick another.")
            return
        }
    }
}
