import SwiftUI
import VibeBuddyCore

/// The Conversation tab: the user's last voice commands and the buddy's spoken replies, newest at the bottom, read from
/// the resident session's thread. Shown only while the daemon runs the resident session (experimental).
struct ConversationView: View {
    /// Reads the exchanges, oldest first; the daemon in the App, canned ones when rendering a screenshot.
    let load: () async throws -> [ResidentExchange]
    /// `GET /v1/resident`: the history's change marker, and whether voice commands can be taken.
    let info: () async throws -> ResidentInfo
    /// Picks the Agent that answers voice commands; the daemon answers with its info.
    let setBrain: (ResidentBrainChoice) async throws -> ResidentInfo
    /// Stores the user's Anthropic API key, or removes it with nil; the daemon answers with its info.
    let setKey: (String?) async throws -> ResidentInfo

    static let limit = 20

    @State private var exchanges: [ResidentExchange]?
    @State private var failure: Failure?
    /// What voice commands are missing (the user's Codex or Claude Code), as the daemon last said.
    @State private var problem: ResidentProblem?
    /// The daemon's last answer, for the Agent setting and whether a key is stored.
    @State private var latest: ResidentInfo?
    @State private var keyDraft = ""
    @State private var settingError: String?
    @State private var loading = false

    enum Failure: Equatable {
        case daemonDown
        case unavailable(String)
    }

