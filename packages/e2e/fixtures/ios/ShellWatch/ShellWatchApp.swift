import SwiftUI

/// A stand-in for the watch app a shell may carry, as Xcode's template makes
/// one. It's there to be repackaged: OmniBuild gives it an ID under the
/// app's, names the app as its companion, gives it the app's version and
/// build, and signs it before the app.
@main
struct ShellWatchApp: App {
    var body: some Scene {
        WindowGroup {
            Text("ShellWatch")
        }
    }
}
