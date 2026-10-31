import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// The General page (canvas: "Mac · General"; the update lines and the unsupported-app banner on "App · alerts,
/// notifications, banners").
struct KorekoreGeneralPage: View {
    @ObservedObject var model: AppModel
    @State private var language = AppLanguage.current

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                // Nothing stops working; the App just says, every time, that it's too old (ADR-0010).
                if model.updates?.unsupportedApp == true {
                    KCard(fill: Theme.Colors.cream, outline: Theme.Colors.korePinkText) {
                        KNote(String(localized: "This version of korekore is no longer supported. It keeps working; update to keep getting firmware for the box."),
                              color: Theme.Colors.ink)
                    }
                }
                KorekoreAccountCard(model: model)
                Text("This Mac").font(.rounded(15)).padding(.leading, 4)
                KRows {
                    KRow(String(localized: "Language")) {
                        // The dialog runs after this update rather than as a nested modal loop inside the menu's action.
                        KMenu(selection: language, options: [(.system, String(localized: "System")), (.en, "English"), (.zhHans, "简体中文")]) { choice in
                            DispatchQueue.main.async { change(to: choice) }
                        }
                    }
                    KDivider()
                    KRow(String(localized: "Launch at login")) {
                        KToggle(String(localized: "Launch at login"), isOn: Binding(get: { model.launchAtLogin }, set: { model.setLaunchAtLogin($0) }))
                    }
                    KDivider()
                    KRow(String(localized: "Tell me when the box disconnects or the daemon fails")) {
                        KToggle(String(localized: "Tell me when the box disconnects or the daemon fails"),
                                isOn: Binding(get: { model.status?.config.notifyLink ?? true }, set: { model.setNotifyLink($0) }))
                            .disabled(model.status == nil)
                    }
                    // ADR-0012: the Relay carries the box's messages through VibeBuddy's server, so it can be turned off.
                    if AccountPlan.shown {
                        KDivider()
                        KRow(String(localized: "Reach the box through the Relay when it's away from this Mac")) {
                            KToggle(String(localized: "Reach the box through the Relay when it's away from this Mac"),
                                    isOn: Binding(get: { model.status?.config.relay ?? true }, set: { model.setRelay($0) }))
                                .disabled(model.status == nil)
                        }
                    }
                }
                Text("Updates").font(.rounded(15)).padding(.leading, 4)
                KRows {
                    KRow(String(localized: "Check for updates")) {
                        KToggle(String(localized: "Check for updates"),
                                isOn: Binding(get: { model.status?.config.checkUpdates ?? model.updates?.enabled ?? false }, set: { model.setCheckUpdates($0) }))
                            .disabled(model.status == nil)
                    }
                    KDivider()
                    HStack(spacing: 12) {
                        updateLine.frame(maxWidth: .infinity, alignment: .leading)
                        if let app = model.updates?.app {
                            if model.updater.available {
                                Button(String(localized: "Install \(app.version)…")) { model.installApp(app) }.kButton(.sun)
                            } else {
                                Button(String(localized: "Download \(app.version)")) { model.installApp(app) }.kButton(.sun)
                            }
                        }
                        Button(String(localized: "Check now")) { model.checkForUpdates() }.kButton()
                            .disabled(!(model.updates?.enabled ?? false))
                    }
                    .padding(.vertical, 12)
                    .padding(.horizontal, 16)
                }
                KCard(padding: EdgeInsets(top: 14, leading: 16, bottom: 14, trailing: 16)) {
                    HStack(spacing: 14) {
                        KoreBadge(fill: Theme.Colors.korePink)
                            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                        VStack(alignment: .leading, spacing: 3) {
                            HStack(alignment: .bottom, spacing: 8) {
                                Text(verbatim: "korekore").font(.rounded(18))
                                PixelTag()
                            }
                            KTag("App \(Resources.displayVersion)", color: Theme.Colors.muted)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        Button(String(localized: "Report a problem…")) { model.reportProblem() }.kButton()
                    }
                }
            }
            .padding(.bottom, Theme.Sticker.sm + 4)
        }
        .scrollIndicators(.never)
    }

    /// Where updates stand, in pixel type (canvas: "BANNERS AND UPDATE STATUS").
    private var updateLine: KTag {
        guard let updates = model.updates, updates.enabled else { return KTag(String(localized: "Update checks off")) }
        if let error = updates.error { return KTag(String(localized: "Last check failed: \(error)"), color: Theme.Colors.korePinkText) }
        if let app = updates.app { return KTag(String(localized: "korekore \(app.version) is available"), color: Theme.Colors.doneText) }
        guard let checked = updates.lastCheck else { return KTag(String(localized: "Not checked yet")) }
        return KTag(String(localized: "Up to date · checked \(checked.formatted(.relative(presentation: .named)))"), color: Theme.Colors.doneText)
    }

    /// Saves the choice, then offers to restart and, when the box speaks the other language, to switch its voice too.
    private func change(to choice: AppLanguage) {
        guard choice != language else { return }
        choice.save()
        language = choice
        let target = choice.resolved
        guard target != Resources.uiLanguage else { return }
        var voiceSwitch: VoiceCatalogEntry?
        var checkbox: KorekoreAlert.Checkbox?
        var note = String(localized: "The box goes offline for a few seconds while the app restarts.")
        let device = model.status?.device
        // Only a box that has answered (build and voice reported) can take a voice; an unknown voice isn't guessed at.
        if let device, device.connected, device.firmwareBuild != nil, let voice = device.voice,
           let entry = VoiceCatalogEntry.switchSuggestion(boxVoice: voice, to: target, bundled: Resources.bundledVoices) {
            let label = device.bridge
                ? String(localized: "Also switch the box's voice to \(entry.name) (a few minutes over the UART bridge; keep it plugged in)")
                : String(localized: "Also switch the box's voice to \(entry.name)")
            checkbox = KorekoreAlert.Checkbox(label: label, isOn: true)
            voiceSwitch = entry
        } else if !(device?.connected ?? false) {
            note += "\n" + String(localized: "The box isn't connected, so its character stays as it is. You can change it later on the Character tab.")
        }
        KorekoreAlert.ask(title: String(localized: "Restart korekore to change the language?"), message: note,
                          buttons: [String(localized: "Restart"), String(localized: "Not now")], checkbox: checkbox) { [model, voiceSwitch] answer in
            let voice = answer.checked ? voiceSwitch?.id : nil
            if answer.button == 0 {
                model.restart(writingVoice: voice)
            } else if let voice {
                model.writeVoice(voice)
            }
        }
    }
}
