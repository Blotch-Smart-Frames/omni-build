import SwiftUI

/// Shows the report on screen, for the harness's screenshot and for anyone
/// running the app by hand.
struct ReportView: View {
    let report: ShellReport

    var body: some View {
        List {
            row("ID", report.id)
            row("Name", report.name)
            row("Version", report.version)
            row("Build", report.build)
            row("Payload", report.payload)
            row("Icon", report.icon)
        }
    }

    private func row(_ label: String, _ value: String?) -> some View {
        LabeledContent(label, value: value ?? "missing")
    }
}
