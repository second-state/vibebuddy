import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// The Device page (canvas: "Mac · Device"; every other state on "App · Device & firmware states"). The current
/// Device tab's logic (`DeviceView`, `WifiSection`, `PairedComputersSection`, `BoxChoice`), copied.
struct KorekoreDevicePage: View {
    @ObservedObject var model: AppModel
    @State private var captured: Date?

    private var device: DeviceState? { model.status?.device }
    private var connected: Bool { device?.connected ?? false }
    /// Firmware that reports `TRANSFER 1` updates over any link (ADR-0016); older firmware over the cable only.
    private var canFlash: Bool { connected && (device?.network != true || device?.transfer == true) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                finding
                HStack(alignment: .top, spacing: 14) {
                    screen
                    theBox
                }
                firmwareState
                if connected, let device, device.boxKey != nil {
                    if device.network == true {
                        KCard(fill: Theme.Colors.cream) {
                            KNote(String(localized: "Plug in the cable to change the box's network, update its firmware or see its paired computers."))
                        }
                    }
                    HStack(alignment: .top, spacing: 14) {
                        KorekoreWifiCard(model: model, device: device)
                        pairedComputers(device)
                    }
                }
            }
            .padding(.bottom, Theme.Sticker.sm + 4)
        }
        .scrollIndicators(.never)
        .onChange(of: model.screenshot) { captured = model.screenshot == nil ? nil : Date() }
    }

    // MARK: Finding the box

    @ViewBuilder private var finding: some View {
        if let busy = device?.busy, !connected {
            let owner = busy.owner.isEmpty ? String(localized: "another computer") : busy.owner
            KCard {
                VStack(alignment: .leading, spacing: 8) {
                    KHeading(String(localized: "The box is with \(owner)."))
                    if busy.reason == "usb" {
                        KNote(String(localized: "The box is plugged into \(owner)."))
                    } else {
                        KNote(String(localized: "Take it over to drive it from here."))
                        Button(String(localized: "Take over")) { model.takeOver() }.kButton(.sun)
                    }
                }
            }
        }
        if let pin = device?.pin {
            KCard(fill: Theme.Colors.cream) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Only looking at \(pin.value), set by \(pin.variable).").font(.rounded(13))
                    KNote(String(localized: "To find the box on its own again, run `launchctl unsetenv \(pin.variable)` in Terminal, then restart VibeBuddy."))
                        .textSelection(.enabled)
                }
            }
        }
        if model.daemonAlive, !connected, !model.operationRunning, model.operation?.state != .replug {
            if let candidates = device?.candidates, !candidates.isEmpty {
                KCard {
                    VStack(alignment: .leading, spacing: 8) {
                        KHeading(String(localized: "Which one is your box?"))
                        KNote(String(localized: "Several devices are plugged in. Pick yours once; korekore remembers it."))
                        ForEach(candidates) { candidate in
                            KDivider()
                            HStack {
                                Text(verbatim: candidate.usbSerial.map { "\(candidate.port) · \($0)" } ?? candidate.port).font(.pixel(12))
                                Spacer()
                                if let serial = candidate.usbSerial {
                                    Button(String(localized: "This is the box")) { model.chooseBox(usbSerial: serial) }.kButton(.sun)
                                        .disabled(model.operationRunning)
                                }
                            }
                        }
                    }
                }
            } else if device?.pin == nil {
                KCard {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 10) {
                            KoreBadge("kore-sleep", gray: true)
                            KHeading(String(localized: "Box not found"))
                        }
                        KNote(String(localized: "Not showing up? Use a cable that carries data, not just power, and click Allow if macOS asks whether to let the accessory connect."))
                        KNote(String(localized: "Still nothing? Hold K0 on the box while you plug in the cable, then click Allow. The box starts in download mode with a dark screen, ready to be flashed with VibeBuddy firmware."))
                    }
                }
            }
        }
        if model.foreignFirmware {
            KCard(dashed: true) {
                VStack(alignment: .leading, spacing: 8) {
                    KHeading(String(localized: "The box isn't running korekore firmware"))
                    if model.firmwareDownloaded {
                        KNote(String(localized: "Flash korekore to use it; its current firmware will be replaced."))
                        Button(String(localized: "Flash korekore")) { KorekoreFlash.foreign(then: model.updateFirmware) }.kButton(.pink)
                            .disabled(model.operationRunning)
                    } else {
                        KorekoreFirmwareUnavailable(model: model)
                    }
                    Text("If flashing can't start: hold K0 while plugging in, to enter download mode.")
                        .font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                }
            }
        }
        if connected, let board = device?.unsupportedBoard {
            KCard {
                VStack(alignment: .leading, spacing: 8) {
                    KHeading(String(localized: "This is a \(board) board."))
                    KNote(String(localized: "Released firmware doesn't run on it, so no update is offered."))
                }
            }
        }
    }

    // MARK: The screen and the box

    /// The box's screen as last captured, at one point per pixel in the design's 352 × 264 frame: any other scale
    /// makes the pixel art uneven. Before the first capture, the box redrawn from the status.
    private var screen: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack {
                Theme.Colors.ink
                if let image = model.screenshot {
                    Image(nsImage: image).interpolation(.none).resizable().frame(width: 320, height: 240)
                } else {
                    MiniBox(picture: BoxPicture.derive(status: model.status, daemonAlive: model.daemonAlive), scale: 1, framed: false)
                }
            }
            .frame(width: 352, height: 264)
            .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.md - Theme.Stroke.bold))
            .padding(Theme.Stroke.bold)
            .sticker(radius: Theme.Radius.md, fill: Theme.Colors.ink, stroke: Theme.Stroke.bold)
            HStack(spacing: 8) {
                Button(String(localized: "Refresh")) { model.takeScreenshot() }.kButton()
                    .disabled(model.screenshotBusy || model.operationRunning || !connected)
                Button(String(localized: "Save image")) { model.saveScreenshot() }.kButton()
                    .disabled(model.screenshot == nil)
                Spacer()
                if model.screenshotBusy {
                    KTag(String(localized: "Refreshing…"))
                } else if let captured {
                    KTag(String(localized: "captured \(captured.formatted(date: .omitted, time: .shortened))"))
                }
            }
            .frame(width: 358)
        }
    }

    private var theBox: some View {
        KCard {
            VStack(alignment: .leading, spacing: 10) {
                KHeading(String(localized: "The box"))
                field(String(localized: "Link")) { Text(verbatim: linkText) }
                if !connected, let relayRefusal {
                    KNote(relayRefusal, color: Theme.Colors.korePinkText)
                }
                field(String(localized: "Firmware")) { firmwareLine }
                field(String(localized: "Box screens")) { screensChoice }
                if connected, device?.screens == nil {
                    KNote(String(localized: "Update the box's firmware to choose its screens."))
                }
                Spacer(minLength: 8)
                if model.firmwareUpdateAvailable, connected, !canFlash {
                    KNote(String(localized: "Plug in the cable to update the box's firmware."))
                } else if model.firmwareUpdateAvailable, let offer = model.offeredFirmware {
                    Button(String(localized: "Update to \(offer.version)")) { KorekoreFlash.update(offer, then: model.updateFirmware) }
                        .kButton(.pink, size: 13).frame(maxWidth: .infinity)
                        .disabled(model.operationRunning || !connected)
                }
                Button { model.identify() } label: { Text("Make the box blink").frame(maxWidth: .infinity) }
                    .kButton(.sun, size: 13)
                    .disabled(model.operationRunning || !connected)
                Button { flashFromFile() } label: { Text("Flash from file…").frame(maxWidth: .infinity) }
                    .kButton(.paper, size: 13)
                    .disabled(model.operationRunning || !canFlash)
            }
            .frame(minHeight: 290, alignment: .top)
        }
    }

    /// Which set of screens the box draws (ADR-0021), as the box last said; firmware that predates the choice says nothing.
    @ViewBuilder private var screensChoice: some View {
        if let screens = device?.screens {
            KMenu(selection: screens, options: [("robot", String(localized: "Robot")), ("korekore", "korekore")]) { style in
                if style != screens { model.setScreens(style) }
            }
            .disabled(model.operationRunning || !connected)
        } else {
            Text(verbatim: "—")
        }
    }

    private func field<Value: View>(_ label: String, @ViewBuilder value: () -> Value) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: label).font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
            value().font(.pixel(13))
        }
    }

    @ViewBuilder private var firmwareLine: some View {
        let version = device?.firmwareVersion ?? model.boxFirmware ?? "—"
        if model.firmwareUpdateAvailable, let offer = model.offeredFirmware {
            Text(verbatim: "\(version) ") + Text(String(localized: "· \(offer.version) available").uppercased()).foregroundColor(Theme.Colors.waitText)
        } else if connected, model.offeredFirmware != nil || model.updates?.lastCheck != nil {
            Text(verbatim: "\(version) ") + Text(String(localized: "· up to date").uppercased()).foregroundColor(Theme.Colors.doneText)
        } else {
            Text(verbatim: version)
        }
    }

    /// Why the Relay won't carry the link, shown while the box is away (ADR-0020).
    private var relayRefusal: String? {
        switch device?.relayRefused {
        case nil: nil
        case "signed_out": String(localized: "Sign in to use the Relay.")
        case "plan": String(localized: "The Relay needs the Paid plan.")
        default: String(localized: "The Relay can't check the account right now. VibeBuddy tries again later.")
        }
    }

    private var linkText: String {
        guard let device else { return String(localized: "daemon isn't running") }
        guard device.connected else {
            return device.candidates?.isEmpty == false ? String(localized: "Several devices found") : String(localized: "Box not found")
        }
        if device.relay == true { return String(localized: "Relay") }
        let port = (device.port ?? "").replacingOccurrences(of: "/dev/cu.", with: "")
        if device.network == true { return "\(String(localized: "Wi-Fi")) · \(port)" }
        return "USB · \(port)"
    }

    // MARK: Firmware writes

    @ViewBuilder private var firmwareState: some View {
        if let operation = model.operation, operation.kind == .firmware {
            switch operation.state {
            case .running:
                KCard {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            KHeading(String(localized: "Updating the firmware…"))
                            Spacer()
                            KTag(operation.progress.formatted(.percent.precision(.fractionLength(0))), color: Theme.Colors.ink)
                        }
                        KBar(operation.progress, fill: Theme.Colors.sun)
                    }
                }
            case .done:
                KCard(outline: Theme.Colors.doneText) {
                    KTag(String(localized: "Firmware flashed · the box has restarted"), color: Theme.Colors.doneText)
                }
            case .replug:
                KCard(fill: Theme.Colors.cream) {
                    VStack(alignment: .leading, spacing: 6) {
                        KHeading(String(localized: "One more step"))
                        KNote(String(localized: "The firmware is on the box, but it didn't start. Unplug the box and plug it back in."))
                    }
                }
            case .failed:
                KCard(outline: Theme.Colors.korePinkText) {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("The update didn't finish").font(.rounded(15)).foregroundStyle(Theme.Colors.korePinkText)
                        KNote(String(localized: "Failed: \(operation.message)"))
                        if device?.transfer != true {
                            KNote(String(localized: "Before retrying, hold K0 on the box and replug the cable to put it in download mode."))
                        }
                        HStack {
                            Button(String(localized: "Try again")) { model.updateFirmware() }.kButton().disabled(!canFlash)
                            Button(String(localized: "Report a problem…")) { model.reportProblem() }.kButton()
                        }
                    }
                }
            }
        } else if device?.rolledBack == true {
            KCard(outline: Theme.Colors.korePinkText) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("The update didn't take").font(.rounded(15)).foregroundStyle(Theme.Colors.korePinkText)
                    KNote(String(localized: "The box went back to firmware \(model.boxFirmware ?? "—") and is fine. Try again later or report a problem."))
                    HStack {
                        Button(String(localized: "Try again")) { model.updateFirmware() }.kButton().disabled(!canFlash)
                        Button(String(localized: "Report a problem…")) { model.reportProblem() }.kButton()
                    }
                }
            }
        } else if !model.foreignFirmware, model.offeredFirmware?.newerThanBox == true, !model.firmwareDownloaded {
            KCard { KorekoreFirmwareUnavailable(model: model) }
        }
    }

    // MARK: Paired computers

    private func pairedComputers(_ device: DeviceState) -> some View {
        KCard {
            VStack(alignment: .leading, spacing: 8) {
                KHeading(String(localized: "Paired computers"))
                if device.network == true {
                    KNote(String(localized: "Plug in the cable to see the paired computers."))
                }
                ForEach(device.pairedComputers ?? []) { computer in
                    HStack {
                        Text(verbatim: computer.name.isEmpty ? String(localized: "Unnamed computer") : computer.name).font(.rounded(13, weight: 700))
                        Spacer()
                        if computer.key == device.computerKey {
                            KTag(String(localized: "This Mac"), color: Theme.Colors.korePinkText)
                        } else {
                            Button(String(localized: "Remove")) { model.unpair(key: computer.key) }.kButton()
                                .disabled(model.operationRunning)
                        }
                    }
                    .padding(.vertical, 6)
                    .padding(.horizontal, 10)
                    .overlay(RoundedRectangle(cornerRadius: Theme.Radius.sm).strokeBorder(Theme.Colors.line, lineWidth: Theme.Stroke.base))
                }
                KNote(String(localized: "A computer pairs when it's plugged in. To forget them all, use COMPUTERS in the box's menu."))
            }
        }
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
        let current = model.boxFirmware ?? String(localized: "unknown")
        let offered = Firmware.label(version: package.version, build: package.build) ?? package.build
        KorekoreAlert.ask(
            title: String(localized: "Flash this firmware?"),
            message: String(localized: "The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes."),
            buttons: [String(localized: "Flash"), String(localized: "Cancel")],
            facts: [(String(localized: "Firmware package"), offered), (String(localized: "Box now"), current)]) { [model] answer in
            if answer.button == 0 { model.flashFirmware(package) }
        }
    }
}

