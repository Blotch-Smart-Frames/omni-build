import Foundation
import OSLog
import WebKit

/// Sends what the page logs to its console to the unified log, under the
/// app's bundle ID and the category "console", as Android's WebView sends it
/// to logcat: Console.app, `log stream --predicate 'category == "console"'`
/// and `xcrun simctl spawn booted log stream` show a web app's console next
/// to the system's, without Safari's Web Inspector. Uncaught errors and
/// unhandled promise rejections go there too.
///
/// The messages are public in the log, as the page's console is to anyone
/// with the inspector: an app shouldn't log what the device's owner mustn't
/// see.
final class ConsoleHandler: NSObject, WKScriptMessageHandler {
    private static let name = "console"

    /// Wraps the console's methods to post each message to the handler as
    /// well, at the start of every document, frames included.
    private static let script = """
        (function () {
          var handler = window.webkit.messageHandlers.\(name);
          function text(value) {
            if (typeof value === 'string') return value;
            if (value instanceof Error) return value.stack || String(value);
            try {
              var json = JSON.stringify(value);
              return json === undefined ? String(value) : json;
            } catch (error) {
              return String(value);
            }
          }
          function post(level, values) {
            try {
              handler.postMessage({ level: level, message: Array.prototype.map.call(values, text).join(' ') });
            } catch (error) {}
          }
          ['debug', 'info', 'log', 'warn', 'error'].forEach(function (level) {
            var original = console[level];
            console[level] = function () {
              post(level, arguments);
              return original.apply(console, arguments);
            };
          });
          window.addEventListener('error', function (event) {
            post('error', [event.message + ' (' + event.filename + ':' + event.lineno + ')']);
          });
          window.addEventListener('unhandledrejection', function (event) {
            post('error', ['Unhandled rejection: ' + text(event.reason)]);
          });
        })();
        """

    private let logger = Logger(
        subsystem: Bundle.main.bundleIdentifier ?? "WebViewShell",
        category: ConsoleHandler.name
    )

    static func install(in controller: WKUserContentController) {
        controller.addUserScript(
            WKUserScript(source: script, injectionTime: .atDocumentStart, forMainFrameOnly: false)
        )
        controller.add(ConsoleHandler(), name: name)
    }

    /// `console.log()` at the log's default level, which it keeps, rather
    /// than its info level, which it drops unless someone is streaming it.
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let body = message.body as? [String: Any], let text = body["message"] as? String else {
            return
        }
        switch body["level"] as? String {
        case "debug":
            logger.debug("\(text, privacy: .public)")
        case "info":
            logger.info("\(text, privacy: .public)")
        case "warn":
            logger.warning("\(text, privacy: .public)")
        case "error":
            logger.error("\(text, privacy: .public)")
        default:
            logger.notice("\(text, privacy: .public)")
        }
    }
}
