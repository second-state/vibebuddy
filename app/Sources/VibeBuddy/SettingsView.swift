import SwiftUI
import VibeBuddyCore

struct SettingsView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        TabView {
            GeneralView(model: model).tabItem { Label("General", systemImage: "gearshape") }
            VoicesView(model: model).tabItem { Label("Character", systemImage: "person.crop.square") }
            HooksView(model: model).tabItem { Label("Agents", systemImage: "link") }
            DeviceView(model: model).tabItem { Label("Device", systemImage: "cpu") }
            AdvancedView(model: model).tabItem { Label("Advanced", systemImage: "wrench.and.screwdriver") }
        }
        .padding(20)
        .frame(minWidth: 640, maxWidth: .infinity, minHeight: 480, maxHeight: .infinity)
    }
}

struct GeneralView: View {
    @ObservedObject var model: AppModel
    @State private var language = AppLanguage.current

    var body: some View {
        Form {
            // Nothing stops working; the App just says, every time, that it's too old (ADR-0010).
            if model.updates?.unsupportedApp == true {
                Label("This version of Vibe Buddy is no longer supported. Update it to keep getting firmware for the box.", systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
            }
            // The dialog runs after this update rather than as a nested modal loop inside SwiftUI's binding setter.
            Picker(selection: Binding(get: { language }, set: { choice in DispatchQueue.main.async { change(to: choice) } })) {
                Text("System").tag(AppLanguage.system)
                Text(verbatim: "English").tag(AppLanguage.en)
                Text(verbatim: "简体中文").tag(AppLanguage.zhHans)
            } label: {
                Text("Language")
            }
            Toggle("Launch at login", isOn: Binding(get: { model.launchAtLogin }, set: { model.setLaunchAtLogin($0) }))
            Toggle("Notify me when the box disconnects or the daemon fails", isOn: Binding(get: { model.status?.config.notifyLink ?? true }, set: { model.setNotifyLink($0) }))
                .disabled(model.status == nil)
            Section("Updates") {
                Toggle("Check for updates", isOn: Binding(get: { model.status?.config.checkUpdates ?? model.updates?.enabled ?? false }, set: { model.setCheckUpdates($0) }))
                    .disabled(model.status == nil)
                HStack {
                    Text(updateSummary).foregroundStyle(.secondary)
                    Spacer()
                    if let app = model.updates?.app {
                        if model.updater.available {
                            Button("Install \(app.version)…") { model.installApp(app) }
                        } else {
                            Button("Download \(app.version)") { model.installApp(app) }
                        }
                    }
                    Button("Check now") { model.checkForUpdates() }.disabled(!(model.updates?.enabled ?? false))
                }
            }
            Section {
                LabeledContent("App", value: Resources.displayVersion)
                LabeledContent("daemon", value: model.status?.daemon.build ?? String(localized: "Not connected"))
            }
        }
        .formStyle(.grouped)
    }

    private var updateSummary: String {
        guard let updates = model.updates, updates.enabled else { return String(localized: "Off") }
        if let error = updates.error { return String(localized: "Last check failed: \(error)") }
        if let app = updates.app { return String(localized: "Vibe Buddy \(app.version) is available") }
        guard let checked = updates.lastCheck else { return String(localized: "Not checked yet") }
        return String(localized: "Up to date · checked \(checked.formatted(.relative(presentation: .named)))")
    }

    /// Saves the choice, then offers to restart and, when the box speaks the other language, to switch its voice too.
    private func change(to choice: AppLanguage) {
        guard choice != language else { return }
        choice.save()
        language = choice
        let target = choice.resolved
        guard target != Resources.uiLanguage else { return }
        let alert = NSAlert()
        alert.messageText = String(localized: "Restart Vibe Buddy to change the language?")
        alert.informativeText = String(localized: "The box goes offline for a few seconds while the app restarts.")
        var voiceSwitch: (entry: VoiceCatalogEntry, checkbox: NSButton)?
        let device = model.status?.device
        // Only a box that has answered (build and voice reported) can take a voice; an unknown voice isn't guessed at.
        if let device, device.connected, device.firmwareBuild != nil, let voice = device.voice,
           let entry = VoiceCatalogEntry.switchSuggestion(boxVoice: voice, to: target, bundled: Resources.bundledVoices) {
            let title = device.bridge
                ? String(localized: "Also switch the box's voice to \(entry.name) (a few minutes over the UART bridge; keep it plugged in)")
                : String(localized: "Also switch the box's voice to \(entry.name)")
            let checkbox = NSButton(checkboxWithTitle: title, target: nil, action: nil)
            checkbox.state = .on
            alert.accessoryView = checkbox
            voiceSwitch = (entry, checkbox)
        } else if !(device?.connected ?? false) {
            alert.informativeText += "\n" + String(localized: "The box isn't connected, so its character stays as it is. You can change it later on the Character tab.")
        }
        alert.addButton(withTitle: String(localized: "Restart now"))
        alert.addButton(withTitle: String(localized: "Later"))
        let restart = alert.runModal() == .alertFirstButtonReturn
        let voice = voiceSwitch.flatMap { $0.checkbox.state == .on ? $0.entry.id : nil }
        if restart {
            model.restart(writingVoice: voice)
        } else if let voice {
            model.writeVoice(voice)
        }
    }
}

/// Volume slider: the value comes from the box's status and is sent on release; while dragging, the status stream can't yank it back.
/// Floor of 20: a saved zero volume would be a persistent mute, and muting is deliberately box-only and not persisted.
struct VolumeRow: View {
    @ObservedObject var model: AppModel
    @State private var level: Double = 65
    @State private var editing = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Volume").font(.headline)
                Spacer()
                Text("\(Int(level))").monospacedDigit().foregroundStyle(.secondary)
            }
            HStack(spacing: 12) {
                Slider(value: $level, in: 20...100, step: 5) { isEditing in
                    editing = isEditing
                    if !isEditing { model.setVolume(Int(level)) }
                }
                Button("Play a line on the box") { model.setVolume(Int(level), preview: true) }
            }
            .disabled(!connected || model.operationRunning)
            Text("Saved on the box and kept across restarts. Previews on this Mac aren't affected; to mute, long-press K2 on the box.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear(perform: sync)
        .onChange(of: model.status?.device.volume) { sync() }
    }

