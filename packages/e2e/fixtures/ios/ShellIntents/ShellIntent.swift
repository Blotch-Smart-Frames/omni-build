import AppIntents

/// The extension's one intent, as Xcode's template has it: an App Intents
/// extension with none fails to build.
struct ShellIntent: AppIntent {
    static var title: LocalizedStringResource { "ShellIntents" }

    func perform() async throws -> some IntentResult {
        return .result()
    }
}
