import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// The Character page (canvas: "Mac · Character (sidebar)"; the write and Make-your-own states on "App · Device &
/// firmware states" and "App · Agents, Conversation, Character states"). What it does is the current Character
/// tab's, in the korekore look.
struct KorekoreCharacterPage: View {
    @ObservedObject var model: AppModel
    @State private var makingOwn = false
    /// Bumped by Make your own look, so the page scrolls down to the card it opens below the grid.
    @State private var showOwn = 0
    @State private var makingCharacter = false

    private var connected: Bool { model.status?.device.connected ?? false }

    var body: some View {
        ScrollViewReader { proxy in
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
                        makeCharacter
                    }
                    .padding(.bottom, Theme.Sticker.sm)
                    if makingOwn || model.status?.device.voice == CustomCharacter.id {
                        KorekoreCustomLook(model: model).id("own")
                    }
                }
                .padding(.bottom, 8)
            }
            .scrollIndicators(.never)
            .onChange(of: showOwn) {
                // After the card is laid out, or there is nothing to scroll to yet.
                DispatchQueue.main.async { withAnimation { proxy.scrollTo("own", anchor: .top) } }
            }
        }
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

    private var makeYourOwn: some View {
        Button {
            makingOwn = true
            showOwn += 1
        } label: {
            dashed(String(localized: "Make your own look"), String(localized: "1–4 images, made pixel here"))
        }
        .buttonStyle(.plain)
    }

    private var makeCharacter: some View {
        Button { makingCharacter = true } label: {
            dashed(String(localized: "Make a Character"), String(localized: "Its own lines, in a voice you pick"))
        }
        .buttonStyle(.plain)
        .disabled(model.status == nil || model.status?.making?.running == true)
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

    private var inUse: Bool { model.status?.device.voice == CustomCharacter.id }

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
                KNote(String(localized: "Draw a figure with any image tool, on a plain white background: one image, or four in the order normal, eyes closed, happy, sad. The box shows it in place of the robot, with the voice and lines of the character you pick."))
                HStack {
                    Button(String(localized: "Choose images…"), action: choose).kButton()
                    Button(String(localized: "Copy a prompt")) {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(CustomCharacter.prompt, forType: .string)
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
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.allowedContentTypes = [.png, .jpeg]
        guard panel.runModal() == .OK else { return }
        let urls = panel.urls.sorted { $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedAscending }
        let images = urls.compactMap(LookImages.load)
        guard images.count == urls.count, (1...4).contains(images.count) else {
            problem = String(localized: "Pick one to four PNG or JPEG images.")
            return
        }
        do {
            let built = try LookBuilder.build(images)
            look = built
            frames = LookBuilder.frames(of: built) ?? []
            problem = nil
        } catch {
            problem = String(localized: "No figure found: use a plain white or transparent background.")
        }
    }

    private func use() {
        guard let look, model.writeCustomCharacter(look: look, lender: lender) else {
            problem = String(localized: "That character can't lend its voice; pick another.")
            return
        }
    }
}

/// The Character the user draws and names: its id in the daemon, and the prompt that the page copies for an image
/// model.
enum CustomCharacter {
    static let id = "custom"
    static let prompt = "Pixel art game sprite of [describe your character], chibi proportions, standing, front view, full body, centered, arms down, flat colors, thick dark outline, limited 16-color palette, plain solid white background. Then the same character in exactly the same pose with the eyes closed; with a big happy smile; with a sad face."
}
