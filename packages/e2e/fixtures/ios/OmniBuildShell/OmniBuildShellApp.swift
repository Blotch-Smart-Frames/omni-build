import SwiftUI

/// A stand-in for the app shells OmniBuild repackages. As in a real shell,
/// nothing that differs between apps is in the Swift code: the bundle ID,
/// name, version, icon and payload are all data in the bundle, which OmniBuild
/// can rewrite without a compiler. On launch the app reports what it finds
/// there, and the E2E harness checks that against what it built.
@main
struct OmniBuildShellApp: App {
    private let report = ShellReport(bundle: .main)

    init() {
        report.save()
    }

    var body: some Scene {
        WindowGroup {
            ReportView(report: report)
        }
    }
}
