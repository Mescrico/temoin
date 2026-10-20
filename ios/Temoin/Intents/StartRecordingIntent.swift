import AppIntents
import Foundation

/// Shared by the app and the widget extension: asks the app to start filming at once.
/// The app registers `handler`; the flag in UserDefaults covers a cold launch.
enum QuickRecord {
    static let defaultsKey = "quick_record_requested"
    @MainActor static var handler: (() -> Void)?

    @MainActor static func request() {
        UserDefaults.standard.set(true, forKey: defaultsKey)
        handler?()
    }

    /// True once per request.
    static func consume() -> Bool {
        let d = UserDefaults.standard
        guard d.bool(forKey: defaultsKey) else { return false }
        d.removeObject(forKey: defaultsKey)
        return true
    }
}

/// "Film with Témoin": opens the app and starts a Live recording right away.
/// Usable from Siri, the Shortcuts app, Back Tap, the Action button and the widgets.
struct StartRecordingIntent: AppIntent {
    static var title: LocalizedStringResource = "Filmer"
    static var description = IntentDescription("Ouvre Témoin et commence à filmer immédiatement.")
    static var openAppWhenRun: Bool = true

    @MainActor
    func perform() async throws -> some IntentResult {
        QuickRecord.request()
        return .result()
    }
}
