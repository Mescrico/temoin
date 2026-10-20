import TemoinCore
import SwiftUI
import UIKit

struct SettingsView: View {
    @EnvironmentObject var model: AppModel
    @EnvironmentObject var settings: Settings
    @EnvironmentObject var engine: CaptureEngine
    @Environment(\.dismiss) private var dismiss
    @State private var warn1080 = false
    @State private var confirmForget = false

    var body: some View {
        NavigationStack {
            Form {
                DeviceSection()
                HandBackSection()
                if let s = settings.server {
                    Section("s_server") {
                        Text(s.name).bold()
                        Text(s.url).font(.footnote).foregroundStyle(muted)
                        VStack(alignment: .leading, spacing: 4) {
                            Text("s_fingerprint").font(.caption).foregroundStyle(muted)
                            Text(Recipient.fingerprint(s.recipient)).font(.body.monospaced())
                        }
                    }

                    Section("s_quality") {
                        Picker("s_quality", selection: Binding(
                            get: { settings.user.videoHeight > 720 ? 1080 : 720 },
                            set: { v in
                                if v == 1080 { warn1080 = true } else { settings.updateUser { $0.videoHeight = 720 }; engine.settingsChanged() }
                            }
                        )) {
                            Text("q_720").tag(720)
                            Text("q_1080").tag(1080)
                        }
                        .pickerStyle(.inline).labelsHidden()
                        .disabled(s.limits.maxVideoHeight < 1080)
                        if s.limits.maxVideoHeight < 1080 {
                            Text(String(format: NSLocalizedString("q_1080_blocked", comment: ""), s.limits.maxVideoHeight)).font(.caption).foregroundStyle(muted)
                        }
                    }
                }

                Section {
                    ForEach(engine.cameras) { c in
                        Button {
                            engine.selectCamera(c.id)
                        } label: {
                            HStack {
                                Text(c.external ? NSLocalizedString("cam_external", comment: "") : c.name)
                                Spacer()
                                if engine.currentCamera?.id == c.id { Image(systemName: "checkmark") }
                            }
                        }.foregroundStyle(.white)
                    }
                } header: { Text("s_camera") } footer: { Text("cam_external_help_ios") }

                Section {
                    Toggle("s_location", isOn: Binding(get: { settings.user.attachLocation }, set: { v in settings.updateUser { $0.attachLocation = v } }))
                    Toggle("s_wifi", isOn: Binding(get: { settings.user.wifiOnly }, set: { v in settings.updateUser { $0.wifiOnly = v } }))
                } footer: { Text("s_location_help") }

                Section("s_queue") {
                    Text("\(model.queueCount) · \(formatBytes(model.queueBytes))")
                    Button("s_send_now") { model.uploader.kick() }
                    if model.rejected > 0 {
                        Text(String(format: NSLocalizedString("s_rejected", comment: ""), model.rejected)).foregroundStyle(accent)
                        Button("s_clear") { model.outbox.clearRejected() }
                    }
                }

                Section("s_about") {
                    Text("s_about_body").font(.footnote).foregroundStyle(muted)
                    Text("ios_background_note").font(.footnote).foregroundStyle(muted)
                }

                Section {
                    Button("s_forget", role: .destructive) { confirmForget = true }
                }
            }
            .navigationTitle(Text("settings"))
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("ok") { dismiss() } } }
            .alert("q_1080_warn_title", isPresented: $warn1080) {
                Button("q_1080") { settings.updateUser { $0.videoHeight = 1080 }; engine.settingsChanged() }
                Button("cancel", role: .cancel) {}
            } message: { Text("q_1080_warn") }
            .alert("s_forget", isPresented: $confirmForget) {
                Button("s_forget_confirm", role: .destructive) {
                    model.outbox.clearAll()
                    settings.forgetServer()
                    dismiss()
                }
                Button("cancel", role: .cancel) {}
            } message: {
                if model.queueCount > 0 { Text(String(format: NSLocalizedString("s_forget_warn", comment: ""), model.queueCount)) }
            }
        }
    }
}

/// "Mon appareil": pseudonym and usage counters from GET /me (never the list of files).
private struct DeviceSection: View {
    @EnvironmentObject var settings: Settings
    @State private var status: DeviceStatus?
    @State private var failure: String?

