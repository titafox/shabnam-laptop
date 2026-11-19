import Foundation
import UserNotifications

/// Optional notifications when the laptop connects or disconnects.
enum Notifier {
    /// The notification service only works for a real app bundle.
    static var available: Bool { Bundle.main.bundleURL.pathExtension == "app" && Bundle.main.bundleIdentifier != nil }

    /// Makes macOS ask the user once whether this app may show notifications.
    static func requestAuthorization() {
        guard available else { return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert]) { _, error in
            if let error { log("notifications: \(error.localizedDescription)") }
        }
    }

    static func post(_ title: String) {
        guard available else { return }
        let content = UNMutableNotificationContent()
        content.title = ProjectInfo.name
        content.body = title
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "link-state", content: content, trigger: nil))
    }
}
