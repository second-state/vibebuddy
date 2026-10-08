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
    /// nil when Ghostty isn't installed: there is nothing to ask for.
    @Published private(set) var ghosttyAccess: TerminalAccess.State?
    @Published private(set) var launchAtLogin = LoginItem.isEnabled
    @Published private(set) var previewingVoice: String?
    /// Serial port open but no build ID ever reported: the box isn't running our firmware (a factory box), so offer to flash it.
    @Published private(set) var foreignFirmware = false

    let client = DaemonClient()
    let supervisor = DaemonSupervisor()
    let preview = VoicePreview()
    let updater = Updater()
    /// Whether the app manages the daemon itself; false when an old LaunchAgent was found and the user kept it.
    var managesDaemon = true

    private var streamTask: Task<Void, Never>?
    private var linkLostSince: Date?
    private var linkNotified = false
    /// A voice the user asked for while changing the UI language, to write once the restarted app sees the box.
    /// Taken out of the defaults at start, so it is tried in this run only and never surprises the user days later.
    private var pendingVoice: String?
    private static let pendingVoiceKey = "pendingVoice"
    /// The last firmware version a notification was posted for, so each new one is announced once.
    private static let notifiedFirmwareKey = "notifiedFirmware"

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
        pendingVoice = UserDefaults.standard.string(forKey: Self.pendingVoiceKey)
        UserDefaults.standard.removeObject(forKey: Self.pendingVoiceKey)
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
        self.status = status
        daemonAlive = true
        refreshMenu()
        refreshForeignFirmware()
        notifyNewFirmware(status)
        updater.follow(checks: status.updates?.enabled ?? false)
        // Wait for the build ID: the box has answered, not just had its port opened (which resets it).
        if let voice = pendingVoice, status.device.connected, status.device.firmwareBuild != nil, !operationRunning {
            pendingVoice = nil
            writeVoice(voice)
        }
    }

    private func refreshForeignFirmware() {
        let foreign = status.map { Firmware.foreign($0.device) } ?? false
        if foreign != foreignFirmware { foreignFirmware = foreign }
    }

    private func refreshMenu() {
        menu = MenuState.derive(status: status, daemonAlive: daemonAlive)
    }

    /// One notification per new firmware version, only while the box is there to take it and once it's downloaded.
    private func notifyNewFirmware(_ status: Status) {
        guard status.device.connected, Firmware.updateAvailable(status.updates), let offer = status.updates?.firmware,
              UserDefaults.standard.string(forKey: Self.notifiedFirmwareKey) != offer.version else { return }
        UserDefaults.standard.set(offer.version, forKey: Self.notifiedFirmwareKey)
        Notifier.notify(title: String(localized: "Box firmware \(offer.version) is available"),
                        body: String(localized: "Open Settings → Device to update the box."))
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

    func chooseBox(usbSerial: String) { run { try await self.client.chooseBox(usbSerial: usbSerial) } }

    func setNotifyLink(_ enabled: Bool) {
        guard var config = status?.config else { return }
        config.notifyLink = enabled
        run { _ = try await self.client.putConfig(config) }
    }

    func setCheckUpdates(_ enabled: Bool) {
        guard var config = status?.config else { return }
        config.checkUpdates = enabled
        run { _ = try await self.client.putConfig(config) }
    }

    /// The daemon reads the manifest (firmware, and the App on Linux); on the Mac, Sparkle checks the App with its own window.
    func checkForUpdates() {
        run { try await self.client.checkForUpdates() }
        updater.checkForUpdates()
    }

    /// Installs a newer App: Sparkle when this build has it, otherwise the download page.
    func installApp(_ offer: AppOffer) {
        if updater.available {
            updater.checkForUpdates()
        } else if let url = URL(string: offer.url) {
            NSWorkspace.shared.open(url)
        }
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

    /// Restarts the app for a new UI language; the restarted app writes `voice` once it sees the box, since a write
    /// in progress would be cut off by the restart.
    func restart(writingVoice voice: String?) {
        if let voice { UserDefaults.standard.set(voice, forKey: Self.pendingVoiceKey) }
        AppRelaunch.relaunch()
    }

    func writeVoice(_ id: String) {
        guard let pack = Resources.voicePack(id) else { lastError = String(localized: "This app has no voice pack for \(id)"); return }
        run { try await self.client.writeVoicePack(self.addressed(pack, id: id).data) }
    }

    /// The form of address picked for a language; nil for none.
    func formOfAddress(_ language: VoiceLanguage) -> String? {
        UserDefaults.standard.string(forKey: "address.\(language.rawValue)")
    }

    /// Picks a form of address and, when the box wears a Character of that language, writes it again
    /// so the box says it.
    func setFormOfAddress(_ form: String?, for language: VoiceLanguage) {
        UserDefaults.standard.set(form, forKey: "address.\(language.rawValue)")
        objectWillChange.send()
        guard let current = status?.device.voice else { return }
        if VoiceCatalogEntry.language(ofVoice: current) == language, current != "builtin" {
            writeVoice(current)
        } else if current == "custom", let saved = savedCustomCharacter, VoiceCatalogEntry.language(ofVoice: saved.lender) == language {
            writeCustomCharacter(look: saved.look, lender: saved.lender)
        } else if current == Self.robotID, VoiceCatalogEntry.language(ofVoice: robotLender) == language {
            writeRobot(lender: robotLender)
        }
    }

    static let robotID = "robot"
    private static let robotLenderKey = "robot.lender"
    /// Who lends the robot voice and lines: "builtin" (the five fixed lines it shipped with) or a Character id.
    var robotLender: String { UserDefaults.standard.string(forKey: Self.robotLenderKey) ?? "builtin" }

    /// Whether the box wears the robot: written as one, or still on the firmware's own default.
    var wearsRobot: Bool { [Self.robotID, "builtin"].contains(status?.device.voice) }

    /// Writes the robot, the default Character: drawn by the box itself, so the pack carries no look,
    /// only the voice and lines of `lender` (with the form of address of its language) or, for
    /// "builtin", the five fixed lines it shipped with.
    func writeRobot(lender: String) {
        let pack: Data?
        if lender == "builtin" {
            pack = Resources.voicePack(Self.robotID)?.data
        } else {
            pack = Resources.voicePack(lender).flatMap { addressed($0, id: lender).withoutLook(id: Self.robotID) }
        }
        guard let pack else { lastError = String(localized: "This app has no voice pack for \(lender)"); return }
        UserDefaults.standard.set(lender, forKey: Self.robotLenderKey)
        objectWillChange.send()
        writePack(pack)
    }

    /// Character `id` with the user's form of address in its language, or as it is with none.
    func addressed(_ pack: VoicePack, id: String) -> VoicePack {
        guard let language = VoiceCatalogEntry.language(ofVoice: id), let form = formOfAddress(language),
              let variant = Resources.addressPack(id, form: form),
              let data = pack.withAddress(variant), let addressed = VoicePack(data: data) else { return pack }
        return addressed
    }

    /// Writes a pack the app put together itself, such as a custom Character.
    func writePack(_ data: Data) {
        run { try await self.client.writeVoicePack(data) }
    }

    private static let customLookURL = Resources.applicationSupport.appendingPathComponent("custom-look.bin")
    private static let customLenderKey = "custom.lender"

    /// The user's own Character as last written: its look and the Character lending voice and lines.
    var savedCustomCharacter: (look: Data, lender: String)? {
        guard let look = try? Data(contentsOf: Self.customLookURL),
              let lender = UserDefaults.standard.string(forKey: Self.customLenderKey) else { return nil }
        return (look, lender)
    }

    /// Writes the user's own Character, with the form of address of the lender's language, and keeps
    /// it so the Character tab can show it later. False if the lender can't lend.
    @discardableResult
    func writeCustomCharacter(look: Data, lender: String) -> Bool {
        guard let base = Resources.voicePack(lender), let pack = addressed(base, id: lender).withLook(look, id: "custom") else { return false }
        try? FileManager.default.createDirectory(at: Resources.applicationSupport, withIntermediateDirectories: true)
        try? look.write(to: Self.customLookURL)
        UserDefaults.standard.set(lender, forKey: Self.customLenderKey)
        writePack(pack)
        return true
    }

    func togglePreview(_ id: String) {
        guard let pack = Resources.voicePack(id) else { lastError = String(localized: "This app has no voice pack for \(id)"); return }
        preview.toggle(pack: pack)
        previewingVoice = preview.playingVoice
    }

    /// The box's firmware as shown in the interface: version and build ID.
    var boxFirmware: String? {
        Firmware.label(version: status?.device.firmwareVersion, build: status?.device.firmwareBuild)
    }

    var updates: UpdateStatus? { status?.updates }
    var offeredFirmware: FirmwareOffer? { updates?.firmware }
    var firmwareUpdateAvailable: Bool { Firmware.updateAvailable(updates) }
    /// The offered firmware is on disk, so a box (including a factory one) can be flashed with it.
    var firmwareDownloaded: Bool { Firmware.files(of: offeredFirmware) != nil }

    /// Flashes the firmware the daemon downloaded from the update manifest.
    func updateFirmware() {
        guard let files = Firmware.files(of: offeredFirmware) else { lastError = String(localized: "The firmware hasn't been downloaded yet"); return }
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

    // MARK: Terminal tabs

    /// Reads the current answer without prompting.
    func refreshGhosttyAccess() {
        guard TerminalAccess.ghosttyURL != nil else {
            ghosttyAccess = nil
            return
        }
        Task {
            let state = await Task.detached { TerminalAccess.check(ask: false) }.value
            ghosttyAccess = state
        }
    }

    /// Shows the system prompt. macOS only asks about a running app, so Ghostty is started first if needed.
    func allowGhosttyAccess() {
        guard let url = TerminalAccess.ghosttyURL else { return }
        Task {
            if !TerminalAccess.ghosttyRunning {
                let configuration = NSWorkspace.OpenConfiguration()
                configuration.activates = false
                _ = try? await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
                for _ in 0..<50 where !TerminalAccess.ghosttyRunning {
                    try? await Task.sleep(for: .milliseconds(100))
                }
            }
            let state = await Task.detached { TerminalAccess.check(ask: true) }.value
            ghosttyAccess = state
        }
    }

    func openAutomationSettings() {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation") {
            NSWorkspace.shared.open(url)
        }
    }
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
