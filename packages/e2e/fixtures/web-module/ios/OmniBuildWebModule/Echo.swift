import Foundation

/// The E2E tests' native module for the WebView shell, as a module's author
/// writes one for its runtime, `webview@1`: a class the shell finds by its
/// Objective-C name, which the page calls by that name, `Echo`, through
/// `omniBuild.call`. Nothing compiled the shell against it.
@objc(Echo)
public final class Echo: NSObject {
    /// `echo` replies with what the page gave it, and the ID of the bundle
    /// its code is in: its own framework's, which it only is once the shell
    /// has loaded the framework OmniBuild added. Any other method fails, as
    /// the page hears.
    @objc public func call(_ method: String, argument: String, reply: @escaping (String?, String?) -> Void) {
        guard method == "echo" else {
            reply(nil, "Echo has no method \(method)")
            return
        }
        let echoed =
            (try? JSONSerialization.jsonObject(with: Data(argument.utf8), options: .fragmentsAllowed))
            ?? NSNull()
        let from = Bundle(for: Echo.self).bundleIdentifier ?? "no bundle"
        let answer = try? JSONSerialization.data(
            withJSONObject: ["echo": echoed, "from": from], options: .sortedKeys)
        reply(answer.flatMap { String(data: $0, encoding: .utf8) }, nil)
    }
}