    private var connected: Bool { model.status?.device.connected ?? false }

    private func sync() {
        guard !editing, let volume = model.status?.device.volume else { return }
        level = Double(volume)
    }
}

struct VoicesView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VolumeRow(model: model)
            Divider()
            Text("Character").font(.headline)
            Text("The box is speaking as “\(currentVoiceName)”. Each character has its own voice and lines. Preview one, then click Use to write it to the box — no firmware flash needed. Over the UART port this takes a few minutes; when it's done the box says a line as the new character.")
                .font(.callout).foregroundStyle(.secondary)
            AddressRow(model: model)
            ScrollView {
                VStack(spacing: 8) {
                    RobotCard(model: model)
                    ForEach(Resources.bundledVoices) { entry in
                        VoiceCard(model: model, entry: entry)
                    }
                    CustomCharacterCard(model: model)
                }
            }
            if let operation = model.operation, operation.kind == .voicePack {
                OperationRow(operation: operation)
                if operation.state == .failed {
                    Text("Didn't finish, so the box keeps its built-in voice. Reconnect the cable and click Use again.").font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }

    private var currentVoiceName: String {
        let id = model.status?.device.voice ?? "builtin"
        if id == "builtin" || id == AppModel.robotID { return "Vibe Buddy" }
        if id == CustomCharacterCard.id { return String(localized: "Your own character") }
        return VoiceCatalogEntry.all.first { $0.id == id }?.name ?? id
    }
}

struct VoiceCard: View {
    @ObservedObject var model: AppModel
    let entry: VoiceCatalogEntry

    private var inUse: Bool { model.status?.device.voice == entry.id }

