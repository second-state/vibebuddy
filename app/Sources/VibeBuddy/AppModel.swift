import AppKit
import Foundation
import VibeBuddyCore

/// 界面看到的一切都从这里来：状态快照、daemon 存活、菜单状态、操作结果。
@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var status: Status?
    @Published private(set) var daemonAlive = false
    @Published private(set) var menu = MenuState.derive(status: nil, daemonAlive: false)
    @Published var lastError: String?
    @Published private(set) var screenshot: NSImage?
    @Published private(set) var screenshotBusy = false
    @Published private(set) var hookInstalled: [HookAgent: Bool] = [:]
    @Published private(set) var launchAtLogin = LoginItem.isEnabled
    @Published private(set) var previewingVoice: String?

    let client = DaemonClient()
    let supervisor = DaemonSupervisor()
    let preview = VoicePreview()
    var bundledFirmwareBuild: String? { Resources.bundledFirmwareBuild(board: status?.device.board) }
    /// 由 App 自己看管 daemon；发现旧 LaunchAgent 且用户不肯卸时为 false。
    var managesDaemon = true

    private var streamTask: Task<Void, Never>?
    private var linkLostSince: Date?
    private var linkNotified = false

    init() {
        preview.onFinish = { [weak self] in self?.previewingVoice = nil }
        supervisor.onStateChange = { [weak self] state in
            guard let self else { return }
            if state != .running { self.daemonAlive = false; self.status = nil; self.refreshMenu() }
        }
        supervisor.onGiveUp = { [weak self] in
            guard let self else { return }
            self.daemonAlive = false
            self.refreshMenu()
            // 「通知我」关掉时连这条也不弹；daemon 没起来时拿不到配置，按默认开。
            if self.status?.config.notifyLink ?? true {
                Notifier.notify(title: "Vibe Buddy", body: "daemon 连续三次启动失败，点菜单栏图标重启。")
            }
        }
        refreshHookStates()
    }

    func start() {
        Resources.migrateLegacyDirectories()
        try? HookInstaller.deployBinary()
        if HookInstaller.migrateLegacyCommands().contains(.codex) {
            // 2026-09-16 改名迁移静默改写了 hooks.json，Codex 把六条 hook 当作
            // 改过的静默停用，盒子四个小时没播过 Codex 的事。这条不受「链路异常
            // 通知」开关管：它就是 App 自己动了手才需要人补一步。
            Notifier.notify(title: "Codex 的 Hook 配置更新了",
                            body: "Vibe Buddy 改写了 ~/.codex/hooks.json。Codex 会静默停用改过的 hook，请在 Codex 里输入 /hooks 重新信任，盒子才收得到 Codex 的事。")
        }
        refreshHookStates()
        if managesDaemon { supervisor.start() }
        streamTask = Task { [weak self] in await self?.followStatus() }
        Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.checkLink() }
        }
    }

    func shutdown() {
        streamTask?.cancel()
        supervisor.stop()
    }

    private func followStatus() async {
        while !Task.isCancelled {
            do {
                try await client.stream { [weak self] status in
                    Task { @MainActor in self?.apply(status) }
                }
            } catch {
                daemonAlive = false
                refreshMenu()
            }
            try? await Task.sleep(nanoseconds: 1_000_000_000)
        }
    }

    private func apply(_ status: Status) {
        self.status = status
        daemonAlive = true
        refreshMenu()
    }

    private func refreshMenu() {
        menu = MenuState.derive(status: status, daemonAlive: daemonAlive)
    }

    /// 链路断开 30 秒去抖后才弹一次通知；插回来就复位。
    private func checkLink() {
        guard let status, daemonAlive, status.config.notifyLink else { linkLostSince = nil; return }
        if status.device.connected {
            linkLostSince = nil
            linkNotified = false
            return
        }
        if linkLostSince == nil { linkLostSince = Date() }
        if !linkNotified, let since = linkLostSince, Date().timeIntervalSince(since) >= 30 {
            linkNotified = true
            Notifier.notify(title: "盒子断开了", body: "Vibe Buddy 已经 30 秒没找到盒子，检查一下 USB 线。")
        }
    }

    // MARK: 操作

    func restartDaemon() {
        if managesDaemon { supervisor.restart() } else { Task { try? await client.restart() } }
    }

    func identify() { run { try await self.client.identify() } }

    func setVolume(_ level: Int, preview: Bool = false) {
        run { try await self.client.setVolume(level, preview: preview) }
    }

    func setNotifyLink(_ enabled: Bool) {
        guard var config = status?.config else { return }
        config.notifyLink = enabled
        run { _ = try await self.client.putConfig(config) }
    }

    func setLaunchAtLogin(_ enabled: Bool) {
        do {
            try LoginItem.set(enabled: enabled)
        } catch {
            lastError = "登录时启动设置失败：\(error.localizedDescription)"
        }
        launchAtLogin = LoginItem.isEnabled
    }

    var operation: DeviceOperation? { status?.operation }
    var operationRunning: Bool { operation?.state == .running }

    func writeVoice(_ id: String) {
        guard let pack = Resources.voicePack(id) else { lastError = "App 里没有 \(id) 的语音包"; return }
        run { try await self.client.writeVoicePack(pack.data) }
    }

    func togglePreview(_ id: String) {
        guard let pack = Resources.voicePack(id) else { lastError = "App 里没有 \(id) 的语音包"; return }
        preview.toggle(pack: pack)
        previewingVoice = preview.playingVoice
    }

    var firmwareUpdateAvailable: Bool {
        Firmware.updateAvailable(device: status?.device.firmwareBuild, bundled: bundledFirmwareBuild)
    }

    func updateFirmware() {
        let board = status?.device.board ?? "alientek-box"
        guard let files = Resources.firmwareFiles(board: board) else { lastError = "这个构建没有附带该板型的固件"; return }
        run { try await self.client.flashFirmware(bootloader: files.bootloader, partitionTable: files.partitionTable, app: files.app, board: board) }
    }

    func takeScreenshot() {
        screenshotBusy = true
        Task {
            defer { screenshotBusy = false }
            do {
                let png = try await client.screenshot()
                screenshot = NSImage(data: png)
            } catch {
                lastError = error.localizedDescription
            }
        }
    }

    func saveScreenshot() {
        guard let image = screenshot, let tiff = image.tiffRepresentation,
              let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) else { return }
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "vibe-buddy-\(Int(Date().timeIntervalSince1970)).png"
        panel.allowedContentTypes = [.png]
        if panel.runModal() == .OK, let url = panel.url {
            try? png.write(to: url)
        }
    }

    // MARK: 接入

    func refreshHookStates() {
        for agent in HookAgent.allCases {
            hookInstalled[agent] = HookInstaller.isInstalled(agent)
        }
    }

    func hookPresent(_ agent: HookAgent) -> Bool { HookInstaller.isPresent(agent) }
    func hookConfigModifiedAt(_ agent: HookAgent) -> Date? { HookInstaller.configModifiedAt(agent) }

    /// 返回要确认的差异；调用方确认后再 apply。
    func hookInstallPlan(_ agent: HookAgent) -> HookInstaller.Plan { HookInstaller.installPlan(for: agent) }
    func hookRemovePlan(_ agent: HookAgent) -> HookInstaller.Plan { HookInstaller.removePlan(for: agent) }

    func applyHookPlan(_ plan: HookInstaller.Plan) {
        do {
            try HookInstaller.deployBinary()
            try HookInstaller.apply(plan)
        } catch {
            lastError = "写 Hook 配置失败：\(error.localizedDescription)"
        }
        refreshHookStates()
    }

    private func run(_ action: @escaping () async throws -> Void) {
        Task {
            do { try await action() } catch { lastError = error.localizedDescription }
        }
    }
}
