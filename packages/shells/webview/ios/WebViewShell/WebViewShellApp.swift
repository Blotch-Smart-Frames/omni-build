import SwiftUI

/// OmniBuild's WebView shell: a web app, the payload's `index.html` and the
/// files beside it, in a WKWebView that fills the screen. Nothing that
/// differs between apps is in the Swift code. The bundle ID, name, version,
/// icon and the web app itself are data in the bundle, which OmniBuild
/// rewrites without a compiler.
@main
struct WebViewShellApp: App {
    var body: some Scene {
        WindowGroup {
            // Under the status bar and the home indicator too, as Safari
            // draws a page: the page keeps clear of them with
            // `env(safe-area-inset-*)`, and one that doesn't ask for the
            // whole screen (`viewport-fit=cover`) is kept clear by WebKit.
            PayloadWebView()
                .ignoresSafeArea()
        }
    }
}