    var body: some View {
        HStack(spacing: 12) {
            Button { model.togglePreview(entry.id) } label: {
                Image(systemName: model.previewingVoice == entry.id ? "stop.fill" : "play.fill")
            }
            .disabled(model.operationRunning)
            if let frame = Resources.voicePack(entry.id)?.look.flatMap(LookBuilder.frames(of:))?.first {
                Image(nsImage: LookImages.image(frame, scale: 1))
            }
            VStack(alignment: .leading) {
                Text(entry.name).font(.body.weight(.semibold))
                Text(entry.tag).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if inUse {
                Label("In use", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            } else {
                Button("Use") { model.writeVoice(entry.id) }
                    .disabled(model.operationRunning || !(model.status?.device.connected ?? false))
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(inUse ? Color.accentColor : .clear, lineWidth: 2))
    }
}

struct OperationRow: View {
    let operation: DeviceOperation

    var body: some View {
        HStack {
            switch operation.state {
            case .running: ProgressView(value: operation.progress)
            case .done: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
            case .failed: Image(systemName: "xmark.octagon.fill").foregroundStyle(.red)
            case .replug: Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
            }
            Text(summary).font(.callout).lineLimit(1)
                .help(operation.state == .failed ? operation.message : "")
        }
    }

    /// The daemon's progress text is technical detail in English; the row says what is
    /// happening in the UI language and only quotes the daemon's detail on failure.
    private var summary: String {
        let percent = operation.progress.formatted(.percent.precision(.fractionLength(0)))
        switch (operation.kind, operation.state) {
        case (.voicePack, .running): return String(localized: "Writing voice pack… \(percent)")
        case (.voicePack, .done): return String(localized: "Voice pack written")
        case (.firmware, .running): return String(localized: "Flashing firmware… \(percent)")
        case (.firmware, .done): return String(localized: "Firmware flashed, the box has restarted")
        case (_, .replug): return String(localized: "Firmware flashed, but the box didn't start it. Unplug the box and plug it back in.")
        case (_, .failed): return String(localized: "Failed: \(operation.message)")
        }
    }
}

struct HooksView: View {
    @ObservedObject var model: AppModel
    @State private var pendingPlan: HookInstaller.Plan?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Connect agents").font(.headline)
            Text("Vibe Buddy only forwards session IDs, event names and working directories — never prompts or replies.").font(.callout).foregroundStyle(.secondary)
            Text("This writes user-level config, so one install covers both the desktop app's sessions and the same agent running in a terminal.").font(.callout).foregroundStyle(.secondary)
            ForEach(HookAgent.allCases, id: \.rawValue) { agent in
                HookRow(model: model, agent: agent, pendingPlan: $pendingPlan)
            }
            Spacer()
        }
        .sheet(item: Binding(get: { pendingPlan.map(PlanBox.init) }, set: { pendingPlan = $0?.plan })) { box in
            PlanSheet(plan: box.plan, confirm: { model.applyHookPlan(box.plan); pendingPlan = nil }, cancel: { pendingPlan = nil })
        }
    }
}

struct PlanBox: Identifiable {
    let plan: HookInstaller.Plan
    var id: String { plan.configURL.path }
}

struct HookRow: View {
    @ObservedObject var model: AppModel
    let agent: HookAgent
    @Binding var pendingPlan: HookInstaller.Plan?

    private var present: Bool { model.hookPresent(agent) }
    private var installed: Bool { model.hookInstalled[agent] ?? false }
    private var lastEvent: Date? { agent == .codex ? model.status?.hooks.codex : model.status?.hooks.claude }
    /// Only Codex has a trust step; Claude Code picks up config changes in its next session.
    private var codexHint: CodexTrustHint? {
        guard agent == .codex, installed else { return nil }
        return HookConfig.codexTrustHint(configModifiedAt: model.hookConfigModifiedAt(agent), lastEvent: lastEvent)
    }
    private var dotColor: Color {
        if !installed { return .gray }
        if case .changedSinceLastEvent = codexHint { return .red }
        return lastEvent == nil ? .orange : .green
    }

