import SwiftUI
import VibeBuddyCore

/// The Conversation tab: the user's last voice commands and the buddy's spoken replies, newest at the bottom, read from
/// the resident session's thread. Shown only while the daemon runs the resident session (experimental).
struct ConversationView: View {
    /// Reads the exchanges, oldest first; the daemon in the App, canned ones when rendering a screenshot.
    let load: () async throws -> [ResidentExchange]

    static let limit = 20

    @State private var exchanges: [ResidentExchange]?
    @State private var failure: Failure?
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
        // Each time the tab is shown: the conversation goes on on the box while the window is closed.
        .task { await reload() }
    }

    @ViewBuilder private var content: some View {
        if let failure {
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