    var body: some View {
        Section {
            VStack(alignment: .leading, spacing: 4) {
                Text("s_device_name").font(.caption).foregroundStyle(muted)
                Text(status?.name ?? settings.server?.deviceName ?? "").bold()
            }
            switch failure {
            case "banned": Text("err_banned").foregroundStyle(accent).font(.footnote)
            case "unknown_device": Text("banner_not_registered").foregroundStyle(accent).font(.footnote)
            case nil: EmptyView()
            default: Text("s_device_offline").foregroundStyle(muted).font(.footnote)
            }
            if let d = status {
                ProgressView(value: d.quotaBytes > 0 ? min(1, Double(d.usedBytes) / Double(d.quotaBytes)) : 0)
                Text(String(format: NSLocalizedString("s_device_quota", comment: ""), formatBytes(d.usedBytes), formatBytes(d.quotaBytes))).font(.footnote)
                Text(String(format: NSLocalizedString("s_device_objects", comment: ""), d.objects)).font(.footnote)
                Text(lastUpload(d.lastUploadAt)).font(.footnote).foregroundStyle(muted)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text("s_device_key").font(.caption).foregroundStyle(muted)
                Text(stride(from: 0, to: 16, by: 4).map { i in
                    let id = settings.deviceKey.deviceID
                    let a = id.index(id.startIndex, offsetBy: i)
                    return String(id[a..<id.index(a, offsetBy: 4)])
                }.joined(separator: " ")).font(.footnote.monospaced())
            }
        } header: { Text("s_device") } footer: { Text("s_device_help") }
        .task(id: settings.server?.url) { await refresh() }
    }

    private func refresh() async {
        guard let s = settings.server else { return }
        do {
            status = try await Api.me(s.url, key: settings.deviceKey)
            failure = nil
        } catch ApiError.refused(_, let error) {
            failure = error
        } catch {
            failure = "offline"
        }
    }

    private func lastUpload(_ iso: String?) -> String {
        guard let iso, let date = ISO8601DateFormatter().date(from: iso) else { return NSLocalizedString("s_device_never", comment: "") }
        let f = DateFormatter()
        f.dateStyle = .short
        f.timeStyle = .short
        return String(format: NSLocalizedString("s_device_last", comment: ""), f.string(from: date))
    }
}

/// "Récupérer mes fichiers": the server gives a short code that the person passes to the
/// operator by any channel; the operator answers with a one-time link through another one.
private struct HandBackSection: View {
    @EnvironmentObject var settings: Settings
    @State private var handBack: HandBack?
    @State private var busy = false
    @State private var failure: String?
    @State private var copied = false

    var body: some View {
        Section {
            if let hb = handBack, !hb.closed {
                VStack(alignment: .leading, spacing: 6) {
                    Text("hb_code").font(.caption).foregroundStyle(muted)
                    HStack {
                        Text(hb.code).font(.system(size: 28, weight: .bold, design: .monospaced)).textSelection(.enabled)
                        Spacer()
                        Button(LocalizedStringKey(copied ? "hb_copied" : "hb_copy")) {
                            UIPasteboard.general.string = hb.code
                            copied = true
                        }.buttonStyle(.borderless)
                    }
                    Text(LocalizedStringKey(linkKey(hb.link))).font(.footnote)
                    if let d = shortDate(hb.expiresAt) {
                        Text(String(format: NSLocalizedString("hb_valid", comment: ""), d)).font(.caption).foregroundStyle(muted)
                    }
                }
            } else {
                if let hb = handBack { Text(LocalizedStringKey(linkKey(hb.link))).font(.footnote) }
                Button(LocalizedStringKey(handBack == nil ? "hb_ask" : "hb_again")) { Task { await ask() } }.disabled(busy)
            }
            if let failure {
                Text(String(format: NSLocalizedString("hb_failed", comment: ""), failure)).font(.footnote).foregroundStyle(accent)
            }
        } header: { Text("hb_title") } footer: { Text(LocalizedStringKey((handBack?.closed ?? true) ? "hb_help" : "hb_how")) }
        .task(id: settings.server?.url) { await refresh() }
    }

    private func refresh() async {
        guard let s = settings.server else { return }
        if let st = try? await Api.me(s.url, key: settings.deviceKey) { handBack = st.request }
    }

    private func ask() async {
        guard let s = settings.server else { return }
        busy = true
        defer { busy = false }
        do {
            handBack = try await Api.requestFiles(s.url, key: settings.deviceKey)
            failure = nil
            copied = false
        } catch ApiError.refused(_, let error) {
            failure = error
        } catch {
            failure = error.localizedDescription
        }
    }

    private func linkKey(_ link: String?) -> String {
        switch link {
        case nil: return "hb_pending"
        case "uploading", "ready": return "hb_ready"
        case "opened": return "hb_opened"
        case "done": return "hb_done"
        case "expired": return "hb_expired"
        default: return "hb_revoked"
        }
    }

    private func shortDate(_ iso: String?) -> String? {
        guard let iso, let date = ISO8601DateFormatter().date(from: iso) else { return nil }
        let f = DateFormatter()
        f.dateStyle = .short
        f.timeStyle = .short
        return f.string(from: date)
    }
}