    var body: some View {
        HStack(spacing: 12) {
            Circle().fill(dotColor).frame(width: 10, height: 10)
            VStack(alignment: .leading) {
                Text(agent.displayName).font(.body.weight(.semibold))
                Text(statusText).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if installed {
                Button("Repair") { pendingPlan = model.hookInstallPlan(agent) }
                Button("Remove") { pendingPlan = model.hookRemovePlan(agent) }
            } else {
                Button("Connect") { pendingPlan = model.hookInstallPlan(agent) }.disabled(!present)
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
    }

    private var statusText: String {
        if !present { return String(localized: "\(agent.displayName) wasn't found on this Mac") }
        if !installed { return String(localized: "Not set up") }
        if case .changedSinceLastEvent(let changed)? = codexHint {
            return String(localized: "The config changed at \(changed.formatted(date: .abbreviated, time: .shortened)) and no Codex event has arrived since. Codex silently disables hooks that change: type /hooks in Codex and re-trust Vibe Buddy's six hooks.")
        }
        if let lastEvent { return String(localized: "Last event \(lastEvent.formatted(date: .abbreviated, time: .shortened))") }
        return agent == .codex
            ? String(localized: "Waiting for the first event… (Codex only runs this config after you trust it in /hooks)")
            : String(localized: "Waiting for the first event…")
    }
}

struct PlanSheet: View {
    let plan: HookInstaller.Plan
    let confirm: () -> Void
    let cancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Will change \(plan.configURL.path)").font(.headline)
            if plan.diff.isEmpty {
                Text("Nothing to change.").foregroundStyle(.secondary)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(plan.diff, id: \.self) { line in
                            Text(line).font(.system(.caption, design: .monospaced))
                                .foregroundStyle(line.hasPrefix("+") ? Color.green : Color.red)
                        }
                    }
                }
                .frame(maxHeight: 200)
            }
            if plan.agent == .codex {
                Text("After writing, open /hooks in Codex to review and trust this config — the app can't do that for you.").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
                Button("Write", action: confirm).keyboardShortcut(.defaultAction).disabled(plan.diff.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 520)
    }
}

struct DeviceView: View {
    @ObservedObject var model: AppModel

