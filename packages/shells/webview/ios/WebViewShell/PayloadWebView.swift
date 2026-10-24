import SwiftUI
import WebKit

/// The WKWebView the app is, showing the payload's `index.html` from
/// `app://localhost/`, which PayloadSchemeHandler serves from the bundle.
struct PayloadWebView: UIViewRepresentable {
    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.setURLSchemeHandler(
            PayloadSchemeHandler(root: Bundle.main.bundleURL),
            forURLScheme: PayloadSchemeHandler.scheme
        )
        ConsoleHandler.install(in: configuration.userContentController)
        // Media plays in the page, as on the web, rather than going full
        // screen as an iPhone otherwise makes it.
        configuration.allowsInlineMediaPlayback = true

        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = context.coordinator
        webView.uiDelegate = context.coordinator
        webView.allowsBackForwardNavigationGestures = true
        // The system's background until the page draws its own, so a dark
        // page doesn't open with a white flash in dark mode.
        webView.isOpaque = false
        webView.backgroundColor = .systemBackground
        #if DEBUG
            // Safari's Web Inspector only for a debug build, as Xcode's Debug
            // configuration makes, never for one in a store.
            webView.isInspectable = true
        #endif
        webView.load(URLRequest(url: PayloadSchemeHandler.startURL))
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {}

    /// Keeps the payload's pages in the app and sends other links out of it,
    /// and shows the page's `alert()`, `confirm()` and `prompt()`, which a
    /// WKWebView otherwise ignores.
    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate {
        /// URLs that only mean something inside the page.
        private static let inPageSchemes: Set<String> = ["about", "blob", "data", "javascript"]

        func webView(
            _ webView: WKWebView,
            decidePolicyFor action: WKNavigationAction,
            decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
        ) {
            guard let url = action.request.url, let scheme = url.scheme?.lowercased() else {
                decisionHandler(.cancel)
                return
            }
            // The payload's pages, URLs inside the page, and frames within
            // it (an embedded video, say) stay. The page itself leaving, or
            // opening a new window, goes to Safari or the app for the link.
            if scheme == PayloadSchemeHandler.scheme || Self.inPageSchemes.contains(scheme)
                || action.targetFrame?.isMainFrame == false
            {
                decisionHandler(.allow)
            } else {
                UIApplication.shared.open(url)
                decisionHandler(.cancel)
            }
        }

        func webView(
            _ webView: WKWebView,
            runJavaScriptAlertPanelWithMessage message: String,
            initiatedByFrame frame: WKFrameInfo,
            completionHandler: @escaping @MainActor () -> Void
        ) {
            let alert = UIAlertController(title: nil, message: message, preferredStyle: .alert)
            alert.addAction(UIAlertAction(title: "OK", style: .default) { _ in completionHandler() })
            present(alert, over: webView, otherwise: completionHandler)
        }

        func webView(
            _ webView: WKWebView,
            runJavaScriptConfirmPanelWithMessage message: String,
            initiatedByFrame frame: WKFrameInfo,
            completionHandler: @escaping @MainActor (Bool) -> Void
        ) {
            let alert = UIAlertController(title: nil, message: message, preferredStyle: .alert)
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in completionHandler(false) })
            alert.addAction(UIAlertAction(title: "OK", style: .default) { _ in completionHandler(true) })
            present(alert, over: webView) { completionHandler(false) }
        }

        func webView(
            _ webView: WKWebView,
            runJavaScriptTextInputPanelWithPrompt prompt: String,
            defaultText: String?,
            initiatedByFrame frame: WKFrameInfo,
            completionHandler: @escaping @MainActor (String?) -> Void
        ) {
            let alert = UIAlertController(title: nil, message: prompt, preferredStyle: .alert)
            alert.addTextField { $0.text = defaultText }
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in completionHandler(nil) })
            alert.addAction(UIAlertAction(title: "OK", style: .default) { _ in
                completionHandler(alert.textFields?.first?.text ?? "")
            })
            present(alert, over: webView) { completionHandler(nil) }
        }

        /// Shows `alert` from the view controller showing the web view, or,
        /// when nothing is (the app went to the background), answers as a
        /// dismissed dialog would: WebKit waits for an answer.
        private func present(
            _ alert: UIAlertController,
            over webView: WKWebView,
            otherwise dismissed: @escaping () -> Void
        ) {
            var presenter = webView.window?.rootViewController
            while let presented = presenter?.presentedViewController {
                presenter = presented
            }
            guard let presenter else {
                dismissed()
                return
            }
            presenter.present(alert, animated: true)
        }
    }
}
