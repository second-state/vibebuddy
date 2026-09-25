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
    /// 串口开了却一直没报构建号：盒子跑的不是我们的固件（出厂机），该提供刷入。
    @Published private(set) var foreignFirmware = false

    let client = DaemonClient()
    let supervisor = DaemonSupervisor()
    let preview = VoicePreview()
    let bundledFirmwareBuild = Resources.bundledFirmwareBuild
    /// 由 App 自己看管 daemon；发现旧 LaunchAgent 且用户不肯卸时为 false。
    var managesDaemon = true

    private var streamTask: Task<Void, Never>?
    private var connectedSince: Date?
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
                Notifier.notify(title: "Vibe Buddy", body: String(localized: "The daemon failed to start three times in a row. Click the menu bar icon to restart it."))
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
            Notifier.notify(title: String(localized: "Codex hook config updated"),
                            body: String(localized: "Vibe Buddy rewrote ~/.codex/hooks.json. Codex silently disables hooks that change: type /hooks in Codex and re-trust them so the box keeps getting Codex events."))
        }
        refreshHookStates()
        if managesDaemon { supervisor.start() }
        streamTask = Task { [weak self] in await self?.followStatus() }
        Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            Task { @MainActor in
                self?.checkLink()
                // 宽限期过了状态流不会再来消息，沉默要靠时钟发现。
                self?.refreshForeignFirmware()
            }
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
        if status.device.connected {
            if connectedSince == nil { connectedSince = Date() }
        } else {
            connectedSince = nil
        }
        self.status = status
        daemonAlive = true
        refreshMenu()
        refreshForeignFirmware()
    }

    private func refreshForeignFirmware() {
        let foreign = Firmware.foreign(
            connected: status?.device.connected ?? false,
            device: status?.device.firmwareBuild,
            connectedFor: connectedSince.map { Date().timeIntervalSince($0) } ?? 0)
        if foreign != foreignFirmware { foreignFirmware = foreign }
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
            Notifier.notify(title: String(localized: "Box disconnected"), body: String(localized: "Vibe Buddy hasn't seen the box for 30 seconds. Check the USB cable."))
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
            lastError = String(localized: "Couldn't change Launch at login: \(error.localizedDescription)")
        }
        launchAtLogin = LoginItem.isEnabled
    }

    var operation: DeviceOperation? { status?.operation }
    var operationRunning: Bool { operation?.state == .running }

    func writeVoice(_ id: String) {
        guard let pack = Resources.voicePack(id) else { lastError = String(localized: "This app has no voice pack for \(id)"); return }
        run { try await self.client.writeVoicePack(pack.data) }
    }

    func togglePreview(_ id: String) {
        guard let pack = Resources.voicePack(id) else { lastError = String(localized: "This app has no voice pack for \(id)"); return }
        preview.toggle(pack: pack)
        previewingVoice = preview.playingVoice
    }

    var firmwareUpdateAvailable: Bool {
        Firmware.updateAvailable(device: status?.device.firmwareBuild, bundled: bundledFirmwareBuild)
    }

    func updateFirmware() {
        guard let files = Resources.firmwareFiles else { lastError = String(localized: "This build has no bundled firmware"); return }
        run { try await self.client.flashFirmware(bootloader: files.bootloader, partitionTable: files.partitionTable, app: files.app) }
    }

    /// 用户自己拿到的固件包（CI 发的 zip）：解到临时目录，验完三件套再交给确认框。
    /// 临时目录留到烧录结束，daemon 按路径读文件。
    func openFirmwarePackage(_ zip: URL) throws -> FirmwarePackage {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("vibebuddy-firmware-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let unzip = Process()
        unzip.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
        unzip.arguments = ["-x", "-k", zip.path, directory.path]
        try unzip.run()
        unzip.waitUntilExit()
        guard unzip.terminationStatus == 0 else { throw DaemonError(message: String(localized: "Couldn't unzip \(zip.lastPathComponent)")) }
        return try FirmwarePackage.inspect(directory: directory)
    }

    func flashFirmware(_ package: FirmwarePackage) {
        run { try await self.client.flashFirmware(bootloader: package.bootloader, partitionTable: package.partitionTable, app: package.app) }
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
            lastError = String(localized: "Couldn't write the hook config: \(error.localizedDescription)")
        }
        refreshHookStates()
    }

    private func run(_ action: @escaping () async throws -> Void) {
        Task {
            do { try await action() } catch { lastError = error.localizedDescription }
        }
    }
}
