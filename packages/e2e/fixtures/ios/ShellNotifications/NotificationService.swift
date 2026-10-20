import UserNotifications

/// A stand-in for the app extensions a shell may carry, such as one that adds
/// images to push notifications, as Xcode's template makes it. It's there to
/// be repackaged: OmniBuild gives it an ID under the app's, the app's version
/// and build, and a signature of its own before the app's.
class NotificationService: UNNotificationServiceExtension {
    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        contentHandler(request.content)
    }
}
