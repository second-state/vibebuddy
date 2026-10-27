import KorekoreUI
import SwiftUI
import VibeBuddyCore

/// The Agents page (canvas: "Mac · Agents (sidebar)"; the write sheet, TRUST AGAIN and terminal states on "App ·
/// Agents, Conversation, Character states"). The current Agents tab's logic (`HooksView`, `HookRow`,
/// `TerminalAccessRow`), copied.
struct KorekoreAgentsPage: View {
    @ObservedObject var model: AppModel
    @State private var pendingPlan: HookInstaller.Plan?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .bottom, spacing: 10) {
                    KoreBadge()
                    Text("I only pass along session IDs, event names and folders, never your prompts or replies. One connect covers the app and the terminal.")
                        .font(.rounded(13, weight: 400))
                        .lineSpacing(4)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.vertical, 8)
                        .padding(.horizontal, 12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Theme.Colors.paper, in: bubble)
                        .overlay(bubble.strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
                }
                KRows {
                    ForEach(Array(HookAgent.allCases.enumerated()), id: \.element.rawValue) { index, agent in
                        if index > 0 { KDivider() }
                        KorekoreAgentRow(model: model, agent: agent, pendingPlan: $pendingPlan)
                    }
                }
                if !TerminalAccess.installed.isEmpty {
                    KRows {
                        ForEach(Array(TerminalAccess.installed.enumerated()), id: \.element.id) { index, terminal in
                            if index > 0 { KDivider() }
                            terminalRow(terminal, state: model.terminalAccess[terminal.bundleID] ?? .unknown)
                        }
                    }
                }
            }
            .padding(.bottom, Theme.Sticker.sm + 4)
        }
        .scrollIndicators(.never)
        .onAppear { model.refreshTerminalAccess() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            model.refreshTerminalAccess()
        }
        .sheet(item: Binding(get: { pendingPlan.map(PlanBox.init) }, set: { pendingPlan = $0?.plan })) { box in
            KorekorePlanSheet(plan: box.plan, confirm: { model.applyHookPlan(box.plan); pendingPlan = nil }, cancel: { pendingPlan = nil })
        }
    }

    private var bubble: UnevenRoundedRectangle {
        UnevenRoundedRectangle(topLeadingRadius: 14, bottomLeadingRadius: 4, bottomTrailingRadius: 14, topTrailingRadius: 14)
    }

    /// K2 jumps straight to the tab a session runs in, with access to the terminal.
    private func terminalRow(_ terminal: TerminalAccess.Terminal, state: TerminalAccess.State) -> some View {
        HStack(spacing: 12) {
            Sprite(name: "icon-nav-adv", scale: 3)
            VStack(alignment: .leading, spacing: 3) {
                Text("Terminal tabs · \(terminal.name)").font(.rounded(15))
                Text(detail(terminal, state)).font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            switch state {
            case .allowed: KTag(String(localized: "Allowed"), color: Theme.Colors.doneText)
            case .notAsked:
                KTag(String(localized: "Not allowed yet"), color: Theme.Colors.waitText)
                Button(String(localized: "Allow…")) { model.allowTerminalAccess(terminal) }.kButton()
            case .denied:
                KTag(String(localized: "Denied"), color: Theme.Colors.korePinkText)
                Button(String(localized: "Open Settings")) { model.openAutomationSettings() }.kButton()
            case .unknown:
                KTag(String(localized: "Not checked"))
                Button(String(localized: "Allow…")) { model.allowTerminalAccess(terminal) }.kButton()
            }
        }
        .padding(.vertical, 11)
        .padding(.horizontal, 14)
    }

    private func detail(_ terminal: TerminalAccess.Terminal, _ state: TerminalAccess.State) -> String {
        switch state {
        case .allowed, .notAsked: String(localized: "K2 jumps straight to the tab a session runs in")
        case .denied: String(localized: "Allow it in System Settings › Privacy › Automation.")
        case .unknown: String(localized: "macOS only answers while \(terminal.name) is running.")
        }
    }
}

/// One agent: its 36 px tile, its name, its state in pixel type and what to do.
struct KorekoreAgentRow: View {
    @ObservedObject var model: AppModel
    let agent: HookAgent
    @Binding var pendingPlan: HookInstaller.Plan?

