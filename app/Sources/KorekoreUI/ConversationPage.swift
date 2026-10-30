import SwiftUI
import VibeBuddyCore

/// The Conversation page (canvas: "Mac · Conversation", and its states on "App · Agents, Conversation, Character
/// states"): the last voice commands and the buddy's spoken replies, newest at the bottom. It reads and follows the
/// history through four calls to the daemon.
public struct KorekoreConversationPage: View {
    public struct Links {
        public var anthropicConsole: URL
        public var codexInstall: URL
        public var claudeInstall: URL

        public init(anthropicConsole: URL, codexInstall: URL, claudeInstall: URL) {
            self.anthropicConsole = anthropicConsole
            self.codexInstall = codexInstall
            self.claudeInstall = claudeInstall
        }
    }

    let load: () async throws -> [ResidentExchange]
    let info: () async throws -> ResidentInfo
    let setBrain: (ResidentBrainChoice) async throws -> ResidentInfo
    let setKey: (String?) async throws -> ResidentInfo
    let links: Links
    /// The Character the box wears, at 1×, beside each reply; nil shows Kore.
    let avatar: NSImage?

    public static let limit = 20

    @State private var exchanges: [ResidentExchange]?
    @State private var failure: Failure?
    @State private var problem: ResidentProblem?
    @State private var latest: ResidentInfo?
    @State private var keyDraft = ""
    @State private var settingError: String?
    @State private var loading = false

    enum Failure: Equatable {
        case daemonDown
        case unavailable(String)
    }

