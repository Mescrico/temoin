import TemoinCore
import SwiftUI

@main
struct TemoinApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @StateObject private var model = AppModel.shared
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(model)
                .environmentObject(model.settings)
                .environmentObject(model.engine)
                .preferredColorScheme(.dark)
                .onOpenURL { url in
                    if url.scheme == "srec" && url.host == "record" {
                        QuickRecord.request()
                    } else if let link = EnrollLink(url.absoluteString) {
                        model.pendingLink = link
                    }
                }
        }
        .onChange(of: phase) { p in
            if p == .active { model.uploader.kick() }
        }
    }
}

final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, handleEventsForBackgroundURLSession identifier: String, completionHandler: @escaping () -> Void) {
        AppModel.shared.uploader.backgroundCompletion = completionHandler
        AppModel.shared.uploader.kick()
    }
}

@MainActor
final class AppModel: ObservableObject {
    static let shared = AppModel()

    let settings = Settings()
    let outbox = Outbox()
    let uploader: Uploader
    private(set) lazy var engine = CaptureEngine(model: self)

    @Published var pendingLink: EnrollLink?
    /// Set by the "Filmer" intent / widget / srec://record: start a Live recording at once.
    @Published var quickRecord = false
    /// The widget was used before any server was joined: the welcome screen explains it.
    @Published var serverNeeded = false
    @Published var queueCount = 0
    @Published var queueBytes: Int64 = 0
    @Published var rejected = 0
    @Published var uploadState: Uploader.State = .idle

    private init() {
        uploader = Uploader(outbox: outbox)
        uploader.config = { Settings.snapshot() }
        uploader.onState = { [weak self] s in Task { @MainActor in self?.uploadState = s } }
        outbox.onChange = { [weak self] in Task { @MainActor in self?.refreshQueue() } }
        QuickRecord.handler = { [weak self] in self?.quickRecordRequested() }
        if QuickRecord.consume() { quickRecordRequested(consumed: true) }
        refreshQueue()
    }

    /// Without a server there is nothing to film for: stay on the welcome screen and drop the
    /// request, so that joining a server later does not start a recording by surprise.
    func quickRecordRequested(consumed: Bool = false) {
        if settings.server == nil {
            if !consumed { _ = QuickRecord.consume() }
            serverNeeded = true
        } else {
            quickRecord = true
        }
    }

    func refreshQueue() {
        let s = outbox.stats()
        queueCount = s.count
        queueBytes = s.bytes
        rejected = outbox.rejectedCount()
    }
}
