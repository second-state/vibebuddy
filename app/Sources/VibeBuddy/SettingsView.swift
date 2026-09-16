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
                LabeledContent("App", value: Resources.bundleVersion)
                LabeledContent("daemon", value: model.status?.daemon.build ?? "未连接")
            }
        }
        .formStyle(.grouped)
    }
}

struct VoicesView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
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
            .disabled(!bundled)
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

    var body: some View {
        HStack(spacing: 12) {
            Circle().fill(installed ? (lastEvent == nil ? Color.orange : Color.green) : Color.gray).frame(width: 10, height: 10)
            VStack(alignment: .leading) {
                Text(agent.displayName).font(.body.weight(.semibold))
                Text(statusText).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if installed {
                Button("修复") { pendingPlan = model.hookInstallPlan(agent) }
                Button("移除") { pendingPlan = model.hookRemovePlan(agent) }
            } else {
                Button("安装") { pendingPlan = model.hookInstallPlan(agent) }.disabled(!present)
            }
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .controlBackgroundColor)))
    }

    private var statusText: String {
        if !present { return "这台 Mac 上没找到 \(agent.displayName)" }
        if !installed { return "未接入" }
        if let lastEvent { return "最近一次事件 \(lastEvent.formatted(date: .abbreviated, time: .shortened))" }
        return agent == .codex ? "等待第一次事件…（记得在 Codex 的 /hooks 页面信任这份配置）" : "等待第一次事件…"
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
                LabeledContent("连接", value: connectionText)
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
                    }
                }
                Button("让盒子眨眼") { model.identify() }.disabled(!(model.status?.device.connected ?? false))
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
        guard let device = model.status?.device else { return "daemon 未连接" }
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
        for name in ["beacond.log", "codex-hooks.log"] {
            let source = Resources.logsDirectory.appendingPathComponent(name)
            if manager.fileExists(atPath: source.path) { try? manager.copyItem(at: source, to: target.appendingPathComponent(name)) }
        }
        let config = Resources.applicationSupport.appendingPathComponent("config.json")
        if manager.fileExists(atPath: config.path) { try? manager.copyItem(at: config, to: target.appendingPathComponent("config.json")) }
        let summary = """
        App \(Resources.bundleVersion)
        daemon \(model.status?.daemon.build ?? "未连接")
        firmware \(model.status?.device.firmwareBuild ?? "—")
        bundled firmware \(model.bundledFirmwareBuild ?? "—")
        voice \(model.status?.device.voice ?? "—")
        """
        try? summary.write(to: target.appendingPathComponent("summary.txt"), atomically: true, encoding: .utf8)
        NSWorkspace.shared.activateFileViewerSelecting([target])
    }
}
