import SwiftUI

let accent = Color(red: 0.898, green: 0.282, blue: 0.302)
let okGreen = Color(red: 0.188, green: 0.643, blue: 0.424)
let muted = Color(white: 0.62)

struct RootView: View {
    @EnvironmentObject var model: AppModel
    @EnvironmentObject var settings: Settings
    @State private var showSettings = false

    var body: some View {
        Group {
            if settings.server == nil || model.pendingLink != nil {
                OnboardingView()
            } else {
                CaptureView(showSettings: $showSettings)
                    .sheet(isPresented: $showSettings) { SettingsView() }
            }
        }
        .background(Color.black.ignoresSafeArea())
    }
}

func formatBytes(_ n: Int64) -> String {
    ByteCountFormatter.string(fromByteCount: n, countStyle: .file)
}
