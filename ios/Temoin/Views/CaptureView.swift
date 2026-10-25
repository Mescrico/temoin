import AVFoundation
import SwiftUI

struct CaptureView: View {
    @EnvironmentObject var model: AppModel
    @EnvironmentObject var settings: Settings
    @EnvironmentObject var engine: CaptureEngine
    @Binding var showSettings: Bool
    @Environment(\.scenePhase) private var phase

    @State private var authorized = AVCaptureDevice.authorizationStatus(for: .video) == .authorized
    @State private var micDenied = AVCaptureDevice.authorizationStatus(for: .audio) == .denied
    @State private var accepting = true
    @State private var keyChanged = false
    @State private var discreet = false
    @State private var flashOpacity = 0.0
    @State private var pinchBase: CGFloat?
    /// Screen brightness before we changed it (front light, dark screen). iOS brightness is
    /// global: it must be restored as soon as the app is no longer in front.
    @State private var savedBrightness: CGFloat?

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            if authorized {
                PreviewView(session: engine.session).ignoresSafeArea()
                    .gesture(
                        MagnificationGesture()
                            .onChanged { scale in
                                if pinchBase == nil { pinchBase = engine.zoomFactor }
                                engine.setZoom((pinchBase ?? 1) * scale)
                            }
                            .onEnded { _ in pinchBase = nil }
                    )
            } else {
                VStack(spacing: 16) {
                    Text("perm_needed").multilineTextAlignment(.center)
                    Button("grant") { requestPermissions() }.buttonStyle(.bordered)
                }.padding(32)
            }
            // Front "flash": a translucent white veil, the scene stays visible.
            if engine.flashOn && engine.isFront {
                Color.white.opacity(0.6).ignoresSafeArea().allowsHitTesting(false)
            }
            Color.white.opacity(flashOpacity).ignoresSafeArea().allowsHitTesting(false)

            VStack(spacing: 0) {
                HStack {
                    statusChip
                    Spacer()
                    if engine.hasFlash || engine.isFront {
                        Button { engine.toggleFlash() } label: {
                            Image(systemName: engine.flashOn ? "bolt.fill" : "bolt.slash.fill").font(.title2)
                                .foregroundStyle(engine.flashOn ? Color.yellow : .white)
                                .frame(width: 44, height: 44)
                        }
                    }
                    Button { showSettings = true } label: { Image(systemName: "gearshape.fill").font(.title2) }
                        .disabled(engine.recording).foregroundStyle(.white)
                }
                .padding(.horizontal, 12).padding(.top, 8)
                if keyChanged { banner(NSLocalizedString("err_key_changed", comment: "")) }
                if model.uploadState == .banned { banner(NSLocalizedString("banner_banned", comment: "")) }
                if model.uploadState == .notRegistered { banner(NSLocalizedString("banner_not_registered", comment: "")) }
                if micDenied {
                    banner(NSLocalizedString("banner_no_mic", comment: "")).onTapGesture {
                        if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
                    }
                }
                if let e = engine.error { banner(e).onTapGesture { engine.error = nil } }
                Spacer()

                if engine.recording, let since = engine.recordingSince {
                    RecordingBadge(since: since, sealed: engine.sealedCount)
                } else if engine.mode == .live {
                    Text("live_hint").font(.footnote).foregroundStyle(Color(white: 0.85)).multilineTextAlignment(.center).padding(.horizontal, 32).padding(.bottom, 8)
                }

                if engine.lenses.count > 1 {
                    HStack(spacing: 10) {
                        ForEach(engine.lenses, id: \.self) { l in
                            let selected = engine.lens == l
                            // The selected bubble shows the real zoom while pinching ("3,4×").
                            let text = selected && abs(engine.zoomFactor - l.factor) > 0.02 ? engine.zoomLabel(engine.zoomFactor) : l.label
                            Button { engine.setLens(l) } label: {
                                Text(text)
                                    .font(.system(size: selected ? 13 : 11, weight: .bold))
                                    .foregroundStyle(selected ? Color.yellow : .white)
                                    .frame(width: selected ? 40 : 32, height: selected ? 40 : 32)
                                    .background(.black.opacity(0.5), in: Circle())
                            }
                        }
                    }
                    .padding(6)
                    .background(.black.opacity(0.25), in: Capsule())
                    .padding(.bottom, 6)
                    .animation(.easeOut(duration: 0.15), value: engine.lens)
                }

                if !engine.recording {
                    // Equal widths, and the chosen mode slides right above the shutter (like the system camera).
                    let cell: CGFloat = 96
                    let index = CGFloat(CaptureMode.allCases.firstIndex(of: engine.mode) ?? 0)
                    let middle = CGFloat(CaptureMode.allCases.count - 1) / 2
                    HStack(spacing: 0) {
                        ForEach(CaptureMode.allCases, id: \.self) { m in
                            Button { engine.setMode(m) } label: {
                                Text(LocalizedStringKey("mode_" + m.rawValue)).textCase(.uppercase).font(.subheadline.weight(engine.mode == m ? .bold : .medium))
                                    .foregroundStyle(engine.mode == m ? (m == .photo ? .white : accent) : muted)
                                    .frame(width: cell).padding(.vertical, 8)
                                    .contentShape(Rectangle())
                            }
                        }
                    }
                    .offset(x: (middle - index) * cell)
                    .animation(.easeOut(duration: 0.2), value: engine.mode)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                }

                HStack {
                    ZStack {
                        if engine.recording && engine.mode != .photo {
                            Button { discreet = true } label: { Image(systemName: "moon.fill").font(.title2) }.foregroundStyle(.white)
                        }
                    }.frame(width: 52, height: 52)
                    Spacer()
                    shutter
                    Spacer()
                    ZStack {
                        if !engine.recording && engine.cameras.count > 1 {
                            Button {
                                let i = engine.cameras.firstIndex { $0.id == engine.currentCamera?.id } ?? 0
                                engine.selectCamera(engine.cameras[(i + 1) % engine.cameras.count].id)
                            } label: { Image(systemName: "arrow.triangle.2.circlepath.camera").font(.title2) }.foregroundStyle(.white)
                        }
                    }.frame(width: 52, height: 52)
                }
                .padding(.horizontal, 32).padding(.bottom, 24).padding(.top, 8)
            }