    private var present: Bool { model.hookPresent(agent) }
    private var installed: Bool { model.hookInstalled[agent] ?? false }
    private var lastEvent: Date? { model.status?.hooks.lastEvent(agent) }
    private var codexHint: CodexTrustHint? {
        guard agent == .codex, installed else { return nil }
        return HookConfig.codexTrustHint(configModifiedAt: model.hookConfigModifiedAt(agent), lastEvent: lastEvent)
    }

    var tile: String {
        switch agent {
        case .claude: "icon-cc"
        case .codex: "icon-cx"
        case .opencode: "icon-oc"
        case .copilot: "icon-gh"
        case .pi: "icon-pi"
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            // The agents' marks are drawn for the box's dark screen, so each sits on a dark tile here too.
            Sprite(name: tile, scale: 2)
                .padding(4)
                .background(Theme.Colors.boxCard, in: RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 3) {
                Text(verbatim: agent.displayName).font(.rounded(15))
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    state.tag
                    Text(verbatim: state.detail).font(.rounded(12, weight: 400)).foregroundStyle(Theme.Colors.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if installed {
                Button(String(localized: "Repair")) { pendingPlan = model.hookInstallPlan(agent) }.kButton()
                Button(String(localized: "Remove")) { pendingPlan = model.hookRemovePlan(agent) }.kButton()
            } else if present {
                Button(String(localized: "Connect")) { pendingPlan = model.hookInstallPlan(agent) }.kButton(.pink)
            }
        }
        .padding(.vertical, 11)
        .padding(.horizontal, 14)
    }

    private var state: (tag: KTag, detail: String) {
        if !present {
            return (KTag(String(localized: "Not found")), String(localized: "install it, then connect"))
        }
        if !installed {
            return (KTag(String(localized: "Not set up"), color: Theme.Colors.korePinkText), String(localized: "found on this Mac"))
        }
        if case .changedSinceLastEvent? = codexHint {
            return (KTag(String(localized: "Trust again"), color: Theme.Colors.korePinkText),
                    String(localized: "The hook config changed after Codex's last event, so Codex turned the hooks off. Type /hooks in Codex and trust korekore's hooks again."))
        }
        if let lastEvent {
            return (KTag(String(localized: "Watching"), color: Theme.Colors.doneText),
                    String(localized: "last event \(lastEvent.formatted(date: .omitted, time: .shortened))"))
        }
        return (KTag(String(localized: "Waiting"), color: Theme.Colors.waitText),
                agent == .codex ? String(localized: "for the first event… Codex runs the hooks once you trust them in /hooks.")
                    : String(localized: "for the first event…"))
    }
}

/// Before writing an agent's config: what changes, in the korekore look (canvas: "AGENTS · BEFORE WRITING A HOOK").
struct KorekorePlanSheet: View {
    let plan: HookInstaller.Plan
    let confirm: () -> Void
    let cancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Will change \(plan.configURL.path)").font(.rounded(16))
            if plan.diff.isEmpty {
                KTag(String(localized: "Nothing to change · already connected"))
            } else {
                KNote(String(localized: "Only korekore's own entries; your other hooks stay."))
                ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(plan.diff, id: \.self) { line in
                            Text(verbatim: line).font(.pixel(12))
                                .foregroundStyle(line.hasPrefix("+") ? Theme.Colors.mint : line.hasPrefix("-") ? Theme.Colors.peach : Theme.Colors.boxTextMuted)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 200)
                .padding(.vertical, 10)
                .padding(.horizontal, 12)
                .background(Theme.Colors.ink, in: RoundedRectangle(cornerRadius: Theme.Radius.sm))
            }
            if plan.agent == .codex {
                KNote(String(localized: "After writing, open /hooks in Codex to review and trust this config — the app can't do that for you."))
            }
            HStack {
                Spacer()
                Button(String(localized: "Cancel"), action: cancel).kButton().keyboardShortcut(.cancelAction)
                Button(String(localized: "Write"), action: confirm).kButton(.sun).keyboardShortcut(.defaultAction).disabled(plan.diff.isEmpty)
            }
        }
        .foregroundStyle(Theme.Colors.ink)
        .padding(EdgeInsets(top: 16, leading: 18, bottom: 16, trailing: 18))
        .frame(width: 520)
        .background(Theme.Colors.cream)
    }
}
