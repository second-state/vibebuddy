import SwiftUI
import VibeBuddyCore

struct SettingsView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        TabView {
            GeneralView(model: model).tabItem { Label("General", systemImage: "gearshape") }
            VoicesView(model: model).tabItem { Label("Sound", systemImage: "speaker.wave.2") }
            HooksView(model: model).tabItem { Label("Agents", systemImage: "link") }
            DeviceView(model: model).tabItem { Label("Device", systemImage: "cpu") }
            AdvancedView(model: model).tabItem { Label("Advanced", systemImage: "wrench.and.screwdriver") }
        }
        .padding(20)
        .frame(width: 640, height: 480)
    }
}

struct GeneralView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Form {
            Toggle("Launch at login", isOn: Binding(get: { model.launchAtLogin }, set: { model.setLaunchAtLogin($0) }))
            Toggle("Notify me when the box disconnects or the daemon fails", isOn: Binding(get: { model.status?.config.notifyLink ?? true }, set: { model.setNotifyLink($0) }))
                .disabled(model.status == nil)
            Section {
                LabeledContent("App", value: Resources.displayVersion)
                LabeledContent("daemon", value: model.status?.daemon.build ?? String(localized: "Not connected"))
            }
        }
        .formStyle(.grouped)
    }
}

/// 音量滑块：值来自盒子的状态，松手才发；拖动中不让状态流把滑块拽回去。
/// 下限 20：能存下来的零音量就是持久静音，而静音有意只在盒子上、不持久化。
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
            Text("Announcement voice").font(.headline)
            Text("The box is using “\(currentVoiceName)”. Preview a voice, then click Use to write it to the box — no firmware flash needed. Over the UART port this takes a few minutes; when it's done the box says a line in the new voice.")
                .font(.callout).foregroundStyle(.secondary)
            ScrollView {
                VStack(spacing: 8) {
                    ForEach(Resources.bundledVoices) { entry in
                        VoiceCard(model: model, entry: entry)
                    }
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
        if id == "builtin" { return String(localized: "Built-in voice (Wanwan Xiaohe)") }
        return VoiceCatalogEntry.all.first { $0.id == id }?.name ?? id
    }
}

struct VoiceCard: View {
    @ObservedObject var model: AppModel
    let entry: VoiceCatalogEntry

    private var bundled: Bool { Resources.voicePack(entry.id) != nil }
    private var inUse: Bool { model.status?.device.voice == entry.id }

