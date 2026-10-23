import Foundation

/// What the korekore UI shows of the box (ADR-0021): the mini box in the sidebar, the menu bar panel and its icon.
/// It is drawn from the daemon's status, not from a screen capture, so it costs nothing over the link.
/// desktop/src/korekore/picture.rs derives the same for Linux.
public struct BoxPicture: Equatable {
    public enum Mood: Equatable, Sendable {
        case ready, working, input, pomodoro, leisure, noLink
    }

    public enum Link: String, Equatable, Sendable {
        case usb = "USB", lan = "LAN", relay = "RELAY"
    }

    /// A task card as the box shows it: the agent's tile and the title without its prefix.
    public struct Ticket: Equatable, Sendable {
        public var icon: String
        public var title: String
        public var needsInput: Bool
    }

    public var mood: Mood
    public var daemonAlive: Bool
    /// How the box is linked; nil without a link.
    public var link: Link?
    /// `duty`, `pomodoro` or `leisure`, as the box last said.
    public var mode: String?
    /// The newest three, as the box's first page shows them.
    public var tickets: [Ticket]
    /// Every Activity the box is watching.
    public var activityCount: Int
    public var today: TodaySummary
    public var firmwareVersion: String?

    /// The box can show this many cards on a page.
    public static let ticketsShown = 3

    public static func derive(status: Status?, daemonAlive: Bool) -> BoxPicture {
        let activities = status?.activities ?? []
        let device = status?.device
        let linked = daemonAlive && device?.connected == true
        let link: Link? = !linked ? nil : device?.relay == true ? .relay : device?.network == true ? .lan : .usb
        let mood: Mood
        if !linked {
            mood = .noLink
        } else if device?.mode == "pomodoro" {
            mood = .pomodoro
        } else if device?.mode == "leisure" {
            mood = .leisure
        } else if activities.contains(where: { $0.state == "needs_input" }) {
            mood = .input
        } else if !activities.isEmpty {
            mood = .working
        } else {
            mood = .ready
        }
        let tickets = activities.prefix(ticketsShown).map {
            Ticket(icon: icon(forAgent: $0.agent), title: cardTitle($0.title), needsInput: $0.state == "needs_input")
        }
        return BoxPicture(mood: mood, daemonAlive: daemonAlive, link: link, mode: device?.mode, tickets: Array(tickets),
                          activityCount: activities.count, today: status?.today ?? TodaySummary(done: 0, asks: 0, busySeconds: 0),
                          firmwareVersion: device?.firmwareVersion)
    }

    /// The agent's tile in docs/design/korekore/pixel/icons.json; anything else is the user's own events.
    public static func icon(forAgent agent: String) -> String {
        switch agent {
        case "claude-code": "cc"
        case "codex": "cx"
        case "github-actions": "ci"
        case "opencode": "oc"
        case "copilot": "gh"
        case "pi": "pi"
        default: "api"
        }
    }

    /// The title without the prefix the daemon adds to tell agents apart (`CC:`, `CX:`…): the tile says that now.
    public static func cardTitle(_ title: String) -> String {
        guard let colon = title.firstIndex(of: ":"), title.distance(from: title.startIndex, to: colon) <= 3,
              title[..<colon].allSatisfy({ $0.isUppercase || $0.isNumber }) else { return title }
        return String(title[title.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
    }

    /// Busy time the way the panel's stat chip shows it: `35m`, `4h12m`.
    public static func busy(_ seconds: Int) -> String {
        let minutes = seconds / 60
        return minutes < 60 ? "\(minutes)m" : "\(minutes / 60)h\(String(format: "%02d", minutes % 60))m"
    }
}
