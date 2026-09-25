import Foundation

/// Menu bar icon and menu copy, derived from the status snapshot and whether the daemon is alive.
/// Copy is keyed in English and looked up in the host bundle (the app ships
/// zh-Hans.lproj); without a table, as in SelfTest, the English key is shown.
public struct MenuState: Equatable {
    public enum Icon: Equatable { case online, offline, daemonDown }
    public var icon: Icon
    public var deviceLine: String
    public var modeLine: String
    public var todayLine: String
    /// The first line is clickable: when the daemon is down, clicking it restarts it.
    public var deviceLineIsAction: Bool

    public init(icon: Icon, deviceLine: String, modeLine: String, todayLine: String, deviceLineIsAction: Bool) {
        self.icon = icon; self.deviceLine = deviceLine; self.modeLine = modeLine; self.todayLine = todayLine; self.deviceLineIsAction = deviceLineIsAction
    }

    public static func derive(status: Status?, daemonAlive: Bool) -> MenuState {
        guard daemonAlive, let status else {
            return MenuState(icon: .daemonDown, deviceLine: String(localized: "daemon not responding · click to restart"), modeLine: "—", todayLine: "—", deviceLineIsAction: true)
        }
        let device = status.device
        let deviceLine: String
        if device.connected {
            let build = device.firmwareBuild.map { String($0.split(separator: " ").first ?? "") } ?? "?"
            deviceLine = String(localized: "Box online · firmware \(build)")
        } else {
            deviceLine = String(localized: "Box not found")
        }
        let mode: String
        switch device.mode {
        case "duty": mode = String(localized: "On duty")
        case "pomodoro": mode = String(localized: "Pomodoro")
        case "leisure": mode = String(localized: "Leisure")
        default: mode = "—"
        }
        let today = status.today
        return MenuState(
            icon: device.connected ? .online : .offline,
            deviceLine: deviceLine,
            modeLine: String(localized: "Mode: \(mode)"),
            todayLine: String(localized: "Today: done \(today.done) · asks \(today.asks) · busy \(MenuState.duration(today.busySeconds))"),
            deviceLineIsAction: false
        )
    }

    public static func duration(_ seconds: Int) -> String {
        let hours = seconds / 3600
        let minutes = (seconds % 3600) / 60
        if hours > 0 { return String(localized: "\(hours) h \(minutes) min") }
        return String(localized: "\(minutes) min")
    }
}