    public init(load: @escaping () async throws -> [ResidentExchange], info: @escaping () async throws -> ResidentInfo,
                setBrain: @escaping (ResidentBrainChoice) async throws -> ResidentInfo,
                setKey: @escaping (String?) async throws -> ResidentInfo, links: Links, avatar: NSImage?) {
        self.load = load
        self.info = info
        self.setBrain = setBrain
        self.setKey = setKey
        self.links = links
        self.avatar = avatar
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                if let choice = latest?.brainSetting {
                    Text("Answered by").font(.rounded(13))
                    brainChips(choice)
                }
                Spacer()
                Button(String(localized: "Refresh")) { Task { await reload() } }.kButton().disabled(loading)
            }
            if let choice = latest?.brainSetting, choice != .codex {
                keyCard
            }
            if problem == nil, let note = latest?.voiceOffNote {
                KCard(outline: Theme.Colors.korePinkText) {
                    HStack(alignment: .top, spacing: 10) {
                        KoreBadge()
                        KNote(note)
                    }
                }
            }
            content.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            Text("last 20 exchanges · newest at the bottom · stays on this Mac")
                .font(.pixel(12)).foregroundStyle(Theme.Colors.muted2)
        }
        .task { await follow() }
    }

    private func brainChips(_ choice: ResidentBrainChoice) -> some View {
        HStack(spacing: 6) {
            chip(String(localized: "Automatic"), tile: nil, on: choice == .auto) { pick(.auto) }
            chip("Codex", tile: "icon-cx", on: choice == .codex) { pick(.codex) }
            chip("Claude Code", tile: "icon-cc", on: choice == .claude) { pick(.claude) }
        }
    }

    private func chip(_ label: String, tile: String?, on: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if let tile { Sprite(name: tile, scale: 2) }
                Text(verbatim: label)
            }
            .font(.rounded(13))
            .padding(EdgeInsets(top: tile == nil ? 4 : 3, leading: tile == nil ? 12 : 4, bottom: tile == nil ? 4 : 3, trailing: 12))
            .foregroundStyle(on ? Theme.Colors.cream : Theme.Colors.ink)
            .background(on ? Theme.Colors.ink : Theme.Colors.paper, in: Capsule())
            .overlay(Capsule().strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
    }

    private func pick(_ choice: ResidentBrainChoice) {
        Task { await apply { try await setBrain(choice) } }
    }

    /// Claude Code answers with the user's own Anthropic API key, typed once and never shown again: the daemon only
    /// says whether it has one.
    private var keyCard: some View {
        KCard {
            VStack(alignment: .leading, spacing: 8) {
                if latest?.anthropicKeySet == true {
                    HStack {
                        KTag(String(localized: "Anthropic API key saved"), color: Theme.Colors.doneText)
                        Spacer()
                        Button(String(localized: "Remove")) { Task { await apply { try await setKey(nil) } } }.kButton()
                    }
                } else {
                    Text("Anthropic API key").font(.rounded(13))
                    HStack {
                        KField(String(localized: "Anthropic API key"), text: $keyDraft, secure: true).onSubmit(saveKey)
                        Button(String(localized: "Save"), action: saveKey).kButton(.sun)
                            .disabled(keyDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                    Link(destination: links.anthropicConsole) {
                        Text("Get a key in the Claude Console").font(.rounded(13, weight: 400)).foregroundStyle(Theme.Colors.korePinkText)
                    }
                }
                KNote(String(localized: "Claude Code answers only with your own Anthropic API key, billed to your Anthropic account; a Claude subscription can't be used here. What you say and your Agents' task titles go to Anthropic."))
                if let settingError {
                    KTag(settingError, color: Theme.Colors.korePinkText)
                }
            }
        }
    }

    private func saveKey() {
        let key = keyDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return }
        Task {
            await apply { try await setKey(key) }
            if latest?.anthropicKeySet == true { keyDraft = "" }
        }
    }

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
            needs(problem)
        } else if let failure {
            KCard(outline: Theme.Colors.korePinkText) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Couldn't read the conversation").font(.rounded(15)).foregroundStyle(Theme.Colors.korePinkText)
                    switch failure {
                    case .daemonDown: KNote(String(localized: "The daemon isn't running. Restart it on the Advanced page."))
                    case .unavailable(let why): KNote(String(localized: "The buddy's session isn't available: \(why)"))
                    }
                }
            }
        } else if let exchanges, exchanges.isEmpty {
            KCard {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 10) {
                        KoreBadge()
                        Text("Nothing said yet").font(.rounded(15))
                    }
                    KNote(String(localized: "Hold K0 on the box and talk to the buddy. Your words and its replies show up here."))
                }
            }
        } else if let exchanges {
            chat(exchanges)
        } else {
            KCard { KTag(String(localized: "Loading the conversation…"), color: Theme.Colors.ink) }
        }
    }

    /// What voice commands are missing: the user's Codex, or Claude Code with their key.
    private func needs(_ problem: ResidentProblem) -> some View {
        let claude = problem == .claudeMissing || problem == .anthropicKeyMissing
        return KCard {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 10) {
                    Sprite(name: claude ? "icon-cc" : "icon-cx", scale: 3)
                    Text(claude ? "Voice needs Claude Code" : "Voice needs Codex").font(.rounded(15))
                }
                switch problem {
                case .codexMissing:
                    KNote(String(localized: "Voice commands are answered by Codex on this computer, and it isn't installed. Install the Codex CLI, then sign in with `codex login`. This tab notices within half a minute."))
                case .codexLoggedOut:
                    KNote(String(localized: "Voice commands are answered by Codex on this computer, and it isn't signed in. Run `codex login` in a terminal and sign in. This tab notices within half a minute."))
                case .claudeMissing:
                    KNote(String(localized: "Voice commands are answered by Claude Code on this computer, and it isn't installed. Install Claude Code; you don't need to sign in to it. This tab notices within half a minute."))
                case .anthropicKeyMissing:
                    KNote(String(localized: "Claude Code answers voice commands only with your own Anthropic API key. Add your key above."))
                case .other(let why):
                    KNote(String(localized: "The buddy's session isn't available: \(why)"))
                }
                if latest?.brainSetting == .auto, problem == .codexMissing || problem == .codexLoggedOut {
                    KNote(String(localized: "Or let Claude Code answer, with your own Anthropic API key: add it above."))
                }
                if problem == .codexMissing {
                    Link(destination: links.codexInstall) { link(String(localized: "How to install the Codex CLI")) }
                }
                if problem == .claudeMissing {
                    Link(destination: links.claudeInstall) { link(String(localized: "How to install Claude Code")) }
                }
            }
        }
    }

    private func link(_ text: String) -> some View {
        Text(verbatim: text).font(.rounded(13, weight: 400)).foregroundStyle(Theme.Colors.korePinkText)
    }

    private func chat(_ exchanges: [ResidentExchange]) -> some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: 12) {
                    ForEach(Array(exchanges.enumerated()), id: \.offset) { index, exchange in
                        VStack(spacing: 8) {
                            mine(exchange.said)
                            ForEach(Array(exchange.replies.enumerated()), id: \.offset) { _, reply in
                                buddy(reply)
                            }
                        }
                        .id(index)
                    }
                }
                .padding(16)
            }
            .id(latest?.threadId ?? "")
            .onAppear { scrollToEnd(proxy, exchanges) }
            .onChange(of: exchanges) { scrollToEnd(proxy, exchanges) }
        }
        .background(Theme.Colors.paper, in: RoundedRectangle(cornerRadius: Theme.Radius.md))
        .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.md))
        .overlay(RoundedRectangle(cornerRadius: Theme.Radius.md).strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
        .background(RoundedRectangle(cornerRadius: Theme.Radius.md).fill(Theme.Colors.ink).offset(y: Theme.Sticker.sm))
    }

    /// Your words: right, in an Ink bubble.
    private func mine(_ words: String) -> some View {
        Text(verbatim: words)
            .font(.korekore(Theme.Typography.body))
            .lineSpacing(6)
            .textSelection(.enabled)
            .foregroundStyle(Theme.Colors.cream)
            .padding(.vertical, 8)
            .padding(.horizontal, 14)
            .background(Theme.Colors.ink, in: UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 16, bottomTrailingRadius: 4, topTrailingRadius: 16))
            .containerRelativeFrame(.horizontal, alignment: .trailing) { width, _ in width * 0.75 }
            .frame(maxWidth: .infinity, alignment: .trailing)
    }

    /// The buddy's reply: left, in a cream bubble, beside the Character the box wears.
    private func buddy(_ words: String) -> some View {
        HStack(alignment: .bottom, spacing: 8) {
            Group {
                if let avatar {
                    Image(nsImage: avatar).interpolation(.none)
                } else {
                    Sprite(name: "kore-happy")
                }
            }
            .padding(.vertical, 3)
            .padding(.horizontal, 4)
            .background(Theme.Colors.ink, in: RoundedRectangle(cornerRadius: 8))
            Text(verbatim: words)
                .font(.korekore(Theme.Typography.body))
                .lineSpacing(7)
                .textSelection(.enabled)
                .padding(.vertical, 8)
                .padding(.horizontal, 14)
                .background(Theme.Colors.cream, in: UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 4, bottomTrailingRadius: 16, topTrailingRadius: 16))
                .overlay(UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 4, bottomTrailingRadius: 16, topTrailingRadius: 16)
                    .strokeBorder(Theme.Colors.ink, lineWidth: Theme.Stroke.base))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.trailing, 60)
    }

    private func scrollToEnd(_ proxy: ScrollViewProxy, _ exchanges: [ResidentExchange]) {
        guard !exchanges.isEmpty else { return }
        DispatchQueue.main.async { proxy.scrollTo(exchanges.count - 1, anchor: .bottom) }
    }

    /// As the current tab does: `GET /v1/resident` every 2 seconds, the history again only when its marker moved
    /// or voice just became ready; every 5 seconds from a daemon without the marker.
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
                seen = nil
                problem = nil
                failure = .daemonDown
            } catch {
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
        } catch let error as URLError where error.code == .cancelled {
        } catch let error as URLError where error.code == .cannotConnectToHost {
            failure = .daemonDown
        } catch {
            failure = .unavailable(error.localizedDescription)
        }
    }
}
