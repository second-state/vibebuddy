import SwiftUI
import VibeBuddyCore

struct SettingsView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        TabView {
            GeneralView(model: model).tabItem { Label("通用", systemImage: "gearshape") }
            VoicesView(model: model).tabItem { Label("声音", systemImage: "speaker.wave.2") }
            HooksView(model: model).tabItem { Label("接入", systemImage: "link") }
            DeviceView(model: model).tabItem { Label("设备", systemImage: "cpu") }
            AdvancedView(model: model).tabItem { Label("高级", systemImage: "wrench.and.screwdriver") }
        }
        .padding(20)
        .frame(width: 640, height: 480)
    }
}

struct GeneralView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Form {
            Toggle("登录时启动", isOn: Binding(get: { model.launchAtLogin }, set: { model.setLaunchAtLogin($0) }))
            Toggle("盒子断开或 daemon 异常时通知我", isOn: Binding(get: { model.status?.config.notifyLink ?? true }, set: { model.setNotifyLink($0) }))
                .disabled(model.status == nil)
            Section {
                LabeledContent("App", value: Resources.displayVersion)
                LabeledContent("daemon", value: model.status?.daemon.build ?? "未连接")
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
                Text("音量").font(.headline)
                Spacer()
                Text("\(Int(level))").monospacedDigit().foregroundStyle(.secondary)
            }
            HStack(spacing: 12) {
                Slider(value: $level, in: 20...100, step: 5) { isEditing in
                    editing = isEditing
                    if !isEditing { model.setVolume(Int(level)) }
                }
                Button("在盒子上试一句") { model.setVolume(Int(level), preview: true) }
            }
            .disabled(!connected || model.operationRunning)
            Text("存在盒子里，重启不丢。Mac 上的试听不受它影响；静音只在盒子上长按 K2。")
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
            Text("播报音色").font(.headline)
            Text("盒子现在用的是「\(currentVoiceName)」。挑一个试听，点「使用」写进盒子；不用刷固件。")
                .font(.callout).foregroundStyle(.secondary)
            ScrollView {
                VStack(spacing: 8) {
                    ForEach(VoiceCatalogEntry.all) { entry in
                        VoiceCard(model: model, entry: entry)
                    }
                }
            }
            if let operation = model.operation, operation.kind == .voicePack {
                OperationRow(operation: operation)
                if operation.state == .failed {
                    Text("没写完，盒子先用内置音色；插好线后重新点「使用」。").font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }

    private var currentVoiceName: String {
        let id = model.status?.device.voice ?? "builtin"
        if id == "builtin" { return "内置音色（湾湾小何）" }
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
                Label("已写入", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            } else {
                Button("使用") { model.writeVoice(entry.id) }
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
            Text(operation.message).font(.callout).lineLimit(1)
        }
    }
}

struct HooksView: View {
    @ObservedObject var model: AppModel
    @State private var pendingPlan: HookInstaller.Plan?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("接入 Agent").font(.headline)
            Text("Vibe Buddy 只转发会话标识、事件名和工作目录，不转发 prompt 与回复。").font(.callout).foregroundStyle(.secondary)
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
                Button("修复") { pendingPlan = model.hookInstallPlan(agent) }
                Button("移除") { pendingPlan = model.hookRemovePlan(agent) }
            } else {
                Button("接入") { pendingPlan = model.hookInstallPlan(agent) }.disabled(!present)
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
    }

    private var statusText: String {
        if !present { return "这台 Mac 上没找到 \(agent.displayName)" }
        if !installed { return "未接入" }
        if case .changedSinceLastEvent(let changed)? = codexHint {
            return "配置在 \(changed.formatted(date: .abbreviated, time: .shortened)) 改过，之后没收到过 Codex 事件。Codex 会静默停用改过的 hook：在 Codex 里输入 /hooks，把 Vibe Buddy 的六条重新信任。"
        }
        if let lastEvent { return "最近一次事件 \(lastEvent.formatted(date: .abbreviated, time: .shortened))" }
        return agent == .codex ? "等待第一次事件…（Codex 要先在 /hooks 里信任这份配置才会运行它）" : "等待第一次事件…"
    }
}

