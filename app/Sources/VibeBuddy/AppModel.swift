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
    /// Keyed by bundle id, for the supported terminals installed on this Mac.
    @Published private(set) var terminalAccess: [String: TerminalAccess.State] = [:]
    @Published private(set) var launchAtLogin = LoginItem.isEnabled
    @Published private(set) var previewingVoice: String?
    /// Serial port open but no build ID ever reported: the box isn't running our firmware (a factory box), so offer to flash it.
    @Published private(set) var foreignFirmware = false
    /// The daemon runs the experimental resident session, so the Conversation tab shows. Asked each time the daemon
    /// comes up, since only its environment at start decides; kept while it is down, so the tab can say so.
    @Published private(set) var residentEnabled = false
    /// The Characters made on this computer, and the voices one can be made with (ADR-0020); asked when the daemon
    /// comes up and whenever a job changes state.
    @Published private(set) var madeCharacters: [MadeCharacter] = []
    @Published private(set) var characterVoices: [CharacterVoice] = []

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
    /// The korekore UI words its notifications its own way (ADR-0021, canvas: "SYSTEM NOTIFICATIONS").
    private static let korekore = AppUI.atLaunch() == .korekore

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
                if Self.korekore {
                    Notifier.notify(title: String(localized: "The daemon failed to start three times"),
                                    body: String(localized: "Open korekore to restart it or export diagnostics."))
                } else {
                    Notifier.notify(title: "VibeBuddy", body: String(localized: "The daemon failed to start three times in a row. Click the menu bar icon to restart it."))
                }
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
                            body: Self.korekore
                                ? String(localized: "Trust korekore's hooks again with /hooks in Codex.")
                                : String(localized: "VibeBuddy rewrote ~/.codex/hooks.json. Codex silently disables hooks that change: type /hooks in Codex and re-trust them so the box keeps getting Codex events."))
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
        if !daemonAlive { refreshResident() }
        if !daemonAlive || status.making?.id != self.status?.making?.id || status.making?.state != self.status?.making?.state {
            refreshMadeCharacters()
        }
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

    private func refreshResident() {
        Task {
            if let info = try? await client.resident() { residentEnabled = info.enabled }
        }
    }

    #if DEBUG
    /// Debug builds only: shows a sample status without a daemon, for the korekore page snapshots.
    func showSample(_ status: Status) {
        self.status = status
        daemonAlive = true
        refreshMenu()
    }
    #endif

    func refreshMadeCharacters() {
        Task {
            if let made = try? await client.madeCharacters() { madeCharacters = made }
            if characterVoices.isEmpty, let voices = try? await client.characterVoices() { characterVoices = voices }
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
                        body: Self.korekore ? String(localized: "Update it from Settings › Device.") : String(localized: "Open Settings → Device to update the box."))
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
            Notifier.notify(title: String(localized: "Box disconnected"),
                            body: Self.korekore ? String(localized: "The box has been gone for 30 seconds.")
                                : String(localized: "VibeBuddy hasn't seen the box for 30 seconds. Check the USB cable."))
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

    func setScreens(_ style: String) { run { try await self.client.setScreens(style) } }

    func chooseBox(usbSerial: String) { run { try await self.client.chooseBox(usbSerial: usbSerial) } }

    func unpair(key: String) { run { try await self.client.unpair(key: key) } }

    func takeOver() { run { try await self.client.takeOver() } }

    func setWifi(ssid: String, password: String) { run { try await self.client.setWifi(ssid: ssid, password: password) } }

    func forgetWifi() { run { try await self.client.forgetWifi() } }

    func scanWifi() { run { try await self.client.scanWifi() } }

    func setNotifyLink(_ enabled: Bool) {
        guard var config = status?.config else { return }
        config.notifyLink = enabled
        run { _ = try await self.client.putConfig(config) }
    }

    func setRelay(_ enabled: Bool) {
        guard var config = status?.config else { return }
        config.relay = enabled
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
        let address = VoiceCatalogEntry.language(ofVoice: id).flatMap(formOfAddress)
        run { try await self.client.writeVoicePack(self.addressed(pack, id: id).data, address: address) }
    }

    /// The language of the Character the box wears, the one whose form of address it says; nil for the robot, an
    /// unknown Character or no box.
    var boxLanguage: VoiceLanguage? {
        guard let current = status?.device.voice, current != "builtin" else { return nil }
        if let language = VoiceCatalogEntry.language(ofVoice: current) { return language }
        if let made = madeCharacters.first(where: { $0.id == current }), made.complete { return language(of: made) }
        if current == "custom", let saved = savedCustomCharacter { return VoiceCatalogEntry.language(ofVoice: saved.lender) }
        return nil
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
        } else if let made = madeCharacters.first(where: { $0.id == current }), made.complete, self.language(of: made) == language {
            writeMadeCharacter(made)
        } else if current == "custom", let saved = savedCustomCharacter, VoiceCatalogEntry.language(ofVoice: saved.lender) == language {
            writeCustomCharacter(look: saved.look, lender: saved.lender)
        }
    }

    static let robotID = "robot"
    /// Older versions let another Character lend the robot its voice and lines; a box written that way
    /// still says that Character's lines until the robot is written again.
    private static let robotLenderKey = "robot.lender"
    var robotBorrowsVoice: Bool { ![nil, "builtin"].contains(UserDefaults.standard.string(forKey: Self.robotLenderKey)) }

    /// Whether the box wears the robot: written as one, or still on the firmware's own default.
    var wearsRobot: Bool { [Self.robotID, "builtin"].contains(status?.device.voice) }

    /// Writes the robot, the default Character: drawn by the box itself, so the pack carries no look,
    /// only its own voice, the built-in one, and the five fixed lines it shipped with.
    func writeRobot() {
        guard let pack = Resources.voicePack(Self.robotID)?.data else {
            lastError = String(localized: "This app has no voice pack for \(Self.robotID)")
            return
        }
        UserDefaults.standard.removeObject(forKey: Self.robotLenderKey)
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

    /// Writes a pack the app put together itself, such as a custom Character with the Character lending it
    /// voice, persona and lines, and the form of address they carry.
    func writePack(_ data: Data, lender: String? = nil, address: String? = nil) {
        run { try await self.client.writeVoicePack(data, lender: lender, address: address) }
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
        writePack(pack, lender: lender, address: VoiceCatalogEntry.language(ofVoice: lender).flatMap(formOfAddress))
        return true
    }

    /// Forgets the user's own Character. A box wearing it goes back to the Character that lent it voice and lines.
    func deleteCustomCharacter() {
        let lender = savedCustomCharacter?.lender
        try? FileManager.default.removeItem(at: Self.customLookURL)
        UserDefaults.standard.removeObject(forKey: Self.customLenderKey)
        objectWillChange.send()
        if status?.device.voice == CustomCharacterCard.id, let lender { writeVoice(lender) }
    }

    // MARK: Making a Character

    /// The language a made Character speaks: its voice's, else its base preset's.
    func language(of made: MadeCharacter) -> VoiceLanguage? {
        characterVoices.first { $0.id == made.voice }?.voiceLanguage ?? VoiceCatalogEntry.language(ofVoice: made.base)
    }

    func characterVoice(_ id: String) -> CharacterVoice? { characterVoices.first { $0.id == id } }

    /// The look a made Character wears: the one the user brought for it, else its base preset's.
    func look(of made: MadeCharacter) -> Data? {
        (try? Data(contentsOf: Resources.madeCharacterDirectory(made.id).appendingPathComponent("look.bin")))
            ?? Resources.voicePack(made.base)?.look
    }

    /// Starts making a Character (`look` is the user's, or nil for the base's); nil when the daemon took it, else why not.
    func makeCharacter(_ request: CharacterRequest, look: Data?) async -> String? {
        do {
            let started = try await client.makeCharacter(request)
            if let look {
                let directory = Resources.madeCharacterDirectory(started.id)
                try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                try? look.write(to: directory.appendingPathComponent("look.bin"))
            }
            refreshMadeCharacters()
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    /// Makes the rest of a Character whose job stopped: the same request again, with its id.
    func resumeCharacter(_ id: String) {
        let manifest = Resources.madeCharacterDirectory(id).appendingPathComponent("manifest.json")
        guard let data = try? Data(contentsOf: manifest), var request = try? JSONDecoder().decode(CharacterRequest.self, from: data) else {
            lastError = String(localized: "Couldn't read what was asked for this Character.")
            return
        }
        request.id = id
        Task { if let problem = await makeCharacter(request, look: nil) { lastError = problem } }
    }

    /// Deletes a made Character, its packs and look with it; the caller asks first.
    func deleteMadeCharacter(_ made: MadeCharacter) {
        Task {
            do { try await client.deleteCharacter(made.id) } catch { lastError = String(localized: "Couldn't delete “\(made.name)”: \(error.localizedDescription)") }
            refreshMadeCharacters()
        }
    }

    /// Writes a made Character to the box: the daemon's pack of its lines, said with the form of address picked for its
    /// language, wearing its look. Works on any plan: nothing is synthesized again.
    func writeMadeCharacter(_ made: MadeCharacter) {
        let directory = Resources.madeCharacterDirectory(made.id)
        let form = language(of: made).flatMap(formOfAddress)
        let variant = form.flatMap { try? Data(contentsOf: directory.appendingPathComponent("address").appendingPathComponent("\($0).bin")) }
        guard let pack = try? Data(contentsOf: directory.appendingPathComponent("pack.bin")),
              let data = VoicePack.made(pack, variant: variant, look: look(of: made), id: made.id) else {
            lastError = String(localized: "“\(made.name)” isn't finished yet.")
            return
        }
        writePack(data, address: form)
    }

    /// Tries a voice in the user's own account and, when it speaks, keeps it and plays what it said.
    func addOwnVoice(provider: String, voice: String, name: String, text: String) async throws -> CharacterVoice {
        let (added, wav) = try await client.addOwnVoice(provider: provider, voice: voice, name: name, text: text)
        characterVoices.removeAll { $0.id == added.id }
        characterVoices.append(added)
        preview.play(wav: wav, as: added.id)
        previewingVoice = preview.playingVoice
        return added
    }

    func deleteOwnVoice(_ id: String) async {
        do {
            try await client.deleteOwnVoice(id)
            characterVoices.removeAll { $0.id == id }
        } catch {
            lastError = String(localized: "Couldn't remove the voice: \(error.localizedDescription)")
        }
    }

    func togglePreview(_ id: String) {
        guard let pack = Resources.voicePack(id) else { lastError = String(localized: "This app has no voice pack for \(id)"); return }
        preview.toggle(pack: pack)
        previewingVoice = preview.playingVoice
    }

    /// Both sides' build IDs, the voice and the OS: summary.txt in exported diagnostics, and the
    /// versions a problem report starts with. No logs and nothing an agent did.
    var diagnosticsSummary: String {
        """
        App \(Resources.displayVersion)
        daemon \(status?.daemon.build ?? "not connected")
        firmware \(boxFirmware ?? "—")
        offered firmware \(offeredFirmware?.version ?? "—")
        voice \(status?.device.voice ?? "—")
        macOS \(ProcessInfo.processInfo.operatingSystemVersionString)
        """
    }

    /// Opens a new GitHub issue in the browser with the versions already filled in.
    func reportProblem() {
        NSWorkspace.shared.open(IssueReport.url(summary: diagnosticsSummary))
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

    /// Reads the current answers without prompting.
    func refreshTerminalAccess() {
        let terminals = TerminalAccess.installed
        Task {
            var states: [String: TerminalAccess.State] = [:]
            for terminal in terminals {
                states[terminal.bundleID] = await Task.detached { TerminalAccess.check(terminal, ask: false) }.value
            }
            terminalAccess = states
        }
    }

    /// Shows the system prompt. macOS only asks about a running app, so the terminal is started first if needed.
    func allowTerminalAccess(_ terminal: TerminalAccess.Terminal) {
        guard let url = terminal.url else { return }
        Task {
            if !terminal.running {
                let configuration = NSWorkspace.OpenConfiguration()
                configuration.activates = false
                _ = try? await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
                for _ in 0..<50 where !terminal.running {
                    try? await Task.sleep(for: .milliseconds(100))
                }
            }
            terminalAccess[terminal.bundleID] = await Task.detached { TerminalAccess.check(terminal, ask: true) }.value
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
