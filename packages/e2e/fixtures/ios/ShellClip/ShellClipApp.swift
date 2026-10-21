import ShellKit
import SwiftUI

/// A stand-in for the App Clip a shell may carry, as Xcode's template makes
/// one. It's there to be repackaged: OmniBuild gives it an ID under the
/// app's, the app's version and build, and a signature of its own before
/// the app's, which names the app as its parent. An App Clip downloads on
/// its own, without the app, so it embeds the frameworks it uses in its own
/// Frameworks/, as this one does ShellKit, which OmniBuild signs first.
@main
struct ShellClipApp: App {
    var body: some Scene {
        WindowGroup {
            Text(ShellKit.name)
        }
    }
}