struct PlanSheet: View {
    let plan: HookInstaller.Plan
    let confirm: () -> Void
    let cancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("将改动 \(plan.configURL.path)").font(.headline)
            if plan.diff.isEmpty {
                Text("没有需要改的地方。").foregroundStyle(.secondary)
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
                Text("写入后还要在 Codex 里打开 /hooks 核对并信任这份配置，App 替不了你。").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("取消", action: cancel).keyboardShortcut(.cancelAction)
                Button("写入", action: confirm).keyboardShortcut(.defaultAction).disabled(plan.diff.isEmpty)
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
                LabeledContent("链路", value: connectionText)
                LabeledContent("盒子固件", value: model.status?.device.firmwareBuild ?? "—")
                LabeledContent("App 附带", value: model.bundledFirmwareBuild ?? "这个构建没有附带固件")
                if model.firmwareUpdateAvailable {
                    Button("更新到 App 附带版本") { confirmUpdate() }
                        .disabled(model.operationRunning || !(model.status?.device.connected ?? false))
                }
                if let operation = model.operation, operation.kind == .firmware {
                    OperationRow(operation: operation)
                    if operation.state == .failed {
                        Text("重试前可以按住盒子的 K0 再插一次线，让它进入下载模式。").font(.caption).foregroundStyle(.secondary)
                        Button("重试") { model.updateFirmware() }.disabled(!(model.status?.device.connected ?? false))
                    }
                }
                Button("让盒子眨眼") { model.identify() }.disabled(model.operationRunning || !(model.status?.device.connected ?? false))
            }
            .formStyle(.grouped)
            HStack {
                Text("盒子画面").font(.headline)
                Spacer()
                Button("刷新") { model.takeScreenshot() }.disabled(model.screenshotBusy || model.operationRunning || !(model.status?.device.connected ?? false))
                Button("保存图片") { model.saveScreenshot() }.disabled(model.screenshot == nil)
            }
            ZStack {
                RoundedRectangle(cornerRadius: 6).fill(Color.black.opacity(0.85))
                if let image = model.screenshot {
                    Image(nsImage: image).interpolation(.none).resizable().aspectRatio(contentMode: .fit).padding(4)
                } else if model.screenshotBusy {
                    ProgressView()
                } else {
                    Text("点「刷新」看看盒子在干什么").foregroundStyle(.secondary)
                }
            }
            .frame(height: 180)
        }
    }

    private var connectionText: String {
        guard let device = model.status?.device else { return "daemon 没起来" }
        guard device.connected else { return "未找到盒子" }
        return "\(device.port ?? "") · \(device.bridge ? "UART 桥" : "原生 USB")"
    }

    private func confirmUpdate() {
        let alert = NSAlert()
        alert.messageText = "更新盒子固件？"
        alert.informativeText = "盒子会重启一次，语音包和当日战绩都保留。UART 桥上大约要几分钟。"
        alert.addButton(withTitle: "更新")
        alert.addButton(withTitle: "取消")
        if alert.runModal() == .alertFirstButtonReturn { model.updateFirmware() }
    }
}

struct AdvancedView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Form {
            Button("打开日志文件夹") { NSWorkspace.shared.open(Resources.logsDirectory) }
            Button("重启 daemon") { model.restartDaemon() }
            Button("导出诊断…") { exportDiagnostics() }
            Section {
                Text("配置文件：\(Resources.applicationSupport.appendingPathComponent("config.json").path)").font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    /// 日志、配置、两边构建标识打成一个文件夹，不含任何 Hook 载荷。
    private func exportDiagnostics() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "vibe-buddy-diagnostics"
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let target = panel.url else { return }
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
        daemon \(model.status?.daemon.build ?? "未连接")
        firmware \(model.status?.device.firmwareBuild ?? "—")
        bundled firmware \(model.bundledFirmwareBuild ?? "—")
        voice \(model.status?.device.voice ?? "—")
        """
        try? summary.write(to: target.appendingPathComponent("summary.txt"), atomically: true, encoding: .utf8)
        NSWorkspace.shared.activateFileViewerSelecting([target])
    }
}
