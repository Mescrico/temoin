import TemoinCore
import SwiftUI

private struct Candidate: Identifiable {
    let id = UUID()
    let url: String
    let invite: String
    let info: ServerInfo
    let fromQR: Bool
}

struct OnboardingView: View {
    @EnvironmentObject var model: AppModel
    @EnvironmentObject var settings: Settings
    @State private var manual = false
    @State private var url = ""
    @State private var invite = ""
    @State private var busy = false
    @State private var error: String?
    @State private var candidate: Candidate?
    @State private var scanning = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Circle().fill(accent).frame(width: 56, height: 56).padding(.top, 32)
                Text("Témoin").font(.largeTitle.bold())
                Text("tagline").font(.title3).foregroundStyle(Color(white: 0.75))
                Text("onb_body").foregroundStyle(muted)

                Button { scanning = true } label: {
                    Label("onb_scan", systemImage: "qrcode.viewfinder").frame(maxWidth: .infinity).padding(.vertical, 14)
                }
                .buttonStyle(.borderedProminent).tint(.white).foregroundStyle(.black).disabled(busy)

                if !manual {
                    Button("onb_manual") { manual = true }.frame(maxWidth: .infinity).disabled(busy)
                } else {
                    TextField("onb_url", text: $url, prompt: Text(verbatim: "https://record.example.org"))
                        .textFieldStyle(.roundedBorder).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("onb_invite", text: $invite, prompt: Text(verbatim: "ABCDE FGHJK LMNPQ RSTUV"))
                        .textFieldStyle(.roundedBorder).textInputAutocapitalization(.characters).autocorrectionDisabled()
                    Button("onb_connect") { check(url, invite, expected: nil) }
                        .buttonStyle(.bordered).disabled(busy || url.isEmpty)
                }
                if busy { HStack { ProgressView(); Text("onb_checking") } }
                if let error { Text(error).foregroundStyle(accent) }
            }
            .padding(24)
        }
        .sheet(isPresented: $scanning) {
            QRScannerView { code in
                scanning = false
                guard let link = EnrollLink(code), Recipient.isValid(link.recipient) else {
                    error = NSLocalizedString("err_bad_qr", comment: ""); return
                }
                check(link.url, link.invite ?? "", expected: link.recipient)
            }
            .ignoresSafeArea()
        }
        .onAppear(perform: consumePending)
        .onChange(of: model.pendingLink) { _ in consumePending() }
        .alert(item: $candidate) { c in
            Alert(
                title: Text("onb_confirm_title"),
                message: Text("\(c.info.name)\n\(c.url)\n\n\(Recipient.fingerprint(c.info.recipient))\n\n")
                    + Text(LocalizedStringKey(c.fromQR ? "onb_confirm_qr_body" : "onb_confirm_body")),
                primaryButton: .default(Text("trust")) { join(c) },
                secondaryButton: .cancel { model.pendingLink = nil }
            )
        }
    }

    private func consumePending() {
        guard let link = model.pendingLink else { return }
        guard Recipient.isValid(link.recipient) else { error = NSLocalizedString("err_bad_qr", comment: ""); model.pendingLink = nil; return }
        check(link.url, link.invite ?? "", expected: link.recipient)
    }

    private func check(_ raw: String, _ code: String, expected: String?) {
        if DeviceKey.normalizeInvite(code).isEmpty {
            // A discreet server does not even describe itself without an invitation code.
            error = NSLocalizedString("err_invite_required", comment: ""); manual = true; url = raw
            return
        }
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                let u = try Api.normalize(raw)
                let info = try await Api.info(u, invite: code)
                if !Recipient.isValid(info.recipient) {
                    error = String(format: NSLocalizedString("err_not_server", comment: ""), "bad key")
                } else if let expected, info.recipient != expected {
                    error = NSLocalizedString("err_key_mismatch", comment: "")
                } else {
                    candidate = Candidate(url: u, invite: code, info: info, fromQR: expected != nil)
                }
            } catch ApiError.refused(404, _) {
                self.error = NSLocalizedString("err_no_answer", comment: "")
            } catch {
                self.error = String(format: NSLocalizedString("err_not_server", comment: ""), error.localizedDescription)
            }
        }
    }

    /// Registers this phone's key with the invitation; the key is kept only on success.
    private func join(_ c: Candidate) {
        busy = true
        error = nil
        Task {
            defer { busy = false }
            let key = settings.keyFor(url: c.url, recipient: c.info.recipient)
            switch await Api.register(c.url, key: key, invite: c.invite) {
            case .ok(let name, _):
                settings.adopt(key)
                settings.saveServer(ServerConfig(url: c.url, recipient: c.info.recipient, name: c.info.name, deviceName: name, limits: c.info.limits))
                settings.updateUser { $0.videoHeight = min(c.info.limits.defaultVideoHeight, c.info.limits.maxVideoHeight) }
                model.pendingLink = nil
                model.uploader.kick()
            case .failed(let code, let message):
                let known = ["unknown_invite": "err_invite_unknown", "invite_expired": "err_invite_expired",
                             "invite_revoked": "err_invite_revoked", "invite_exhausted": "err_invite_exhausted", "banned": "err_banned"]
                error = known[code].map { NSLocalizedString($0, comment: "") }
                    ?? String(format: NSLocalizedString("err_register", comment: ""), message)
                model.pendingLink = nil
            }
        }
    }
}
