import AppKit
import Foundation
import VibeBuddyCore

/// Everything the UI shows comes from here: the status snapshot, daemon liveness, menu state, operation results.
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
    /// Serial port open but no build ID ever reported: the box isn't running our firmware (a factory box), so offer to flash it.
    @Published private(set) var foreignFirmware = false

    let client = DaemonClient()
    let supervisor = DaemonSupervisor()
    let preview = VoicePreview()
    let bundledFirmwareBuild = Resources.bundledFirmwareBuild
    /// Whether the app manages the daemon itself; false when an old LaunchAgent was found and the user kept it.
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
            // With "Notify me" off, not even this one shows; without a daemon there's no config, so default to on.
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
            // On 2026-09-16 the rename migration silently rewrote hooks.json, Codex treated all six hooks
            // as changed and silently disabled them, and the box announced nothing from Codex for four hours. This one ignores the
            // link-notification switch: the app itself made the change, so a person has to finish the job.
            Notifier.notify(title: String(localized: "Codex hook config updated"),
                            body: String(localized: "Vibe Buddy rewrote ~/.codex/hooks.json. Codex silently disables hooks that change: type /hooks in Codex and re-trust them so the box keeps getting Codex events."))
        }
        refreshHookStates()
        if managesDaemon { supervisor.start() }
        streamTask = Task { [weak self] in await self?.followStatus() }
        Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            Task { @MainActor in
                self?.checkLink()
                // Once the grace period passes the status stream sends nothing new, so a timer has to notice the silence.
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

    /// Notify once after the link has been down for 30 seconds (debounced); reset when it's plugged back in.
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

    // MARK: Operations

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

    /// A firmware package the user obtained (the zip CI publishes): unzip to a temp directory and verify the three images before the confirmation dialog.
    /// The temp directory stays until flashing ends, since the daemon reads the files by path.
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

    // MARK: Agent hooks

    func refreshHookStates() {
        for agent in HookAgent.allCases {
            hookInstalled[agent] = HookInstaller.isInstalled(agent)
        }
    }

    func hookPresent(_ agent: HookAgent) -> Bool { HookInstaller.isPresent(agent) }
    func hookConfigModifiedAt(_ agent: HookAgent) -> Date? { HookInstaller.configModifiedAt(agent) }

    /// Returns the diff to confirm; the caller applies it after confirmation.
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
