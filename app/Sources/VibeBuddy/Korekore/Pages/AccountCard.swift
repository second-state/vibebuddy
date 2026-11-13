import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// The account (ADR-0018) and the user's own keys (ADR-0023), at the top of the General page: the design has no
/// Account page or sidebar tile, so it sits with the other settings.
struct KorekoreAccountCard: View {
    @ObservedObject var model: AppModel
    /// The browser is open on the sign-in page; cleared when the status says signed in.
    @State private var waiting = false
    /// A plan button's request is running: a second click would open a second Checkout.
    @State private var planBusy = false
    /// What the plan row says while the App waits for Stripe to confirm a payment.
    @State private var paying: String?
    /// Reads the account every few seconds after Checkout opens, since the plan changes only when Stripe's payment
    /// reaches the service, which can be after the App comes back.
    @State private var watching: Task<Void, Never>?

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
                            KNote(String(localized: "Signing in opens your browser."))
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
                    KorekoreOwnKeyRow(title: "ElevenLabs", key: keys.elevenlabs,
                                      refused: String(localized: "ElevenLabs refused your own key. Replies in its voices stay silent until you set another one.")) { key in
                        await saveKey("elevenlabs", key)
                    }
                    if keys.showsVolc {
                        KDivider()
                        KorekoreOwnKeyRow(title: String(localized: "Volcano Engine"), key: keys.volc,
                                          refused: String(localized: "Volcano Engine refused your own key. Replies in its voices stay silent until you set another one.")) { key in
                            await saveKey("volc", key)
                        }
                    }
                    if let groq = keys.groq {
                        KDivider()
                        row {
                            KNote(String(localized: "When you are signed in, voice commands go through VibeBuddy's voice service and use your voice credit. With your own Groq key, from console.groq.com, your recordings go straight to Groq instead, billed to your account there."))
                        }
                        KDivider()
                        KorekoreOwnKeyRow(title: "Groq", key: groq,
                                          refused: String(localized: "Groq refused your own key. Voice commands stop until you set another one.")) { key in
                            await saveKey("groq", key)
                        }
                    }
                }
            }
        }
        .onChange(of: account?.signedIn) { waiting = false }
        .onDisappear { watching?.cancel(); paying = nil }
        // Reading the plan and the voice credit costs a request to the service, so only when the page is shown.
        .task { if account?.signedIn == true { try? await model.client.refreshAccount() } }
        // Back from Stripe's pages in the browser: read the plan and the voice credit again.
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            if account?.signedIn == true { Task { try? await model.client.refreshAccount() } }
        }
    }

    @ViewBuilder private func signedIn(_ account: AccountStatus) -> some View {
        row {
            Text(verbatim: signedInText).font(.rounded(14)).frame(maxWidth: .infinity, alignment: .leading)
            Button(String(localized: "Sign out")) { signOut() }.kButton()
            Button(String(localized: "Delete account…")) { confirmDelete() }.kButton()
        }
        // The plan row shows with its button: Upgrade while the Paid plan is offered, and Manage plan always on Paid.
        if account.showsPlanRow, let plan = account.plan {
            KDivider()
            row {
                Text("Plan").font(.rounded(14))
                Spacer()
                if let paying, !plan.paid { KTag(paying, color: Theme.Colors.waitText) }
                KTag(plan.paid ? String(localized: "Paid plan") : String(localized: "Free"), color: plan.paid ? Theme.Colors.doneText : Theme.Colors.ink)
                switch account.planButton {
                case .upgrade: Button(String(localized: "Upgrade")) { openPlanPage(.upgrade) }.kButton(.sun).disabled(planBusy)
                case .managePlan: Button(String(localized: "Manage plan")) { openPlanPage(.managePlan) }.kButton().disabled(planBusy)
                case nil: EmptyView()
                }
            }
        }
        // Nothing before the first use of our voice service.
        if let credit = account.voiceCredit {
            KDivider()
            row { KTag(String(localized: "Voice credit \(credit)")) }
        }
    }

    private func row<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        HStack(spacing: 12) { content() }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 12)
            .padding(.horizontal, 16)
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

    /// Opens Checkout or the Customer Portal in the browser; the account is read again when the App comes back.
    private func openPlanPage(_ button: AccountStatus.PlanButton) {
        planBusy = true
        Task {
            let answer = await model.client.planPage(button)
            planBusy = false
            switch answer {
            case .open(let url):
                NSWorkspace.shared.open(url)
                if button == .upgrade { watchForPaid(String(localized: "Waiting for the payment…")) }
            // The daemon has read the account again. Still Free: Stripe has the payment, the service not yet.
            case .conflict("already_paid"):
                if model.status?.account?.plan?.paid != true { watchForPaid(String(localized: "Your payment is being processed.")) }
            // The status shows signed out.
            case .notSignedIn: break
            case .conflict: model.lastError = String(localized: "This Paid plan can't be managed here.")
            case .notOffered: model.lastError = String(localized: "The Paid plan isn't offered here.")
            case .failed(let message):
                model.lastError = String(localized: "Couldn't open the plan page: \(message ?? String(localized: "the daemon didn't say why"))")
            }
        }
    }

    /// Reads the account every 3 s for a minute, or until it is on the Paid plan.
    private func watchForPaid(_ note: String) {
        paying = note
        watching?.cancel()
        watching = Task {
            for _ in 0..<20 {
                try? await Task.sleep(for: .seconds(3))
                if Task.isCancelled || model.status?.account?.plan?.paid == true { break }
                try? await model.client.refreshAccount()
            }
            if !Task.isCancelled { paying = nil }
        }
    }

    private func signOut() {
        Task {
            do { try await model.client.signOut() } catch { model.lastError = String(localized: "Couldn't sign out: \(error.localizedDescription)") }
        }
    }

    private func confirmDelete() {
        KorekoreAlert.ask(title: String(localized: "Delete your VibeBuddy account?"),
                          message: String(localized: "This deletes the account and everything kept about it, and signs out on every computer. It can't be undone."),
                          buttons: [String(localized: "Delete account"), String(localized: "Cancel")], warning: true) { [model] answer in
            guard answer.button == 0 else { return }
            Task {
                do { try await model.client.deleteAccount() } catch { model.lastError = String(localized: "Couldn't delete the account: \(error.localizedDescription)") }
            }
        }
    }
}

/// One provider's own key for spoken replies (ADR-0023). It is typed once and never shown again: the daemon only says
/// whether it has one.
struct KorekoreOwnKeyRow: View {
    let title: String
    let key: VoiceKeysStatus.Key
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
