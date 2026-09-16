import Foundation
import UserNotifications

/// 只在链路异常时出声：盒子断开超过 30 秒、daemon 三次重启失败。
/// 权限在第一次需要弹的时候才申请。
enum Notifier {
    static func notify(title: String, body: String) {
        let center = UNUserNotificationCenter.current()
        center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
            guard granted else { return }
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
            center.add(request)
        }
    }
}
