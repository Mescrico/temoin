import AppIntents

/// Siri / Shortcuts / Back Tap / Action button (app target only, not the widget extension).
struct TemoinShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: StartRecordingIntent(),
            phrases: [
                "Filmer avec \(.applicationName)",
                "Enregistrer avec \(.applicationName)",
                "Film with \(.applicationName)",
            ],
            shortTitle: "Filmer",
            systemImageName: "record.circle"
        )
    }
}