    var body: some View {
        VStack(spacing: 12) {
            HStack {
                Text("Your last voice commands and the buddy's spoken replies, newest at the bottom.")
                    .font(.callout).foregroundStyle(.secondary)
                Spacer()
                if loading { ProgressView().controlSize(.small) }
                Button("Refresh") { Task { await reload() } }.disabled(loading)
            }
            // A daemon older than the setting doesn't say it; then there is only Codex.
            if let choice = latest?.brainSetting {
                agentSetting(choice)
            }
            if problem == nil, let note = latest?.voiceOffNote {
                Label(note, systemImage: "mic.slash")
                    .font(.callout)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
            }
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .padding(16)
        // Read each time the tab is shown (the conversation goes on on the box while the window is closed), then
        // followed while it stays shown. SwiftUI cancels the task when the tab is left or the window closed.
        .task { await follow() }
    }

    /// Which Agent answers, and the user's Anthropic API key for Claude Code. The key is typed once and never shown
    /// again: the daemon only says whether it has one.
    private func agentSetting(_ choice: ResidentBrainChoice) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Picker(selection: Binding(get: { choice }, set: { picked in Task { await apply { try await setBrain(picked) } } })) {
                Text("Automatic").tag(ResidentBrainChoice.auto)
                Text(verbatim: "Codex").tag(ResidentBrainChoice.codex)
                Text(verbatim: "Claude Code").tag(ResidentBrainChoice.claude)
            } label: {
                Text("Answered by")
            }
            .pickerStyle(.segmented)
            .fixedSize()
            if choice == .auto {
                Text("Automatic: Codex when it's ready, else Claude Code with your API key.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if choice != .codex {
                HStack {
                    if latest?.anthropicKeySet == true {
                        Label("Anthropic API key saved", systemImage: "key.fill")
                        Button("Remove") { Task { await apply { try await setKey(nil) } } }
                    } else {
                        SecureField(text: $keyDraft, prompt: Text("Anthropic API key")) { Text("Anthropic API key") }
                            .labelsHidden()
                            .textFieldStyle(.roundedBorder)
                            .frame(maxWidth: 320)
                            .onSubmit { saveKey() }
                        Button("Save") { saveKey() }
                            .disabled(keyDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                    Link("Get an API key", destination: Resources.anthropicConsole)
                }
                Text("Claude Code answers only with your own Anthropic API key, billed to your Anthropic account; a Claude subscription can't be used here. What you say and your Agents' task titles go to Anthropic.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let settingError {
                Text(verbatim: settingError).font(.caption).foregroundStyle(.red)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func saveKey() {
        let key = keyDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return }
        Task {
            await apply { try await setKey(key) }
            if latest?.anthropicKeySet == true { keyDraft = "" }
        }
    }

    /// Sends a setting and takes the daemon's answer as the new state.
    private func apply(_ change: () async throws -> ResidentInfo) async {
        do {
            let now = try await change()
            latest = now
            problem = now.problem
            settingError = nil
        } catch {
            settingError = error.localizedDescription
        }
    }

    @ViewBuilder private var content: some View {
        if let problem {
            ContentUnavailableView {
                switch problem {
                case .claudeMissing, .anthropicKeyMissing:
                    Label("Voice needs Claude Code", systemImage: "mic.slash")
                default:
                    Label("Voice needs Codex", systemImage: "mic.slash")
                }
            } description: {
                VStack(spacing: 8) {
                    switch problem {
                    case .codexMissing:
                        Text("Voice commands are answered by Codex on this computer, and it isn't installed. Install the Codex CLI, then sign in with `codex login`. This tab notices within half a minute.")
                    case .codexLoggedOut:
                        Text("Voice commands are answered by Codex on this computer, and it isn't signed in. Run `codex login` in a terminal and sign in. This tab notices within half a minute.")
                    case .claudeMissing:
                        Text("Voice commands are answered by Claude Code on this computer, and it isn't installed. Install Claude Code; you don't need to sign in to it. This tab notices within half a minute.")
                    case .anthropicKeyMissing:
                        Text("Claude Code answers voice commands only with your own Anthropic API key. Add your key above.")
                    case .other(let why):
                        Text("The buddy's session isn't available: \(why)")
                    }
                    if latest?.brainSetting == .auto, problem == .codexMissing || problem == .codexLoggedOut {
                        Text("Or let Claude Code answer, with your own Anthropic API key: add it above.")
                    }
                }
            } actions: {
                if problem == .codexMissing {
                    Link("How to install the Codex CLI", destination: Resources.codexInstallPage)
                }
                if problem == .claudeMissing {
                    Link("How to install Claude Code", destination: Resources.claudeInstallPage)
                }
            }
        } else if let failure {
            ContentUnavailableView {
                Label("Couldn't read the conversation", systemImage: "exclamationmark.bubble")
            } description: {
                switch failure {
                case .daemonDown:
                    Text("The daemon isn't running. Restart it on the Advanced tab.")
                case .unavailable(let why):
                    Text("The buddy's session isn't available: \(why)")
                }
            }
        } else if let exchanges, exchanges.isEmpty {
            ContentUnavailableView {
                Label("Nothing said yet", systemImage: "bubble.left.and.bubble.right")
            } description: {
                Text("Hold K0 on the box and talk to the buddy. What you say and what it says back show up here.")
            }
        } else if let exchanges {
            ScrollViewReader { proxy in
                ScrollView {
                    // A plain stack: at most 20 exchanges, and a lazy one can't be scrolled to rows it hasn't laid
                    // out, which left the view scrolled past a shorter history (switching from Codex to Claude).
                    VStack(spacing: 16) {
                        ForEach(Array(exchanges.enumerated()), id: \.offset) { index, exchange in
                            ExchangeRow(exchange: exchange).id(index)
                        }
                    }
                    .padding(12)
                }
                // A new thread (the other Agent answering) gets a new scroll view, so no offset carries over.
                .id(latest?.threadId ?? "")
                .background(.background, in: RoundedRectangle(cornerRadius: 8))
                .onAppear { scrollToEnd(proxy) }
                .onChange(of: exchanges) { scrollToEnd(proxy) }
            }
        } else {
            ProgressView()
        }
    }

    /// Scrolls to the newest exchange once the rows are laid out.
    private func scrollToEnd(_ proxy: ScrollViewProxy) {
        guard let exchanges, !exchanges.isEmpty else { return }
        DispatchQueue.main.async { proxy.scrollTo(exchanges.count - 1, anchor: .bottom) }
    }

    /// Asks `GET /v1/resident` every 2 seconds and reads the history again only when its change marker moved, or
    /// when voice has just become ready. A daemon without the marker gets its history read every 5 seconds instead.
    private func follow() async {
        var seen: UInt64?
        var marked = true
        if let first = try? await info() {
            seen = first.revision
            problem = first.problem
            latest = first
        }
        await reload()
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(marked ? 2 : 5))
            if Task.isCancelled { return }
            do {
                let now = try await info()
                marked = now.revision != nil
                let becameReady = problem != nil && now.problem == nil
                problem = now.problem
                latest = now
                if now.revision == nil || now.revision != seen || becameReady {
                    seen = now.revision
                    await reload()
                }
            } catch let error as URLError where error.code == .cannotConnectToHost {
                // Read again once it answers: a restarted daemon counts from 0 again.
                seen = nil
                problem = nil
                failure = .daemonDown
            } catch {
                // A slow or cancelled ask; the next one tries again.
            }
        }
    }

    private func reload() async {
        loading = true
        defer { loading = false }
        do {
            exchanges = try await load()
            failure = nil
        } catch is CancellationError {
            // The tab was left while reading; the next visit reads again.
        } catch let error as URLError where error.code == .cancelled {
        } catch let error as URLError where error.code == .cannotConnectToHost {
            failure = .daemonDown
        } catch {
            failure = .unavailable(error.localizedDescription)
        }
    }
}

/// The user's words on the right, then the buddy's spoken replies on the left, as in a chat.
struct ExchangeRow: View {
    let exchange: ResidentExchange

    var body: some View {
        VStack(spacing: 6) {
            bubble(exchange.said, mine: true)
            ForEach(Array(exchange.replies.enumerated()), id: \.offset) { _, reply in
                bubble(reply, mine: false)
            }
        }
    }

    private func bubble(_ words: String, mine: Bool) -> some View {
        Text(verbatim: words)
            .textSelection(.enabled)
            .padding(.horizontal, 12).padding(.vertical, 7)
            .foregroundStyle(mine ? Color.white : Color.primary)
            .background(mine ? Color.accentColor : Color.secondary.opacity(0.15), in: RoundedRectangle(cornerRadius: 12))
            .frame(maxWidth: 440, alignment: mine ? .trailing : .leading)
            .frame(maxWidth: .infinity, alignment: mine ? .trailing : .leading)
    }
}