    // One scrolling form: a form above a fixed-height screen preview got squeezed, cutting off its last rows.
    var body: some View {
        Form {
            Section {
                LabeledContent("Link", value: connectionText)
                // Not while a flash waits for a replug: that row says not to hold K0 this time.
                if let pin = model.status?.device.pin {
                    PinNotice(pin: pin)
                }
                if model.daemonAlive, !connected, !model.operationRunning, model.operation?.state != .replug {
                    BoxSearchHelp(model: model)
                }
                LabeledContent("Box firmware", value: model.boxFirmware ?? "—")
                LabeledContent("Latest firmware", value: latestFirmware)
                if connected, let board = model.status?.device.unsupportedBoard {
                    Text("This is a \(board) board, which released firmware doesn't run on, so no update is offered.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if model.firmwareUpdateAvailable, let offer = model.offeredFirmware {
                    Button("Update to \(offer.version)") { confirmUpdate(offer) }
                        .disabled(model.operationRunning || !connected)
                } else if model.foreignFirmware {
                    Text("The box isn't running Vibe Buddy firmware.").foregroundStyle(.orange)
                    if model.firmwareDownloaded {
                        Button("Flash Vibe Buddy firmware") { FlashConfirm.foreign(then: model.updateFirmware) }
                            .disabled(model.operationRunning)
                    } else {
                        FirmwareUnavailable(model: model)
                    }
                } else if model.offeredFirmware?.newerThanBox == true {
                    // Newer firmware exists but isn't on disk yet.
                    FirmwareUnavailable(model: model)
                }
                if let operation = model.operation, operation.kind == .firmware {
                    OperationRow(operation: operation)
                    if operation.state == .failed {
                        Text("Before retrying, hold K0 on the box and replug the cable to put it in download mode.").font(.caption).foregroundStyle(.secondary)
                        Button("Retry") { model.updateFirmware() }.disabled(!connected)
                    }
                }
                HStack {
                    // Separately distributed firmware (VibeBuddy-firmware-*.zip on Releases) comes in here.
                    Button("Flash from file…") { flashFromFile() }
                    Button("Make the box blink") { model.identify() }
                }
                .disabled(model.operationRunning || !connected)
            }
            Section {
                ZStack {
                    RoundedRectangle(cornerRadius: 6).fill(Color.black.opacity(0.85))
                    if let image = model.screenshot {
                        // The box's 320×240 at one point per pixel: any other scale makes the pixel art uneven.
                        Image(nsImage: image).interpolation(.none).resizable().frame(width: 320, height: 240)
                    } else if model.screenshotBusy {
                        ProgressView()
                    } else {
                        // The panel is dark in both appearances, so the hint can't use the secondary text color.
                        Text("Click Refresh to see what the box is showing").foregroundStyle(.white.opacity(0.6))
                    }
                }
                .frame(height: 256)
            } header: {
                HStack {
                    Text("Box screen")
                    Spacer()
                    Button("Refresh") { model.takeScreenshot() }.disabled(model.screenshotBusy || model.operationRunning || !connected)
                    Button("Save image") { model.saveScreenshot() }.disabled(model.screenshot == nil)
                }
            }
        }
        .formStyle(.grouped)
    }

    private var connected: Bool { model.status?.device.connected ?? false }

    /// The offered version, or why there is none: a bare dash read as "no update checks at all".
    private var latestFirmware: String {
        if let offer = model.offeredFirmware { return offer.version }
        guard let updates = model.updates else { return "—" }
        guard updates.enabled else { return String(localized: "Update checks are off (General)") }
        if updates.error != nil { return String(localized: "Last check failed") }
        guard updates.lastCheck != nil else { return String(localized: "Not checked yet") }
        return String(localized: "None released yet")
    }

    private var connectionText: String {
        guard let device = model.status?.device else { return String(localized: "daemon isn't running") }
        guard device.connected else {
            return device.candidates?.isEmpty == false ? String(localized: "Several devices found") : String(localized: "Box not found")
        }
        return "\(device.port ?? "") · \(device.bridge ? String(localized: "UART bridge") : String(localized: "native USB"))"
    }

    private func flashFromFile() {
        let panel = NSOpenPanel()
        panel.title = String(localized: "Choose a firmware package")
        panel.allowedContentTypes = [.zip]
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let zip = panel.url else { return }
        let package: FirmwarePackage
        do {
            package = try model.openFirmwarePackage(zip)
        } catch {
            model.lastError = error.localizedDescription
            return
        }
        let alert = NSAlert()
        alert.messageText = String(localized: "Flash this firmware?")
        let current = model.boxFirmware ?? String(localized: "unknown")
        let offered = Firmware.label(version: package.version, build: package.build) ?? package.build
        alert.informativeText = String(localized: "Firmware package: \(offered)\nBox now: \(current)\nThe box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.")
        alert.addButton(withTitle: String(localized: "Flash"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        if alert.runModal() == .alertFirstButtonReturn { model.flashFirmware(package) }
    }

    private func confirmUpdate(_ offer: FirmwareOffer) {
        let alert = NSAlert()
        alert.messageText = String(localized: "Update the box firmware to \(offer.version)?")
        var text = String(localized: "The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.")
        if let notes = Firmware.notes(offer.notes, chinese: Resources.uiLanguage == .zh) { text = notes + "\n\n" + text }
        alert.informativeText = text
        alert.addButton(withTitle: String(localized: "Update"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        if alert.runModal() == .alertFirstButtonReturn { model.updateFirmware() }
    }
}

/// Why there's no firmware to flash yet and what to do about it: shared by onboarding and the Device tab.
/// The daemon downloads firmware from the update manifest; without it, the zip from the releases page still works.
struct FirmwareUnavailable: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let updates = model.updates, updates.enabled {
                if let error = updates.error {
                    Text("Couldn't get the firmware: \(error)")
                } else if model.offeredFirmware != nil {
                    Text("Downloading the firmware…")
                } else {
                    Text("Looking for firmware…")
                }
                Button("Check again") { model.checkForUpdates() }
            } else {
                Text("Update checks are off, so Vibe Buddy can't download firmware.")
            }
            HStack(spacing: 4) {
                Text("Or download the firmware zip yourself and use Flash from file… on the Device tab:")
                Link("Releases", destination: Resources.releasesPage)
            }
        }
        .font(.caption).foregroundStyle(.secondary)
    }
}

/// Why no box is connected yet: several devices to choose from, or else the usual causes of none at all.
struct BoxSearchHelp: View {
    @ObservedObject var model: AppModel

    var body: some View {
        if let candidates = model.status?.device.candidates, !candidates.isEmpty {
            BoxChoice(model: model, candidates: candidates)
        } else if model.status?.device.pin == nil {
            BoxNotFoundHelp()
        }
    }
}

/// Every ESP32-S3 on native USB looks the same, so with several plugged in and none of them the box seen before,
/// the user says which one it is; the daemon remembers that, as it would a box that reported our firmware.
struct BoxChoice: View {
    @ObservedObject var model: AppModel
    let candidates: [BoxCandidate]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Several devices are plugged in, and none of them is the box seen before. Which one is the box?")
                .font(.caption).foregroundStyle(.secondary)
            ForEach(candidates) { candidate in
                HStack {
                    Text(candidate.usbSerial.map { "\(candidate.port) · \($0)" } ?? candidate.port)
                        .font(.caption.monospaced())
                    Spacer()
                    if let serial = candidate.usbSerial {
                        Button("This is the box") { model.chooseBox(usbSerial: serial) }
                            .disabled(model.operationRunning)
                    }
                }
            }
        }
    }
}

/// A pin left set hides every other box behind "Box not found", so it is always shown.
struct PinNotice: View {
    let pin: SerialPin

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Only looking at \(pin.value), set by \(pin.variable).").foregroundStyle(.orange)
            Text("To find the box on its own again, run `launchctl unsetenv \(pin.variable)` in Terminal, then restart Vibe Buddy.")
                .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
        }
    }
}