/// Why there's no firmware to flash yet, and what to do about it.
struct KorekoreFirmwareUnavailable: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let updates = model.updates, updates.enabled {
                if let error = updates.error {
                    KTag(String(localized: "Couldn't get the firmware: \(error)"), color: Theme.Colors.korePinkText)
                } else if model.offeredFirmware != nil {
                    KTag(String(localized: "Downloading the firmware…"))
                } else {
                    KTag(String(localized: "Looking for firmware…"))
                }
                Button(String(localized: "Check again")) { model.checkForUpdates() }.kButton()
            } else {
                KNote(String(localized: "Update checks are off, so korekore doesn't download firmware. Turn them on in General, or get it from Releases."))
            }
            Link(destination: Resources.releasesPage) {
                Text("Releases").font(.rounded(13, weight: 400)).foregroundStyle(Theme.Colors.korePinkText)
            }
        }
    }
}

/// The Wi-Fi network the box joins (ADR-0012). Set over the cable only; the password goes to the box and is kept
/// nowhere else, so it is never shown again. `WifiSection`'s logic.
struct KorekoreWifiCard: View {
    @ObservedObject var model: AppModel
    let device: DeviceState
    @State private var ssid = ""
    @State private var password = ""
    @State private var editing = false

    static func wifiFailure(_ why: String) -> String {
        switch why {
        case "wrong_password": return String(localized: "The network turned the box away; the password is probably wrong. Check it, then join again.")
        case "not_found": return String(localized: "The box can't find this network. It only joins 2.4 GHz Wi-Fi: check the name, and that the box is in range.")
        case "security": return String(localized: "The box can't join this network's kind of security. It joins WPA2 and open networks.")
        case "password": return String(localized: "The password must be 8 to 63 characters.")
        case "ssid": return String(localized: "The network name is too long: 32 bytes at most.")
        default: return String(localized: "Couldn't join (\(why)). Check the network name and password, then join again.")
        }
    }