    var body: some View {
        HStack(spacing: 12) {
            Button { model.togglePreview(entry.id) } label: {
                Image(systemName: model.previewingVoice == entry.id ? "stop.fill" : "play.fill")
            }
            .disabled(!bundled || model.operationRunning)
            VStack(alignment: .leading) {
                Text(entry.name).font(.body.weight(.semibold))
                Text(entry.tag).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if inUse {
                Label("In use", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            } else {
                Button("Use") { model.writeVoice(entry.id) }
                    .disabled(!bundled || model.operationRunning || !(model.status?.device.connected ?? false))
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
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
            }
            Text(summary).font(.callout).lineLimit(1).help(operation.message)
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
        case (.firmware, .done): return String(localized: "Firmware flashed, the box is restarting")
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
    /// 只有 Codex 有信任这一关；Claude Code 改完配置下个会话就生效。
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

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Form {
                LabeledContent("Link", value: connectionText)
                LabeledContent("Box firmware", value: model.status?.device.firmwareBuild ?? "—")
                LabeledContent("Bundled with app", value: model.bundledFirmwareBuild ?? String(localized: "This build has no bundled firmware"))
                if model.firmwareUpdateAvailable {
                    Button("Update to bundled version") { confirmUpdate() }
                        .disabled(model.operationRunning || !(model.status?.device.connected ?? false))
                } else if model.foreignFirmware, model.bundledFirmwareBuild != nil {
                    Text("The box isn't running Vibe Buddy firmware.").foregroundStyle(.orange)
                    Button("Flash Vibe Buddy firmware") { FlashConfirm.foreign(then: model.updateFirmware) }
                        .disabled(model.operationRunning)
                }
                if let operation = model.operation, operation.kind == .firmware {
                    OperationRow(operation: operation)
                    if operation.state == .failed {
                        Text("Before retrying, hold K0 on the box and replug the cable to put it in download mode.").font(.caption).foregroundStyle(.secondary)
                        Button("Retry") { model.updateFirmware() }.disabled(!(model.status?.device.connected ?? false))
                    }
                }
                // 单独分发的固件（Release 上的 VibeBuddy-firmware-*.zip）从这里进来。
                Button("Flash from file…") { flashFromFile() }
                    .disabled(model.operationRunning || !(model.status?.device.connected ?? false))
                Button("Make the box blink") { model.identify() }.disabled(model.operationRunning || !(model.status?.device.connected ?? false))
            }
            .formStyle(.grouped)
            HStack {
                Text("Box screen").font(.headline)
                Spacer()
                Button("Refresh") { model.takeScreenshot() }.disabled(model.screenshotBusy || model.operationRunning || !(model.status?.device.connected ?? false))
                Button("Save image") { model.saveScreenshot() }.disabled(model.screenshot == nil)
            }
            ZStack {
                RoundedRectangle(cornerRadius: 6).fill(Color.black.opacity(0.85))
                if let image = model.screenshot {
                    Image(nsImage: image).interpolation(.none).resizable().aspectRatio(contentMode: .fit).padding(4)
                } else if model.screenshotBusy {
                    ProgressView()
                } else {
                    Text("Click Refresh to see what the box is showing").foregroundStyle(.secondary)
                }
            }
            .frame(height: 180)
        }
    }

    private var connectionText: String {
        guard let device = model.status?.device else { return String(localized: "daemon isn't running") }
        guard device.connected else { return String(localized: "Box not found") }
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
        let current = model.status?.device.firmwareBuild ?? String(localized: "unknown")
        alert.informativeText = String(localized: "Firmware package: \(package.build)\nBox now: \(current)\nThe box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.")
        alert.addButton(withTitle: String(localized: "Flash"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        if alert.runModal() == .alertFirstButtonReturn { model.flashFirmware(package) }
    }

    private func confirmUpdate() {
        let alert = NSAlert()
        alert.messageText = String(localized: "Update the box firmware?")
        alert.informativeText = String(localized: "The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.")
        alert.addButton(withTitle: String(localized: "Update"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        if alert.runModal() == .alertFirstButtonReturn { model.updateFirmware() }
    }
}

/// 往出厂机上刷固件的确认：引导页和设备页共用一段话。
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
            // 菜单栏 App 没有 Dock 图标，Finder 窗口和保存面板都可能开在别人窗口后面，
            // 看起来像"点了没反应"（2026-09-22 同事机器上如此）。开文件夹显式把
            // Finder 拉到前台；保存面板挂成设置窗的 sheet，躲不到后面去。
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

    /// 日志、配置、两边构建标识打成一个文件夹，不含任何 Hook 载荷。
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
        // 配置从 daemon 拿，不直接碰它的文件。
        if let config = model.status?.config, let data = try? StatusCoding.encoder().encode(config) {
            try? data.write(to: target.appendingPathComponent("config.json"))
        }
        let summary = """
        App \(Resources.displayVersion)
        daemon \(model.status?.daemon.build ?? "not connected")
        firmware \(model.status?.device.firmwareBuild ?? "—")
        bundled firmware \(model.bundledFirmwareBuild ?? "—")
        voice \(model.status?.device.voice ?? "—")
        """
        try? summary.write(to: target.appendingPathComponent("summary.txt"), atomically: true, encoding: .utf8)
        NSWorkspace.shared.activateFileViewerSelecting([target])
    }
}