/// What to try when the Mac can't see the box: the everyday causes first, then download mode as the last resort.
/// Holding K0 at power-up starts the ROM bootloader instead of the firmware, so that step always ends in a flash
/// (the box then shows up without our firmware and the flash button appears).
struct BoxNotFoundHelp: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Not showing up? Use a cable that carries data, not just power, and click Allow if macOS asks whether to let the accessory connect.")
            Text("Still nothing? Hold K0 on the box while you plug in the cable, then click Allow. The box starts in download mode with a dark screen, ready to be flashed with Vibe Buddy firmware.")
        }
        .font(.caption).foregroundStyle(.secondary)
    }
}

/// Confirmation for flashing a factory box: shared by onboarding and the Device tab.
enum FlashConfirm {
    @MainActor
    static func foreign(then flash: () -> Void) {
        let alert = NSAlert()
        alert.messageText = String(localized: "Flash the box with Vibe Buddy?")
        alert.informativeText = String(localized: "The box's current firmware and data will be erased and can't be recovered. It restarts on its own when done; over the UART port this takes a few minutes.")
        alert.alertStyle = .warning
        alert.addButton(withTitle: String(localized: "Flash"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        if alert.runModal() == .alertFirstButtonReturn { flash() }
    }
}

struct AdvancedView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Form {
            // A menu bar app has no Dock icon, so Finder windows and save panels can open behind other windows
            // and look like "clicking did nothing" (seen on a colleague's machine on 2026-09-22). Opening the folder explicitly
            // brings Finder to the front; the save panel is a sheet on the Settings window so it can't hide behind.
            Button("Open logs folder") {
                NSWorkspace.shared.activateFileViewerSelecting([Resources.logsDirectory.appendingPathComponent("vibebuddyd.log")])
            }
            LabeledContent("daemon", value: model.daemonAlive ? String(localized: "Running · \(model.status?.daemon.build ?? "")") : String(localized: "Not running"))
            Button("Restart daemon") { model.restartDaemon() }
            Button("Export diagnostics…") { exportDiagnostics() }
            Section {
                Text("Config file: \(Resources.applicationSupport.appendingPathComponent("config.json").path)").font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    /// Logs, config and both sides' build IDs in one folder, with no hook payloads.
    private func exportDiagnostics() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "vibe-buddy-diagnostics"
        panel.canCreateDirectories = true
        if let window = NSApp.keyWindow {
            panel.beginSheetModal(for: window) { response in
                guard response == .OK, let target = panel.url else { return }
                writeDiagnostics(to: target)
            }
        } else {
            NSApp.activate(ignoringOtherApps: true)
            guard panel.runModal() == .OK, let target = panel.url else { return }
            writeDiagnostics(to: target)
        }
    }

    private func writeDiagnostics(to target: URL) {
        let manager = FileManager.default
        try? manager.createDirectory(at: target, withIntermediateDirectories: true)
        for name in ["vibebuddyd.log", "codex-hooks.log"] {
            let source = Resources.logsDirectory.appendingPathComponent(name)
            if manager.fileExists(atPath: source.path) { try? manager.copyItem(at: source, to: target.appendingPathComponent(name)) }
        }
        // Get the config from the daemon instead of touching its file.
        if let config = model.status?.config, let data = try? StatusCoding.encoder().encode(config) {
            try? data.write(to: target.appendingPathComponent("config.json"))
        }
        let summary = """
        App \(Resources.displayVersion)
        daemon \(model.status?.daemon.build ?? "not connected")
        firmware \(model.boxFirmware ?? "—")
        offered firmware \(model.offeredFirmware?.version ?? "—")
        voice \(model.status?.device.voice ?? "—")
        """
        try? summary.write(to: target.appendingPathComponent("summary.txt"), atomically: true, encoding: .utf8)
        NSWorkspace.shared.activateFileViewerSelecting([target])
    }
}

/// A Character of the user's own: their drawings as the look, a preset Character's voice and lines.
/// The app doesn't draw (ADR-0009): the user brings one to four images (normal, eyes closed, happy,
/// sad), made with whatever tool they like.
struct CustomCharacterCard: View {
    static let id = "custom"
    @ObservedObject var model: AppModel
    @State private var frames: [RGBAImage] = []
    @State private var look: Data?
    @State private var lender = Resources.bundledVoices.first?.id ?? ""
    @State private var problem: String?

    private var inUse: Bool { model.status?.device.voice == Self.id }

    static let prompt = "Pixel art game sprite of [describe your character], chibi proportions, standing, front view, full body, centered, arms down, flat colors, thick dark outline, limited 16-color palette, plain solid white background. Then the same character in exactly the same pose with the eyes closed; with a big happy smile; with a sad face."

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Your own character").font(.body.weight(.semibold))
                Spacer()
                if inUse {
                    Label("In use", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                }
            }
            Text("Draw a figure with any image tool, on a plain white background: one image, or four in the order normal, eyes closed, happy, sad. The box shows it in place of the robot, with the voice and lines of the character you pick.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("Choose images…") { choose() }
                Button("Copy a prompt") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(Self.prompt, forType: .string)
                }
            }
            if let problem {
                Text(problem).font(.caption).foregroundStyle(.red)
            }
            if !frames.isEmpty {
                HStack(spacing: 6) {
                    ForEach(frames.indices, id: \.self) { index in
                        Image(nsImage: LookImages.image(frames[index], scale: 2))
                            .background(Color.black)
                    }
                }
                Picker(String(localized: "Voice and lines from"), selection: $lender) {
                    ForEach(Resources.bundledVoices) { entry in Text(entry.name).tag(entry.id) }
                }
                .frame(maxWidth: 320)
                Button("Use") { use() }
                    .disabled(model.operationRunning || !(model.status?.device.connected ?? false))
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(inUse ? Color.accentColor : .clear, lineWidth: 2))
        .onAppear {
            // The last one written, so the card shows what the box wears after the app restarts.
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
        // In name order, so normal.png, closed.png, ... or 1.png, 2.png, ... come in as meant.
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

/// What the buddy calls the user, one pick per language: every Character of that language says it.
struct AddressRow: View {
    @ObservedObject var model: AppModel

    var body: some View {
        HStack(spacing: 16) {
            Text("What the buddy calls you")
            ForEach([VoiceLanguage.zh, .en], id: \.self) { language in
                Picker(selection: Binding(get: { model.formOfAddress(language) ?? "" },
                                          set: { model.setFormOfAddress($0.isEmpty ? nil : $0, for: language) })) {
                    Text("Nothing").tag("")
                    ForEach(FormOfAddress.all.filter { $0.language == language }) { form in
                        Text(verbatim: form.words).tag(form.id)
                    }
                } label: {
                    Text(language == .zh ? "Chinese" : "English")
                }
                .frame(maxWidth: 180)
                .disabled(model.operationRunning)
            }
        }
        .font(.callout)
    }
}

/// The robot, the default Character: drawn by the box itself, wearing the voice and lines of the
/// Character picked here, or the five fixed lines it shipped with.
struct RobotCard: View {
    @ObservedObject var model: AppModel
    @State private var lender = "builtin"

    var body: some View {
        HStack(spacing: 12) {
            if let face = Resources.robotFace {
                Image(nsImage: face)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(verbatim: "Vibe Buddy").font(.body.weight(.semibold))
                Text("The original robot, drawn by the box itself").font(.caption).foregroundStyle(.secondary)
                Picker(String(localized: "Voice and lines from"), selection: $lender) {
                    Text("Built-in voice (Jessica)").tag("builtin")
                    ForEach(Resources.bundledVoices) { entry in Text(entry.name).tag(entry.id) }
                }
                .frame(maxWidth: 320)
            }
            Spacer()
            if model.wearsRobot && lender == model.robotLender {
                Label("In use", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            } else {
                Button("Use") { model.writeRobot(lender: lender) }
                    .disabled(model.operationRunning || !(model.status?.device.connected ?? false))
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(model.wearsRobot ? Color.accentColor : .clear, lineWidth: 2))
        .onAppear { lender = model.robotLender }
    }
}