    var body: some View {
        KCard {
            VStack(alignment: .leading, spacing: 8) {
                KHeading(String(localized: "Wi-Fi"))
                row(String(localized: "Network")) {
                    Text(verbatim: device.wifiNetwork ?? String(localized: "Not set")).font(.rounded(13, weight: 700))
                }
                if let network = device.wifiNetwork {
                    if let why = device.wifiError {
                        KTag(String(localized: "Couldn't join"), color: Theme.Colors.korePinkText)
                        KNote(Self.wifiFailure(why))
                    } else if let address = device.wifiAddress {
                        row(String(localized: "Status")) { KTag(String(localized: "Joined · \(address)"), color: Theme.Colors.doneText) }
                    } else {
                        HStack {
                            Text("Joining \(network)…").font(.rounded(13))
                            Spacer()
                            KTag(String(localized: "Up to 15 seconds"))
                        }
                        KBar(0.35, fill: Theme.Colors.sky)
                    }
                }
                if device.network == true {
                    KNote(String(localized: "Plug in the cable to change the box's network."))
                } else if device.wifiNetwork != nil && device.wifiError == nil && !editing {
                    HStack(spacing: 8) {
                        Button(String(localized: "Change network")) { editing = true }.kButton()
                        Button(String(localized: "Forget")) { model.forgetWifi() }.kButton().disabled(model.operationRunning)
                    }
                } else {
                    HStack(spacing: 8) {
                        KMenu(selection: ssid, options: [("", String(localized: "Choose…"))]
                              + (device.wifiSeen ?? []).map { ($0.ssid, "\($0.ssid)  \($0.rssi) dBm") }) { ssid = $0 }
                        Button(String(localized: "Scan")) { model.scanWifi() }.kButton().disabled(model.operationRunning)
                    }
                    .onAppear { if device.wifiSeen?.isEmpty ?? true { model.scanWifi() } }
                    KField(String(localized: "Network name"), text: $ssid)
                    KField(String(localized: "Password"), text: $password, secure: true)
                    HStack(spacing: 8) {
                        Button(String(localized: "Join")) {
                            model.setWifi(ssid: ssid, password: password)
                            password = ""
                            editing = false
                        }
                        .kButton(.sun)
                        .disabled(ssid.isEmpty || ssid.utf8.count > 32 || password.utf8.count > 63 || model.operationRunning)
                        if device.wifiNetwork != nil {
                            Button(String(localized: "Forget")) { model.forgetWifi() }.kButton().disabled(model.operationRunning)
                        }
                    }
                }
                Text("On the same network, a paired computer drives the box without the cable.")
                    .font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func row<Value: View>(_ label: String, @ViewBuilder value: () -> Value) -> some View {
        HStack {
            Text(verbatim: label).font(.rounded(13, weight: 400)).foregroundStyle(Theme.Colors.muted)
            Spacer()
            value()
        }
    }
}
