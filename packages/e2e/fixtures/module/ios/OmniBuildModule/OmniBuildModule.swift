import Foundation

/// The E2E tests' native module, as a module's author writes one: a class
/// in a framework of its own, which the fixture shell finds by its
/// Objective-C name (`NSClassFromString`), as the runtime it names,
/// `omnibuild-e2e@1`, registers a module. Nothing compiled the shell
/// against it.
@objc(OmniBuildModule)
public final class OmniBuildModule: NSObject {
    /// What the module says when the shell asks, with the ID of the bundle
    /// its code is in: its own framework's, which it only is once the shell
    /// has loaded the framework OmniBuild added.
    @objc public func report() -> String {
        "OmniBuildModule, in \(Bundle(for: OmniBuildModule.self).bundleIdentifier ?? "no bundle")"
    }
}
