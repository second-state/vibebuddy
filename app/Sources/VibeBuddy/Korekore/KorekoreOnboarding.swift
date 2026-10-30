import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// First-run onboarding in the korekore look (canvas: "Mac · Onboarding welcome", "Onboarding · find the box,
/// connect, pick, Wi-Fi", and the end of onboarding on "App · alerts, notifications, banners").
struct KorekoreOnboarding: View {
    @ObservedObject var model: AppModel
    let finish: () -> Void
    @State private var step: Int
    @State private var showWhatItSends = false
    @State private var notThisOne = false
    @State private var blinked = false
    @State private var chosen: Set<HookAgent> = []
    @State private var queue: [HookAgent] = []
    @State private var pendingPlan: HookInstaller.Plan?
    @State private var writing: String?
    @State private var ssid = ""
    @State private var password = ""
    @State private var joined = false

    /// Snapshots only: nothing is sent to the box when a page appears (no blink, no Wi-Fi scan).
    let passive: Bool

    init(model: AppModel, finish: @escaping () -> Void, step: Int = 0, passive: Bool = false) {
        self.model = model
        self.finish = finish
        self.passive = passive
        _step = State(initialValue: step)
    }

    private var steps: [String] {
        [String(localized: "Welcome"), String(localized: "Find the box"), String(localized: "Connect agents"),
         String(localized: "Pick a character"), String(localized: "Wi-Fi"), String(localized: "Launch at login"), String(localized: "Done")]
    }

    private var flashing: Bool { model.operation?.kind == .firmware && model.operation?.state == .running }
    private var device: DeviceState? { model.status?.device }
    private var connected: Bool { device?.connected ?? false }

    var body: some View {
        KorekoreOnboardingFrame(steps: steps, current: step, goTo: flashing ? nil : { step = $0 }) {
            Group {
                switch step {
                case 0: welcome
                case 1: findTheBox
                case 2: connectAgents
                case 3: pickACharacter
                case 4: wifi
                case 5: launchAtLogin
                default: allSet
                }
            }
        }
        .sheet(item: Binding(get: { pendingPlan.map(PlanBox.init) }, set: { pendingPlan = $0?.plan })) { box in
            KorekorePlanSheet(plan: box.plan, confirm: { model.applyHookPlan(box.plan); nextPlan() }, cancel: nextPlan)
        }
    }

    private func title(_ text: String, size: CGFloat = 30) -> some View {
        Text(verbatim: text).font(.rounded(size)).fixedSize(horizontal: false, vertical: true)
    }

    // MARK: Welcome

