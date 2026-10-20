import AppIntents
import ExtensionFoundation

/// A stand-in for the ExtensionKit extensions a shell may carry, such as
/// this App Intents extension, as Xcode's template makes it. Unlike an app
/// extension in PlugIns/, Xcode puts it in the app's Extensions/. It's there
/// to be repackaged: OmniBuild gives it an ID under the app's, the app's
/// version and build, and a signature of its own before the app's.
@main
struct ShellIntentsExtension: AppIntentsExtension {
}
