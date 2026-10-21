import SwiftUI
import WidgetKit

/// A stand-in for the widgets a watch app may carry, as its complications,
/// as Xcode's template makes them: an app extension in the watch app's own
/// PlugIns/, code nested in code nested in the app. It's there to be
/// repackaged: OmniBuild gives it an ID under the watch app's, the app's
/// version and build, and a signature of its own before the watch app's.
@main
struct ShellWatchWidgets: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "ShellWatchWidgets", provider: Provider()) { _ in
            Text("ShellWatch")
                .containerBackground(.clear, for: .widget)
        }
        .supportedFamilies([.accessoryCircular])
    }
}

struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> Entry {
        Entry(date: .now)
    }

    func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) {
        completion(Entry(date: .now))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
        completion(Timeline(entries: [Entry(date: .now)], policy: .never))
    }
}

struct Entry: TimelineEntry {
    let date: Date
}
