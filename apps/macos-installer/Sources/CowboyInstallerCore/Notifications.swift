import Foundation
import UserNotifications

public protocol InstallerNotifying: AnyObject {
    func requestAuthorization() async -> Bool
    func send(title: String, body: String) async
}

public final class SystemInstallerNotifier: InstallerNotifying, @unchecked Sendable {
    public init() {}

    public func requestAuthorization() async -> Bool {
        (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])) ?? false
    }

    public func send(title: String, body: String) async {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        try? await UNUserNotificationCenter.current().add(request)
    }
}
