import SwiftUI
import VibeBuddyCore

/// The Conversation tab: the user's last voice commands and the buddy's spoken replies, newest at the bottom, read from
/// the resident session's thread. Shown only while the daemon runs the resident session (experimental).
struct ConversationView: View {
    /// Reads the exchanges, oldest first; the daemon in the App, canned ones when rendering a screenshot.
    let load: () async throws -> [ResidentExchange]
    /// `GET /v1/resident`: the history's change marker, and whether voice commands can be taken.
    let info: () async throws -> ResidentInfo

    static let limit = 20

    @State private var exchanges: [ResidentExchange]?
    @State private var failure: Failure?
    /// What voice commands are missing (the user's Codex), as the daemon last said.
    @State private var problem: ResidentProblem?
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
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .padding(16)
        // Read each time the tab is shown (the conversation goes on on the box while the window is closed), then
        // followed while it stays shown. SwiftUI cancels the task when the tab is left or the window closed.
        .task { await follow() }
    }

    @ViewBuilder private var content: some View {
        if let problem {
            ContentUnavailableView {
                Label("Voice needs Codex", systemImage: "mic.slash")
            } description: {
                switch problem {
                case .codexMissing:
                    Text("Voice commands are answered by Codex on this computer, and it isn't installed. Install the Codex CLI, then sign in with `codex login`. This tab notices within half a minute.")
                case .codexLoggedOut:
                    Text("Voice commands are answered by Codex on this computer, and it isn't signed in. Run `codex login` in a terminal and sign in. This tab notices within half a minute.")
                case .other(let why):
                    Text("The buddy's session isn't available: \(why)")
                }
            } actions: {
                if problem == .codexMissing {
                    Link("How to install the Codex CLI", destination: Resources.codexInstallPage)
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
                    LazyVStack(spacing: 16) {
                        ForEach(Array(exchanges.enumerated()), id: \.offset) { index, exchange in
                            ExchangeRow(exchange: exchange).id(index)
                        }
                    }
                    .padding(12)
                }
                .background(.background, in: RoundedRectangle(cornerRadius: 8))
                .onAppear { proxy.scrollTo(exchanges.count - 1, anchor: .bottom) }
                .onChange(of: exchanges) { proxy.scrollTo(exchanges.count - 1, anchor: .bottom) }
            }
        } else {
            ProgressView()
        }
    }

    /// Asks `GET /v1/resident` every 2 seconds and reads the history again only when its change marker moved, or
    /// when voice has just become ready. A daemon without the marker gets its history read every 5 seconds instead.
    private func follow() async {
        var seen: UInt64?
        var marked = true
        if let first = try? await info() {
            seen = first.revision
            problem = first.problem
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