    private var welcome: some View {
        HStack(spacing: 36) {
            VStack(spacing: 16) {
                Text("hajimemashite!")
                    .font(.rounded(20))
                    .padding(.vertical, 6)
                    .padding(.horizontal, 14)
                    .background(Theme.Colors.cream, in: UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 4, bottomTrailingRadius: 16, topTrailingRadius: 16))
                    .overlay(UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 4, bottomTrailingRadius: 16, topTrailingRadius: 16)
                        .strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                Sprite(name: "kore-happy", scale: 11)
            }
            .frame(width: 340 - 2 * Theme.Stroke.bold, height: 360 - 2 * Theme.Stroke.bold)
            .sticker(radius: Theme.Radius.xl, fill: Theme.Colors.korePink, stroke: Theme.Stroke.bold, shadow: Theme.Sticker.md)
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .bottom, spacing: 12) {
                    Text(verbatim: "korekore").font(.rounded(30)).tracking(-0.6)
                    PixelTag()
                }
                Text("Kore kore!\nI'll call you when\nyour agents need you.").font(.korekore(Theme.Typography.hero)).lineSpacing(0)
                    .lineLimit(3).minimumScaleFactor(0.8)
                Text("I watch Claude Code, Codex and CI from your desk and stay quiet while they work. Let's wake up the box first.")
                    .font(.rounded(16, weight: 400)).foregroundStyle(Theme.Colors.muted).lineSpacing(6)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 14) {
                    Button(String(localized: "Let's go")) { step = 1 }.buttonStyle(KBigButtonStyle(.sun, size: 17)).keyboardShortcut(.defaultAction)
                    Button { showWhatItSends.toggle() } label: {
                        Text("What does it send?").font(.rounded(14, weight: 700)).foregroundStyle(Theme.Colors.korePinkText)
                    }
                    .buttonStyle(.plain)
                }
                if showWhatItSends {
                    KNote(String(localized: "I only pass along session IDs, event names and folders, never your prompts or replies. One connect covers the app and the terminal."))
                }
                Text("Already have a VibeBuddy? Same box, new name. Everything you set up carries over.")
                    .font(.rounded(13, weight: 400)).foregroundStyle(Theme.Colors.muted)
                    .padding(.vertical, 8)
                    .padding(.horizontal, 12)
                    .background(Theme.Colors.paper, in: RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Theme.Colors.mistText, style: StrokeStyle(lineWidth: Theme.Stroke.base, dash: [6, 4])))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(EdgeInsets(top: 4, leading: 44, bottom: 0, trailing: 44))
    }

    // MARK: Find the box

    private var findTheBox: some View {
        HStack(spacing: 32) {
            VStack(spacing: 12) {
                boxDrawing
                KTag(connected ? String(localized: "the box blinks yellow") : String(localized: "waiting for the box"), color: Theme.Colors.muted)
            }
            .frame(width: 340)
            VStack(alignment: .leading, spacing: 12) {
                findTheBoxSide
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(EdgeInsets(top: 8, leading: 40, bottom: 0, trailing: 40))
        .onAppear(perform: blinkOnce)
        .onChange(of: connected) { blinkOnce() }
    }

    /// When the box shows up, it blinks once by itself, so the question can be "is this it?".
    private func blinkOnce() {
        guard connected, !blinked, !model.foreignFirmware, !passive else { return }
        blinked = true
        model.identify()
    }

    /// The box, drawn: its shell with the three keys on top, and on its screen what it's doing.
    private var boxDrawing: some View {
        ZStack(alignment: .topTrailing) {
            Group {
                if flashing {
                    screenWord("kore-working", "UPDATING", fill: Theme.Colors.ink, text: Theme.Colors.sky)
                } else if connected {
                    screenWord("kore-happy", "HELLO!", fill: Theme.Colors.sun, text: Theme.Colors.ink)
                } else {
                    screenWord("kore-sleep", "Z z", fill: Theme.Colors.boxGrayGround, text: Theme.Colors.mist, gray: true)
                }
            }
            .frame(width: 300 - 2 * Theme.Stroke.bold, height: 240 - 2 * Theme.Stroke.bold)
            .sticker(radius: Theme.Radius.xl, fill: Theme.Colors.line, stroke: Theme.Stroke.bold, shadow: Theme.Sticker.md)
            HStack(spacing: 8) {
                ForEach([Theme.Colors.korePink, Theme.Colors.sun, Theme.Colors.sky].indices, id: \.self) { index in
                    RoundedRectangle(cornerRadius: 4)
                        .fill([Theme.Colors.korePink, Theme.Colors.sun, Theme.Colors.sky][index])
                        .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                        .frame(width: 26, height: 12)
                }
            }
            .offset(x: -40, y: -14)
        }
    }

    private func screenWord(_ kore: String, _ word: String, fill: Color, text: Color, gray: Bool = false) -> some View {
        VStack(spacing: 6) {
            Sprite(name: kore, scale: 4, gray: gray)
            Text(verbatim: word).font(.pixel(18)).foregroundStyle(text)
        }
        .frame(width: 240, height: 180)
        .background(fill, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.bold))
    }

    @ViewBuilder private var findTheBoxSide: some View {
        if let operation = model.operation, operation.kind == .firmware, operation.state != .done {
            // The daemon lets go of the port while flashing, so the box reads as gone: progress comes first, or this page
            // falls back to "no box yet" and it looks as if nothing started.
            title(String(localized: "Flashing korekore firmware…"), size: 26)
            switch operation.state {
            case .failed:
                KNote(String(localized: "Hold K0 on the box and replug the cable to put it in download mode, then retry."))
                Button(String(localized: "Try again")) { model.updateFirmware() }.buttonStyle(KBigButtonStyle(.sun))
            case .replug:
                KNote(String(localized: "The firmware is on the box, but it didn't start. Unplug the box and plug it back in. Don't hold K0 this time."))
            default:
                HStack {
                    KTag(operation.progress.formatted(.percent.precision(.fractionLength(0))), color: Theme.Colors.ink)
                    Spacer()
                    KTag(String(localized: "Keep it plugged in"))
                }
                KBar(operation.progress, fill: Theme.Colors.sun)
            }
        } else if let device, device.connected {
            title(String(localized: "Is this your box blinking?"), size: 32)
            HStack(spacing: 10) {
                Rectangle().fill(Theme.Colors.mint).frame(width: 10, height: 10)
                    .overlay(Rectangle().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.thin))
                Text(verbatim: portLine(device)).font(.pixel(14))
                Spacer()
                Button(String(localized: "Blink again")) { model.identify() }.kButton()
            }
            .padding(.vertical, 10)
            .padding(.horizontal, 14)
            .background(Theme.Colors.paper, in: RoundedRectangle(cornerRadius: 14))
            .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
            if model.foreignFirmware {
                VStack(alignment: .leading, spacing: 8) {
                    KNote(String(localized: "This box runs other firmware. Put korekore on it? Its current firmware will be replaced."), color: Theme.Colors.ink)
                    if model.firmwareDownloaded {
                        Button(String(localized: "Flash korekore")) { KorekoreFlash.foreign(then: model.updateFirmware) }.kButton(.pink)
                    } else {
                        KorekoreFirmwareUnavailable(model: model)
                    }
                }
                .padding(.vertical, 10)
                .padding(.horizontal, 14)
                .background(Theme.Colors.paper, in: RoundedRectangle(cornerRadius: 14))
                .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(Theme.Colors.korePinkText, style: StrokeStyle(lineWidth: Theme.Stroke.base, dash: [6, 4])))
            } else {
                HStack(spacing: 10) {
                    Button(String(localized: "Yes, that's it")) { step = 2 }.buttonStyle(KBigButtonStyle(.sun)).keyboardShortcut(.defaultAction)
                    Button(String(localized: "Not this one")) { notThisOne = true }.buttonStyle(KBigButtonStyle(.paper, primary: false))
                }
                if notThisOne {
                    KNote(String(localized: "Unplug it, then plug in your box. If several devices are plugged in, korekore asks which one is yours."))
                }
                // The cable is in and someone is watching: the best moment to bring older factory firmware up to date.
                // Firmware without TRANSFER 1 is flashed over the cable only (ADR-0016), as the Device page says.
                if model.firmwareUpdateAvailable, device.network == true, device.transfer != true {
                    KNote(String(localized: "Plug in the cable to update the box's firmware."))
                } else if model.firmwareUpdateAvailable, let offer = model.offeredFirmware {
                    KCard {
                        HStack(spacing: 8) {
                            (Text("Newer firmware is available").bold().foregroundColor(Theme.Colors.ink) + Text(" for this box (\(offer.version)).").foregroundColor(Theme.Colors.muted))
                                .font(.rounded(13, weight: 400))
                            Spacer()
                            Button(String(localized: "Update now")) { KorekoreFlash.update(offer, then: model.updateFirmware) }.kButton(.sun)
                        }
                    }
                }
            }
        } else if !model.daemonAlive {
            title(String(localized: "Plug in your box"), size: 32)
            KCard { KTag(String(localized: "Starting the daemon…"), color: Theme.Colors.ink) }
        } else {
            title(String(localized: "Plug in your box"), size: 32)
            KNote(String(localized: "Connect the box to this Mac with a USB cable. This page notices it on its own."))
            if let pin = device?.pin {
                KNote(String(localized: "Only looking at \(pin.value), set by \(pin.variable)."), color: Theme.Colors.ink)
            }
            if let candidates = device?.candidates, !candidates.isEmpty {
                KCard {
                    VStack(alignment: .leading, spacing: 8) {
                        KHeading(String(localized: "Which one is your box?"))
                        ForEach(candidates) { candidate in
                            HStack {
                                Text(verbatim: candidate.usbSerial.map { "\(candidate.port) · \($0)" } ?? candidate.port).font(.pixel(12))
                                Spacer()
                                if let serial = candidate.usbSerial {
                                    Button(String(localized: "This is the box")) { model.chooseBox(usbSerial: serial) }.kButton(.sun)
                                }
                            }
                        }
                    }
                }
            } else if device?.pin == nil {
                KNote(String(localized: "Not showing up? Use a cable that carries data, not just power, and click Allow if macOS asks whether to let the accessory connect."))
            }
            Button(String(localized: "Skip for now")) { step = 2 }.buttonStyle(KBigButtonStyle(.paper, primary: false))
        }
    }

    private func portLine(_ device: DeviceState) -> String {
        let port = (device.port ?? "").replacingOccurrences(of: "/dev/cu.", with: "")
        if device.relay == true { return String(localized: "Relay") }
        if device.network == true { return "\(port) · \(String(localized: "Wi-Fi"))" }
        return "\(port) · \(device.bridge ? String(localized: "UART bridge") : String(localized: "native USB"))"
    }

    // MARK: Connect agents

    private var connectAgents: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 14) {
                KoreBadge(scale: 3)
                VStack(alignment: .leading, spacing: 4) {
                    title(String(localized: "Who should I watch?"))
                    Text("I found these on this Mac. I'll show you each change before writing it.")
                        .font(.rounded(14, weight: 400)).foregroundStyle(Theme.Colors.muted)
                }
            }
            KRows {
                ForEach(Array(HookAgent.allCases.enumerated()), id: \.element.rawValue) { index, agent in
                    if index > 0 { KDivider() }
                    agentRow(agent)
                }
            }
            HStack(spacing: 10) {
                let toConnect = chosen.filter { !(model.hookInstalled[$0] ?? false) }
                if toConnect.isEmpty {
                    Button(String(localized: "Next")) { step = 3 }.buttonStyle(KBigButtonStyle(.sun)).keyboardShortcut(.defaultAction)
                } else {
                    Button(String(localized: "Connect \(toConnect.count)")) { connect(toConnect) }
                        .buttonStyle(KBigButtonStyle(.sun)).keyboardShortcut(.defaultAction)
                    Button(String(localized: "Not now")) { step = 3 }.buttonStyle(KBigButtonStyle(.paper, primary: false))
                }
                Text("Codex asks you to trust its hooks once, in /hooks.")
                    .font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted).padding(.leading, 6)
            }
        }
        .padding(EdgeInsets(top: 8, leading: 48, bottom: 0, trailing: 48))
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .onAppear {
            // Everything found and not yet connected starts ticked.
            chosen = Set(HookAgent.allCases.filter { model.hookPresent($0) && !(model.hookInstalled[$0] ?? false) })
        }
    }

    private func agentRow(_ agent: HookAgent) -> some View {
        let present = model.hookPresent(agent)
        let installed = model.hookInstalled[agent] ?? false
        return HStack(spacing: 12) {
            Toggle(isOn: Binding(get: { installed || chosen.contains(agent) },
                                 set: { on in if on { chosen.insert(agent) } else { chosen.remove(agent) } })) { EmptyView() }
                .toggleStyle(.checkbox)
                .tint(Theme.Colors.korePink)
                .disabled(!present || installed)
            Sprite(name: KorekoreAgentRow(model: model, agent: agent, pendingPlan: .constant(nil)).tile, scale: 3)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: agent.displayName).font(.rounded(15))
                Text(verbatim: detail(agent, present: present, installed: installed))
                    .font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
            }
            Spacer()
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 14)
        .opacity(present ? 1 : 0.6)
    }

    private func detail(_ agent: HookAgent, present: Bool, installed: Bool) -> String {
        if !present { return String(localized: "not found on this Mac") }
        if installed { return String(localized: "connected") }
        if agent.ownsFile { return String(localized: "a file of korekore's own") }
        let path = HookInstaller.configURL(for: agent).path.replacingOccurrences(of: FileManager.default.homeDirectoryForCurrentUser.path, with: "~")
        return String(localized: "\(path) · \(agent.events.count) events")
    }

    /// Each agent's change is shown before it's written, one after the other.
    private func connect(_ agents: Set<HookAgent>) {
        queue = HookAgent.allCases.filter(agents.contains)
        nextPlan()
    }

    private func nextPlan() {
        pendingPlan = nil
        guard !queue.isEmpty else {
            if !chosen.isEmpty { step = 3 }
            return
        }
        let agent = queue.removeFirst()
        // A sheet dismissed and another presented in the same update would be lost.
        DispatchQueue.main.async { pendingPlan = model.hookInstallPlan(agent) }
    }

    // MARK: Pick a character

    private var pickACharacter: some View {
        VStack(alignment: .leading, spacing: 14) {
            title(String(localized: "Who lives in your box?"))
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: 12) {
                    pickCard(name: "Kore", blurb: String(localized: "The box itself"), stage: Theme.Colors.ink, inUse: false, comingSoon: true, action: nil) {
                        Sprite(name: "kore-happy", scale: 4)
                    }
                    pickCard(name: "VibeBuddy", blurb: String(localized: "The original robot"), stage: Theme.Colors.boxCard,
                             inUse: model.wearsRobot && !model.robotBorrowsVoice, comingSoon: false,
                             action: { writing = "VibeBuddy"; model.writeRobot() }) {
                        if let face = Resources.robotFace { Image(nsImage: face).interpolation(.none) }
                    }
                    ForEach(Resources.bundledVoices) { entry in
                        pickCard(name: entry.name, blurb: entry.tag, stage: Theme.Colors.boxCard, inUse: model.status?.device.voice == entry.id,
                                 comingSoon: false, action: { writing = entry.name; model.writeVoice(entry.id) }) {
                            if let frame = Resources.voicePack(entry.id)?.look.flatMap(LookBuilder.frames(of:))?.first {
                                Image(nsImage: LookImages.image(frame, scale: 1)).interpolation(.none)
                            }
                        }
                    }
                }
                .padding(.bottom, Theme.Sticker.sm + 2)
                .padding(.trailing, 2)
            }
            .scrollIndicators(.never)
            if let operation = model.operation, operation.kind == .voicePack, operation.state == .running {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text("Writing \(writing ?? String(localized: "the character")) to the box…").font(.rounded(13))
                        Spacer()
                        KTag(operation.progress.formatted(.percent.precision(.fractionLength(0))), color: Theme.Colors.ink)
                    }
                    KSegments(operation.progress)
                }
                .padding(.vertical, 12)
                .padding(.horizontal, 14)
                .background(Theme.Colors.paper, in: RoundedRectangle(cornerRadius: 14))
                .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
            } else if model.operation?.kind == .voicePack, model.operation?.state == .failed {
                KNote(String(localized: "The character didn't finish writing, so the box keeps its built-in voice for now. Write it again when you're ready."))
            } else if !connected {
                KNote(String(localized: "Plug in the box to write a character to it. If you skip this, it speaks with its built-in voice, Jessica (English)."))
            }
            HStack(spacing: 10) {
                if model.operation?.kind == .voicePack, model.operation?.state == .done {
                    Button(String(localized: "Next")) { step = 4 }.buttonStyle(KBigButtonStyle(.sun)).keyboardShortcut(.defaultAction)
                } else {
                    Button(String(localized: "Skip")) { step = 4 }.buttonStyle(KBigButtonStyle(.paper, primary: false))
                        .disabled(model.operationRunning)
                }
            }
        }
        .padding(EdgeInsets(top: 8, leading: 48, bottom: 0, trailing: 48))
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    /// A 150 px card; picking one writes it to the box.
    private func pickCard<Stage: View>(name: String, blurb: String, stage: Color, inUse: Bool, comingSoon: Bool,
                                       action: (() -> Void)?, @ViewBuilder look: () -> Stage) -> some View {
        Button { action?() } label: {
            VStack(alignment: .leading, spacing: 6) {
                ZStack(alignment: .bottom) {
                    stage
                    look().padding(.bottom, 6)
                }
                .frame(height: 120)
                .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.sm))
                HStack {
                    Text(verbatim: name).font(.rounded(15)).lineLimit(1)
                    Spacer(minLength: 2)
                    if inUse { KTag(String(localized: "In use"), color: Theme.Colors.korePinkText) }
                    if comingSoon { KTag(String(localized: "Coming soon")) }
                }
                Text(verbatim: blurb).font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted).lineLimit(1)
            }
            .padding(8)
            .frame(width: 150 - 2 * (inUse ? Theme.Stroke.bold : Theme.Stroke.base))
            .foregroundStyle(Theme.Colors.ink)
            .sticker(radius: Theme.Radius.md, fill: Theme.Colors.paper, stroke: inUse ? Theme.Stroke.bold : Theme.Stroke.base,
                     outline: inUse ? Theme.Colors.korePink : Theme.Colors.ink)
        }
        .buttonStyle(.plain)
        .disabled(action == nil || inUse || model.operationRunning || !connected)
    }

    // MARK: Wi-Fi

    private var wifi: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 4) {
                title(String(localized: "Let the box go wireless?"))
                Text("On Wi-Fi, this Mac reaches the box without the cable. Keep the cable in for this step.")
                    .font(.rounded(14, weight: 400)).foregroundStyle(Theme.Colors.muted)
            }
            if let device, device.connected, device.network != true, device.boxKey != nil {
                wifiForm(device)
            } else {
                KNote(String(localized: "Plug in the box to set its network. You can also do it later on the Device page."))
                Button(String(localized: "Skip for now")) { step = 5 }.buttonStyle(KBigButtonStyle(.paper, primary: false))
            }
        }
        .padding(EdgeInsets(top: 8, leading: 48, bottom: 0, trailing: 48))
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder private func wifiForm(_ device: DeviceState) -> some View {
        if joined, let network = device.wifiNetwork {
            KCard {
                VStack(alignment: .leading, spacing: 8) {
                    if let why = device.wifiError {
                        KTag(String(localized: "Couldn't join"), color: Theme.Colors.korePinkText)
                        KNote(KorekoreWifiCard.wifiFailure(why))
                    } else if let address = device.wifiAddress {
                        KTag(String(localized: "Joined · \(address)"), color: Theme.Colors.doneText)
                    } else {
                        HStack {
                            Text("Joining \(network)…").font(.rounded(13))
                            Spacer()
                            KTag(String(localized: "Up to 15 seconds"))
                        }
                        KBar(0.35, fill: Theme.Colors.sky)
                    }
                }
            }
            HStack(spacing: 10) {
                if device.wifiError != nil {
                    Button(String(localized: "Try again")) { joined = false }.buttonStyle(KBigButtonStyle(.sun))
                } else {
                    Button(String(localized: "Next")) { step = 5 }.buttonStyle(KBigButtonStyle(.sun)).keyboardShortcut(.defaultAction)
                        .disabled(device.wifiAddress == nil)
                }
                Button(String(localized: "Skip for now")) { step = 5 }.buttonStyle(KBigButtonStyle(.paper, primary: false))
            }
        } else {
            KRows {
                let seen = Array((device.wifiSeen ?? []).prefix(4))
                if seen.isEmpty {
                    HStack {
                        KTag(String(localized: "Looking for networks…"))
                        Spacer()
                        Button(String(localized: "Scan")) { model.scanWifi() }.kButton().disabled(model.operationRunning)
                    }
                    .padding(.vertical, 10)
                    .padding(.horizontal, 14)
                }
                ForEach(Array(seen.enumerated()), id: \.offset) { index, network in
                    if index > 0 { KDivider() }
                    Button { ssid = network.ssid } label: {
                        HStack(spacing: 12) {
                            SignalBars(rssi: network.rssi)
                            Text(verbatim: network.ssid).font(.rounded(15))
                            Spacer()
                            KTag("\(network.rssi) dBm")
                        }
                        .padding(.vertical, 10)
                        .padding(.horizontal, 14)
                        .background(ssid == network.ssid ? Theme.Colors.cream : Theme.Colors.paper)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .onAppear { if !passive, device.wifiSeen?.isEmpty ?? true { model.scanWifi() } }
            VStack(alignment: .leading, spacing: 6) {
                Text(ssid.isEmpty ? String(localized: "Network name") : String(localized: "Password for \(ssid)")).font(.rounded(13))
                if ssid.isEmpty || !(device.wifiSeen ?? []).contains(where: { $0.ssid == ssid }) {
                    KField(String(localized: "Network name"), text: $ssid)
                }
                KField(String(localized: "Password"), text: $password, secure: true)
            }
            HStack(spacing: 10) {
                Button(String(localized: "Join")) {
                    model.setWifi(ssid: ssid, password: password)
                    password = ""
                    joined = true
                }
                .buttonStyle(KBigButtonStyle(.sun))
                .disabled(ssid.isEmpty || ssid.utf8.count > 32 || password.utf8.count > 63 || model.operationRunning)
                Button(String(localized: "Skip for now")) { step = 5 }.buttonStyle(KBigButtonStyle(.paper, primary: false))
            }
        }
    }

    // MARK: The end

    private var launchAtLogin: some View {
        VStack(alignment: .leading, spacing: 14) {
            title(String(localized: "Start korekore when you log in?"))
            KCard(padding: EdgeInsets(top: 14, leading: 16, bottom: 14, trailing: 16)) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        KHeading(String(localized: "Start korekore when you log in"))
                        Spacer()
                        KToggle(String(localized: "Start korekore when you log in"),
                                isOn: Binding(get: { model.launchAtLogin }, set: { model.setLaunchAtLogin($0) }))
                    }
                    KNote(String(localized: "So the box is never left waiting after a restart. macOS may say a background item was added; that's expected."))
                }
            }
            Button(String(localized: "Next")) { step = 6 }.buttonStyle(KBigButtonStyle(.sun)).keyboardShortcut(.defaultAction)
        }
        .padding(EdgeInsets(top: 8, leading: 48, bottom: 0, trailing: 48))
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var allSet: some View {
        VStack {
            KCard(padding: EdgeInsets(top: 18, leading: 18, bottom: 18, trailing: 18), fill: Theme.Colors.cream, stroke: Theme.Stroke.bold) {
                VStack(alignment: .leading, spacing: 14) {
                    HStack(spacing: 14) {
                        KoreBadge("kore-happy", scale: 3, fill: Theme.Colors.korePink)
                            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                        VStack(alignment: .leading, spacing: 4) {
                            Text("All set. Yatta!").font(.rounded(22))
                            KNote(String(localized: "korekore lives in the menu bar now. Kore kore when someone needs you."))
                        }
                    }
                    HStack {
                        Spacer()
                        Button(String(localized: "Done"), action: finish).buttonStyle(KBigButtonStyle(.sun)).keyboardShortcut(.defaultAction)
                    }
                }
            }
            .frame(maxWidth: 560)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Signal strength as four pixel bars, 4 to 16 px tall.
struct SignalBars: View {
    let rssi: Int

    private var lit: Int {
        if rssi >= -55 { return 4 }
        if rssi >= -67 { return 3 }
        if rssi >= -75 { return 2 }
        return 1
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 2) {
            ForEach(0..<4, id: \.self) { index in
                Rectangle().fill(index < lit ? Theme.Colors.ink : Theme.Colors.backdrop).frame(width: 4, height: CGFloat(4 + index * 4))
            }
        }
        .frame(height: 16, alignment: .bottom)
    }
}

/// Write progress as a 20-segment bar (canvas: "Pick a character").
struct KSegments: View {
    let progress: Double

    init(_ progress: Double) {
        self.progress = progress
    }

    var body: some View {
        HStack(spacing: 2) {
            ForEach(0..<20, id: \.self) { index in
                RoundedRectangle(cornerRadius: 1).fill(Double(index) < progress * 20 ? Theme.Colors.korePink : .clear)
            }
        }
        .padding(3)
        .frame(height: 14)
        .background(Theme.Colors.cream, in: Capsule())
        .overlay(Capsule().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
    }
}
