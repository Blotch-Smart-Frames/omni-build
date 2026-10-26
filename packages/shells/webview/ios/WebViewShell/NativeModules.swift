import Foundation
import WebKit

/// The native modules OmniBuild added to the app, which the page calls from
/// JavaScript, as the WebView shell's runtime, `webview@1`
/// (`OmniBuildRuntime` in Info.plist), has them:
///
/// ```js
/// omniBuild.modules; // ['Haptics']
/// const result = await omniBuild.call('Haptics', 'impact', { style: 'heavy' });
/// ```
///
/// Nothing compiled the shell against them, so it finds them as the
/// Objective-C runtime does, by name: it loads each framework Info.plist
/// lists (`OmniBuildModuleFrameworks`), then makes each class it lists
/// (`OmniBuildModules`) with no arguments. The page calls a module by its
/// class's own name, without its Swift module (`Haptics` for
/// `HapticsKit.Haptics`), and the module answers through a method of its
/// own, which a module's class declares as:
///
/// ```swift
/// @objc(Haptics)
/// public final class Haptics: NSObject {
///     @objc public func call(_ method: String, argument: String, reply: @escaping (String?, String?) -> Void)
/// }
/// ```
///
/// The argument is JSON, and the module replies once, with JSON or with why
/// it failed, on any thread. The page's call resolves with the JSON parsed,
/// or rejects with an Error of the module's message. Only the payload's own
/// pages reach native code: a frame from elsewhere in them, such as an
/// embedded video, doesn't.
final class NativeModules: NSObject, WKScriptMessageHandlerWithReply {
    private static let handler = "omniBuild"
    /// The selector of a module's `call(_:argument:reply:)`.
    private static let call = NSSelectorFromString("call:argument:reply:")
    private typealias Call = @convention(c) (
        NSObject, Selector, NSString, NSString, @escaping @convention(block) (NSString?, NSString?) -> Void
    ) -> Void

    /// Each module, by the name the page calls it by.
    private let modules: [String: NSObject]

    private init(modules: [String: NSObject]) {
        self.modules = modules
    }

    /// Gives every page of the payload `window.omniBuild`, before its own
    /// scripts run, and the modules to call through it.
    static func install(in controller: WKUserContentController, from bundle: Bundle = .main) {
        let modules = NativeModules(modules: load(from: bundle))
        let names = modules.modules.keys.sorted()
        let namesJSON = (try? JSONSerialization.data(withJSONObject: names))
            .flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
        controller.addUserScript(
            WKUserScript(
                source: script(names: namesJSON),
                injectionTime: .atDocumentStart,
                forMainFrameOnly: false
            )
        )
        controller.addScriptMessageHandler(modules, contentWorld: .page, name: handler)
    }

    /// The modules Info.plist lists, by the name the page calls each by,
    /// once their frameworks are loaded.
    private static func load(from bundle: Bundle) -> [String: NSObject] {
        let info = bundle.infoDictionary ?? [:]
        for name in info["OmniBuildModuleFrameworks"] as? [String] ?? [] {
            bundle.privateFrameworksURL
                .flatMap { Bundle(url: $0.appendingPathComponent(name)) }?
                .load()
        }
        var modules: [String: NSObject] = [:]
        for name in info["OmniBuildModules"] as? [String] ?? [] {
            guard let module = (NSClassFromString(name) as? NSObject.Type)?.init() else {
                NSLog("OmniBuild: the app has no class %@ for a native module", name)
                continue
            }
            modules[String(name.split(separator: ".").last ?? Substring(name))] = module
        }
        return modules
    }

    /// `window.omniBuild`. postMessage to a handler that replies returns a
    /// promise of the reply, so a call is one message.
    private static func script(names: String) -> String {
        """
        (function () {
          var handler = window.webkit && window.webkit.messageHandlers.\(handler);
          window.omniBuild = {
            modules: \(names),
            call: function (module, method, argument) {
              if (!handler) {
                return Promise.reject(new Error('Only the app\\u2019s own pages call its native modules'));
              }
              return handler
                .postMessage({
                  module: String(module),
                  method: String(method),
                  argument: JSON.stringify(argument === undefined ? null : argument),
                })
                .then(function (result) {
                  return JSON.parse(result);
                });
            },
          };
        })();
        """
    }

    func userContentController(
        _ controller: WKUserContentController,
        didReceive message: WKScriptMessage,
        replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void
    ) {
        // The payload's own pages, from the shell's scheme, and none other:
        // the script runs in every frame, so that the page's own frames can
        // call modules, but a frame from elsewhere mustn't.
        guard message.frameInfo.securityOrigin.protocol == PayloadSchemeHandler.scheme else {
            replyHandler(nil, "Only the app's own pages call its native modules")
            return
        }
        guard let body = message.body as? [String: Any],
            let name = body["module"] as? String,
            let method = body["method"] as? String,
            let argument = body["argument"] as? String
        else {
            replyHandler(nil, "A call names a module and a method")
            return
        }
        guard let module = modules[name] else {
            replyHandler(nil, "The app has no native module \(name)")
            return
        }
        guard module.responds(to: Self.call) else {
            replyHandler(nil, "\(name) has no call(_:argument:reply:) for the page to call")
            return
        }
        let call = unsafeBitCast(module.method(for: Self.call), to: Call.self)
        // A module replies once, but may do it from any thread: the page
        // hears on the main one.
        call(module, Self.call, method as NSString, argument as NSString) { result, error in
            let result = result.map { $0 as String }
            let error = error.map { $0 as String }
            Task { @MainActor in
                if let error {
                    replyHandler(nil, error)
                } else {
                    replyHandler(result ?? "null", nil)
                }
            }
        }
    }
}