            if discreet { DiscreetOverlay { discreet = false } }
        }
        .statusBarHidden(discreet)
        .onAppear {
            if authorized { engine.start(); askMicrophoneIfNeeded() } else { requestPermissions() }
            startQuickRecordIfAsked()
        }
        .onChange(of: model.quickRecord) { _ in startQuickRecordIfAsked() }
        .onDisappear { engine.stop() }
        .onChange(of: phase) { p in
            // iOS stops the camera in the background: close the recording cleanly.
            if p == .background { engine.stop() } else if p == .active && authorized { engine.start(); askMicrophoneIfNeeded() }
        }
        .onChange(of: engine.flash) { _ in
            flashOpacity = 0.8
            withAnimation(.easeOut(duration: 0.25)) { flashOpacity = 0 }
        }
        .onChange(of: discreet) { _ in applyBrightness() }
        .onChange(of: engine.flashOn) { _ in applyBrightness() }
        .onChange(of: engine.currentCamera) { _ in applyBrightness() }
        .onChange(of: phase) { p in if p == .active { applyBrightness() } else { restoreBrightness() } }
        .onDisappear { restoreBrightness() }
        .task(id: settings.server?.url) { await healthLoop() }
    }

    /// Launched from the "Filmer" intent / widget: switch to Live and start recording,
    /// once the camera had a moment to start.
    private func startQuickRecordIfAsked() {
        guard model.quickRecord || QuickRecord.consume() else { return }
        model.quickRecord = false
        _ = QuickRecord.consume()
        guard authorized, settings.server != nil, !engine.recording else { return }
        Task {
            try? await Task.sleep(nanoseconds: 900_000_000)
            engine.setMode(.live)
            try? await Task.sleep(nanoseconds: 600_000_000)
            if !engine.recording { engine.startRecording() }
        }
    }

    /// Dark screen wins over the front light; nothing requested = original brightness.
    private func applyBrightness() {
        let want: CGFloat? = discreet ? 0.01 : (engine.flashOn && engine.isFront ? 1.0 : nil)
        if let want {
            if savedBrightness == nil { savedBrightness = UIScreen.main.brightness }
            UIScreen.main.brightness = want
        } else {
            restoreBrightness()
        }
    }

    private func restoreBrightness() {
        if let s = savedBrightness { UIScreen.main.brightness = s }
        savedBrightness = nil
    }

    private var shutter: some View {
        let enabled = authorized && !engine.busy && !keyChanged && engine.currentCamera != nil
        return Button {
            switch (engine.mode, engine.recording) {
            case (.photo, _): engine.takePhoto()
            case (_, true): engine.stopRecording()
            default: engine.startRecording()
            }
        } label: {
            ZStack {
                Circle().stroke(.white, lineWidth: 4).frame(width: 78, height: 78)
                if engine.recording {
                    RoundedRectangle(cornerRadius: 6).fill(accent).frame(width: 30, height: 30)
                } else {
                    Circle().fill(enabled ? (engine.mode == .photo ? .white : accent) : .gray).frame(width: 62, height: 62)
                }
            }
        }
        .disabled(!enabled)
    }

    private var statusChip: some View {
        let (icon, tint): (String, Color) = {
            if model.uploadState == .banned || model.uploadState == .notRegistered { return ("nosign", accent) }
            if !accepting || model.uploadState == .pausedByServer { return ("pause.circle.fill", .orange) }
            if model.queueCount == 0 { return ("checkmark.icloud.fill", okGreen) }
            if model.uploadState == .waitingNetwork { return ("icloud.slash.fill", .orange) }
            return ("icloud.and.arrow.up.fill", .white)
        }()
        let detail: String = {
            if model.queueCount == 0 { return NSLocalizedString("queue_empty", comment: "") }
            let st: String
            switch model.uploadState {
            case .waitingNetwork: st = NSLocalizedString("up_waiting", comment: "")
            case .pausedByServer: st = NSLocalizedString("up_paused", comment: "")
            case .banned: st = NSLocalizedString("up_banned", comment: "")
            case .notRegistered: st = NSLocalizedString("up_not_registered", comment: "")
            case .error: st = NSLocalizedString("up_error", comment: "")
            default: st = NSLocalizedString("up_uploading", comment: "")
            }
            return String(format: NSLocalizedString("queue_pending", comment: ""), model.queueCount, st) + " · " + formatBytes(model.queueBytes)
        }()
        return HStack(spacing: 8) {
            Image(systemName: icon).foregroundStyle(tint)
            VStack(alignment: .leading, spacing: 1) {
                Text(settings.server?.name ?? "").font(.footnote.weight(.semibold)).lineLimit(1)
                Text(detail).font(.caption2).foregroundStyle(Color(white: 0.75)).lineLimit(1)
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 6)
        .background(.black.opacity(0.55), in: Capsule())
    }

    private func banner(_ text: String) -> some View {
        Text(text).font(.footnote).frame(maxWidth: .infinity, alignment: .leading).padding(12)
            .background(accent.opacity(0.9), in: RoundedRectangle(cornerRadius: 12)).padding(.horizontal, 12).padding(.vertical, 4)
    }

    /// The camera may already be allowed by the invitation scanner, which never asks for the
    /// microphone: ask for it here, otherwise every video would be recorded without sound.
    private func askMicrophoneIfNeeded() {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .notDetermined:
            Task {
                let ok = await AVCaptureDevice.requestAccess(for: .audio)
                micDenied = !ok
            }
        case .authorized: micDenied = false
        default: micDenied = true
        }
    }

    private func requestPermissions() {
        Task {
            let v = await AVCaptureDevice.requestAccess(for: .video)
            _ = await AVCaptureDevice.requestAccess(for: .audio)
            micDenied = AVCaptureDevice.authorizationStatus(for: .audio) != .authorized
            authorized = v
            if v { engine.start() }
        }
    }

    /// Server health + limits. The pinned key is never replaced.
    private func healthLoop() async {
        while !Task.isCancelled {
            if let s = settings.server, let info = try? await Api.info(s.url, key: settings.deviceKey) {
                keyChanged = info.recipient != s.recipient
                accepting = info.accepting
                if !keyChanged { settings.updateLimits(info.limits) }
            }
            model.uploader.kick()
            try? await Task.sleep(nanoseconds: 60_000_000_000)
        }
    }
}

private struct RecordingBadge: View {
    let since: Date
    let sealed: Int

    var body: some View {
        VStack(spacing: 6) {
            TimelineView(.periodic(from: .now, by: 0.5)) { ctx in
                let s = max(0, Int(ctx.date.timeIntervalSince(since)))
                HStack(spacing: 8) {
                    Circle().fill(.white).frame(width: 8, height: 8)
                    Text(String(format: "%02d:%02d:%02d", s / 3600, s / 60 % 60, s % 60)).bold().monospacedDigit()
                }
                .padding(.horizontal, 14).padding(.vertical, 4).background(accent, in: Capsule())
            }
            Label(String(format: NSLocalizedString("sealed_info", comment: ""), sealed), systemImage: "lock.fill")
                .font(.caption2).foregroundStyle(Color(white: 0.75))
        }
    }
}

private struct DiscreetOverlay: View {
    let onExit: () -> Void
    var body: some View {
        ZStack(alignment: .bottom) {
            Color.black.ignoresSafeArea()
            Text("discreet_hint").font(.caption2).foregroundStyle(Color(white: 0.16)).padding(32)
        }
        .contentShape(Rectangle())
        .onTapGesture(count: 3, perform: onExit)
    }
}
