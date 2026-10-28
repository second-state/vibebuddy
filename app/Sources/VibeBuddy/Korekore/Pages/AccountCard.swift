import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// The account (ADR-0018) and the user's own keys (ADR-0020), at the top of the General page: the design has no
/// Account page or sidebar tile, so it sits with the other settings. The same logic as `AccountView`.
struct KorekoreAccountCard: View {
    @ObservedObject var model: AppModel
    /// The browser is open on the sign-in page; cleared when the status says signed in.
    @State private var waiting = false

    private var account: AccountStatus? { model.status?.account }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Account").font(.rounded(15)).padding(.leading, 4)
            KRows {
                if model.status == nil {
                    row { KNote(String(localized: "The daemon isn't running. Restart it on the Advanced tab.")) }
                } else if let account, account.available {
                    if account.signedIn {
                        signedIn(account)
                    } else {
                        row {
                            KNote(String(localized: "Signing in opens your browser. Voice commands need an account."))
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if waiting { KTag(String(localized: "Waiting for the browser…"), color: Theme.Colors.waitText) }
                            Button(String(localized: "Sign in")) { signIn() }.kButton(.sun)
                        }
                    }
                } else {
                    row { KNote(String(localized: "Signing in isn't available in this version of VibeBuddy.")) }
                }
            }
            // A daemon older than own keys doesn't report them.
            if let keys = model.status?.voiceKeys {
                Text("Your own keys").font(.rounded(15)).padding(.leading, 4)
                KRows {
                    row {
                        KNote(String(localized: "With your own key, spoken replies in that provider's voices go straight to it, billed to your account there, and never through VibeBuddy."))
                    }
                    KDivider()
                    KorekoreOwnKeyRow(title: "ElevenLabs", key: keys.elevenlabs, keyPage: Resources.elevenLabsKeys,
                                      refused: String(localized: "ElevenLabs refused your own key. Replies in its voices stay silent until you set another one.")) { key in
                        await saveKey("elevenlabs", key)
                    }
                    KDivider()
                    KorekoreOwnKeyRow(title: String(localized: "Volcano Engine"), key: keys.volc, keyPage: Resources.volcConsole,
                                      refused: String(localized: "Volcano Engine refused your own key. Replies in its voices stay silent until you set another one.")) { key in
                        await saveKey("volc", key)
                    }
                }
            }
        }
        .onChange(of: account?.signedIn) { waiting = false }
        // Reading the day's use and the plan costs a request to the service, so only when the page is shown.
        .task { if account?.signedIn == true { try? await model.client.refreshAccount() } }
    }

    @ViewBuilder private func signedIn(_ account: AccountStatus) -> some View {
        row {
            Text(verbatim: signedInText).font(.rounded(14)).frame(maxWidth: .infinity, alignment: .leading)
            Button(String(localized: "Sign out")) { signOut() }.kButton()
            Button(String(localized: "Delete account…")) { confirmDelete() }.kButton()
        }
        if AccountPlan.shown, let plan = account.plan {
            KDivider()
            row {
                Text("Plan").font(.rounded(14))
                Spacer()
                KTag(plan.paid ? String(localized: "Paid plan") : String(localized: "Free"), color: plan.paid ? Theme.Colors.doneText : Theme.Colors.ink)
            }
            if plan.paid, let used = plan.characters, let limit = plan.charactersLimit, let end = plan.periodEnd {
                KDivider()
                row {
                    KTag(String(localized: "This month: \(used.formatted()) of \(limit.formatted()) characters · renews \(end.formatted(.dateTime.day().month(.abbreviated)))"))
                }
            }
        }
        if let refusal {
            KDivider()
            row { KTag(refusal, color: Theme.Colors.korePinkText) }
        } else if let today = account.today {
            KDivider()
            row { KTag(String(localized: "Voice today: \(today.commands) of \(today.commandsLimit)")) }
        }
    }

    private func row<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        HStack(spacing: 12) { content() }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 12)
            .padding(.horizontal, 16)
    }

    /// Why voice is refused today, if it is.
    private var refusal: String? {
        switch account?.refused {
        case nil: nil
        case "global_cap": String(localized: "Voice is paused for everyone today. It comes back at \(AccountStatus.voiceComesBack()).")
        default: String(localized: "Today's voice is used up. It comes back at \(AccountStatus.voiceComesBack()).")
        }
    }

    private var signedInText: String {
        if let login = account?.githubLogin { return String(localized: "Signed in with GitHub as \(login)") }
        if let id = account?.githubID { return String(localized: "Signed in with GitHub (user \(id))") }
        return String(localized: "Signed in as \(account?.email ?? "")")
    }

    /// The status stream then shows the key as set or not; true when the daemon took it.
    private func saveKey(_ provider: String, _ key: String?) async -> Bool {
        do {
            try await model.client.setVoiceKey(provider, key)
            return true
        } catch {
            model.lastError = String(localized: "Couldn't save your own key: \(error.localizedDescription)")
            return false
        }
    }

    /// Each click starts a new sign-in; the daemon drops any unfinished one.
    private func signIn() {
        Task {
            do {
                let url = try await model.client.signIn()
                NSWorkspace.shared.open(url)
                waiting = true
            } catch {
                waiting = false
                model.lastError = String(localized: "Couldn't sign in: \(error.localizedDescription)")
            }
        }
    }

    private func signOut() {
        Task {
            do { try await model.client.signOut() } catch { model.lastError = String(localized: "Couldn't sign out: \(error.localizedDescription)") }
        }
    }

    private func confirmDelete() {
        let answer = KorekoreAlert.run(title: String(localized: "Delete your VibeBuddy account?"),
                                       message: String(localized: "This deletes the account and everything kept about it, and signs out on every computer. It can't be undone."),
                                       buttons: [String(localized: "Delete account"), String(localized: "Cancel")], warning: true)
        guard answer.button == 0 else { return }
        Task {
            do { try await model.client.deleteAccount() } catch { model.lastError = String(localized: "Couldn't delete the account: \(error.localizedDescription)") }
        }
    }
}

/// One provider's own key for spoken replies (ADR-0020). It is typed once and never shown again: the daemon only says
/// whether it has one.
struct KorekoreOwnKeyRow: View {
    let title: String
    let key: VoiceKeysStatus.Key
    let keyPage: URL
    let refused: String
    /// Sends the key, or nil to clear it; true when the daemon took it.
    let save: (String?) async -> Bool
    @State private var draft = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Text(verbatim: title).font(.rounded(14))
                Spacer()
                if key.set {
                    KTag(String(localized: "Set"), color: Theme.Colors.doneText)
                    Button(String(localized: "Clear")) { Task { _ = await save(nil) } }.kButton()
                } else {
                    KField(String(localized: "Your own key"), text: $draft, secure: true)
                        .frame(maxWidth: 240)
                        .onSubmit { submit() }
                    Button(String(localized: "Save")) { submit() }.kButton(.sun)
                        .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                Link(destination: keyPage) { Text("Get a key").font(.rounded(12)).underline() }
                    .foregroundStyle(Theme.Colors.korePinkText)
            }
            if key.refused {
                KTag(refused, color: Theme.Colors.korePinkText)
            }
        }
        .padding(.vertical, 11)
        .padding(.horizontal, 16)
    }

    private func submit() {
        let typed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !typed.isEmpty else { return }
        Task { if await save(typed) { draft = "" } }
    }
}
