import SwiftUI
import VibeBuddyCore

/// First-run onboarding: welcome → find the box → connect agents → pick a voice → launch at login → done.
struct OnboardingView: View {
    @ObservedObject var model: AppModel
    let finish: () -> Void
    @State private var step = 0
    @State private var pendingPlan: HookInstaller.Plan?

    private let titles: [LocalizedStringKey] = ["Welcome", "Find the box", "Connect agents", "Pick a voice", "Launch at login", "Done"]

    var body: some View {
        VStack(spacing: 16) {
            HStack {
                ForEach(titles.indices, id: \.self) { index in
                    Text(titles[index]).font(.caption).foregroundStyle(index == step ? .primary : .secondary)
                    if index + 1 < titles.count { Text("›").foregroundStyle(.tertiary) }
                }
            }
            Divider()
            Group {
                switch step {
                case 0: welcome
                case 1: findDevice
                case 2: hooks
                case 3: voices
                case 4: loginItem
                default: done
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            Divider()
            HStack {
                if step > 0 && step < 5 { Button("Back") { step -= 1 } }
                Spacer()
                if step == 1 || step == 3 { Button("Skip") { step += 1 }.disabled(flashing) }
                if step < 5 {
                    // No paging while flashing: once you page away, nobody is watching the progress.
                    Button(step == 2 && !anyHookInstalled ? "Not now" : "Next") { step += 1 }
                        .keyboardShortcut(.defaultAction)
                        .disabled(step == 1 && flashing)
                } else {
                    Button("Get started", action: finish).keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(24)
        .frame(width: 560, height: 440)
        .sheet(item: Binding(get: { pendingPlan.map(PlanBox.init) }, set: { pendingPlan = $0?.plan })) { box in
            PlanSheet(plan: box.plan, confirm: { model.applyHookPlan(box.plan); pendingPlan = nil }, cancel: { pendingPlan = nil })
        }
    }

    private var anyHookInstalled: Bool { model.hookInstalled.values.contains(true) }
    private var flashing: Bool { model.operation?.kind == .firmware && model.operation?.state == .running }

    private var welcome: some View {
        HStack(alignment: .top, spacing: 24) {
            Image(nsImage: PixelFace.largeImage()).interpolation(.none)
            VStack(alignment: .leading, spacing: 10) {
                Text("Vibe Buddy turns what Codex and Claude Code are doing into pictures and sounds on the box.").font(.title3)
                Text("Next: plug in the box, connect your agents, pick an announcement voice, and choose whether to launch at login. Everything can be changed or skipped.")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var findDevice: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Connect the box to this Mac with a USB cable.").font(.title3)
            if let operation = model.operation, operation.kind == .firmware, operation.state != .done {
                // As soon as flashing starts the daemon releases the serial port and the box shows as "not connected"; progress must
                // come before the connection check, or this page falls back to "no box yet" and the user thinks nothing started
                // (a colleague clicked Next exactly like that on their first flash, 2026-09-22).
                Label("Flashing Vibe Buddy firmware…", systemImage: "arrow.down.circle").foregroundStyle(.orange)
                OperationRow(operation: operation)
                if operation.state == .failed {
                    Text("Hold K0 on the box and replug the cable to put it in download mode, then retry.").font(.caption).foregroundStyle(.secondary)
                    Button("Retry") { model.updateFirmware() }
                } else {
                    Text("The box screen stays dark for a few minutes — don't unplug it. It restarts on its own when done, and this page will show it as found.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            } else if let device = model.status?.device, device.connected, model.foreignFirmware {
                // Factory box: the serial port is there but the firmware isn't ours. Flashing uses the same path as an upgrade.
                Label("Found a box (\(device.port ?? "")), but it isn't running Vibe Buddy firmware.", systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
                if model.bundledFirmwareBuild == nil {
                    Text("This build has no bundled firmware, so it can't flash the box.").font(.caption).foregroundStyle(.secondary)
                } else {
                    Button("Flash Vibe Buddy firmware") { FlashConfirm.foreign(then: model.updateFirmware) }
                    Text("This erases the box's current firmware and data for good. If the native USB port doesn't find the box, use its UART port instead.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            } else if let device = model.status?.device, device.connected {
                Label("Found it: \(device.port ?? "")", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                Button("Make it blink") { model.identify() }
                Text("If the backlight flashes, that's the one.").font(.caption).foregroundStyle(.secondary)
            } else {
                Label(model.daemonAlive ? "No box yet — this updates once you plug it in." : "Starting the daemon…", systemImage: "cable.connector")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var hooks: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Connect your agents so their progress shows up on the box.").font(.title3)
            ForEach(HookAgent.allCases, id: \.rawValue) { agent in
                HookRow(model: model, agent: agent, pendingPlan: $pendingPlan)
            }
            Text("Only session IDs, event names and working directories are forwarded. After writing Codex's config, trust it once on its /hooks page.").font(.caption).foregroundStyle(.secondary)
        }
    }

    private var voices: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Pick an announcement voice.").font(.title3)
            ScrollView {
                VStack(spacing: 6) {
                    ForEach(Resources.bundledVoices) { entry in VoiceCard(model: model, entry: entry) }
                }
            }
            if let operation = model.operation, operation.kind == .voicePack { OperationRow(operation: operation) }
            Text(voiceFootnote).font(.caption).foregroundStyle(.secondary)
        }
    }

    /// The firmware's built-in voice speaks Chinese; point English users at an English pack.
    private var voiceFootnote: String {
        let englishBundled = Resources.bundledVoices.contains { $0.language == .en }
        if Resources.uiLanguage == .en && englishBundled {
            return String(localized: "The box's built-in voice speaks Chinese (Wanwan Xiaohe); pick an English voice above to hear announcements in English.")
        }
        return String(localized: "If you skip this, the box uses its built-in voice, Wanwan Xiaohe (Chinese).")
    }

    private var loginItem: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Start Vibe Buddy automatically when you log in?").font(.title3)
            Toggle("Launch at login", isOn: Binding(get: { model.launchAtLogin }, set: { model.setLaunchAtLogin($0) }))
            Text("macOS may say a background item was added — that's expected.").font(.caption).foregroundStyle(.secondary)
        }
    }

    private var done: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("All set.").font(.title3)
            Text("Vibe Buddy lives in the menu bar: the icon is the buddy's face, with its eyes open while the box is online. To change anything, choose Settings… from its menu.")
                .foregroundStyle(.secondary)
        }
    }
}
