import Foundation

/// 菜单栏图标与菜单文案，从状态快照与 daemon 存活情况推出来。
public struct MenuState: Equatable {
    public enum Icon: Equatable { case online, offline, daemonDown }
    public var icon: Icon
    public var deviceLine: String
    public var modeLine: String
    public var todayLine: String
    /// 第一行可点：daemon 异常时点它重启。
    public var deviceLineIsAction: Bool

    public init(icon: Icon, deviceLine: String, modeLine: String, todayLine: String, deviceLineIsAction: Bool) {
        self.icon = icon; self.deviceLine = deviceLine; self.modeLine = modeLine; self.todayLine = todayLine; self.deviceLineIsAction = deviceLineIsAction
    }

    public static func derive(status: Status?, daemonAlive: Bool) -> MenuState {
        guard daemonAlive, let status else {
            return MenuState(icon: .daemonDown, deviceLine: "daemon 异常 · 点击重启", modeLine: "—", todayLine: "—", deviceLineIsAction: true)
        }
        let device = status.device
        let deviceLine: String
        if device.connected {
            let build = device.firmwareBuild.map { String($0.split(separator: " ").first ?? "") } ?? "?"
            deviceLine = "已连接 · 固件 \(build)"
        } else {
            deviceLine = "未找到盒子"
        }
        let mode: String
        switch device.mode {
        case "duty": mode = "值班"
        case "pomodoro": mode = "番茄钟"
        case "leisure": mode = "休闲"
        default: mode = "—"
        }
        let today = status.today
        return MenuState(
            icon: device.connected ? .online : .offline,
            deviceLine: deviceLine,
            modeLine: "模式：\(mode)",
            todayLine: "今天：完成 \(today.done) · 确认 \(today.asks) · 忙碌 \(MenuState.duration(today.busySeconds))",
            deviceLineIsAction: false
        )
    }

    public static func duration(_ seconds: Int) -> String {
        let hours = seconds / 3600
        let minutes = (seconds % 3600) / 60
        if hours > 0 { return "\(hours) 小时 \(minutes) 分" }
        return "\(minutes) 分"
    }
}
