import Foundation
import UserNotifications

/// Speaks up only about link trouble: the box gone for over 30 seconds, or the daemon failing to restart three times.
/// Permission is requested the first time a notification is needed.
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
