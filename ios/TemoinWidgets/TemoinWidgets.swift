import AppIntents
import SwiftUI
import WidgetKit

/// Lock Screen / Home Screen widget: one tap opens the app and starts filming.
struct RecordWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "io.github.temoin-app.temoin.record", provider: Provider()) { _ in
            RecordWidgetView()
                .widgetURL(URL(string: "srec://record"))
        }
        .configurationDisplayName("Filmer")
        .description("Ouvre Témoin et commence à filmer.")
        .supportedFamilies([.accessoryCircular, .systemSmall])
    }

    struct Entry: TimelineEntry { let date: Date }

    struct Provider: TimelineProvider {
        func placeholder(in context: Context) -> Entry { Entry(date: .now) }
        func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) { completion(Entry(date: .now)) }
        func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
            completion(Timeline(entries: [Entry(date: .now)], policy: .never))
        }
    }
}

struct RecordWidgetView: View {
    @Environment(\.widgetFamily) private var family

    var body: some View {
        switch family {
        case .accessoryCircular:
            ZStack {
                AccessoryWidgetBackground()
                Image(systemName: "record.circle").font(.title)
            }
            .widgetBackground()
        default:
            VStack(spacing: 8) {
                Image(systemName: "record.circle.fill").font(.system(size: 44)).foregroundStyle(.red)
                Text("Filmer").font(.headline)
            }
            .widgetBackground()
        }
    }
}

private extension View {
    @ViewBuilder func widgetBackground() -> some View {
        if #available(iOS 17.0, *) {
            containerBackground(for: .widget) { Color.black }
        } else {
            background(Color.black)
        }
    }
}

/// Control Center / Lock Screen control and Action button (iOS 18).
@available(iOS 18.0, *)
struct RecordControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "io.github.temoin-app.temoin.control.record") {
            ControlWidgetButton(action: StartRecordingIntent()) {
                Label("Filmer", systemImage: "record.circle")
            }
        }
        .displayName("Filmer")
        .description("Ouvre Témoin et commence à filmer.")
    }
}

@main
struct TemoinWidgetBundle: WidgetBundle {
    var body: some Widget {
        RecordWidget()
        if #available(iOS 18.0, *) {
            RecordControl()
        }
    }
}
