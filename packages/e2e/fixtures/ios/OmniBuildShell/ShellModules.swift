import Foundation

/// The native modules OmniBuild added to the app, registered as the shell's
/// runtime, `omnibuild-e2e@1` (`OmniBuildRuntime` in Info.plist), registers
/// a module: once the frameworks Info.plist lists are loaded from the app's
/// Frameworks/, each module is a class found by its name, made with no
/// arguments, which says something when asked to `report`. Nothing compiled
/// the shell against them, so it finds them as the Objective-C runtime
/// does, by name.
enum ShellModules {
    /// What each module said, in the order Info.plist lists them, or nil
    /// when the app has none.
    static func reports(of bundle: Bundle) -> String? {
        let info = bundle.infoDictionary ?? [:]
        for name in info["OmniBuildModuleFrameworks"] as? [String] ?? [] {
            bundle.privateFrameworksURL
                .flatMap { Bundle(url: $0.appendingPathComponent(name)) }?
                .load()
        }
        let modules = info["OmniBuildModules"] as? [String] ?? []
        return modules.isEmpty ? nil : modules.map(report).joined(separator: ", ")
    }

    /// What the module `name` says, or why it says nothing.
    private static func report(of name: String) -> String {
        guard let module = (NSClassFromString(name) as? NSObject.Type)?.init() else {
            return "\(name): no such class"
        }
        let report = NSSelectorFromString("report")
        guard module.responds(to: report),
            let said = module.perform(report)?.takeUnretainedValue() as? String
        else {
            return "\(name): no report"
        }
        return said
    }
}
