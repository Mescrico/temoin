import Foundation
import TemoinCore

struct ServerLimits: Codable, Equatable {
    var maxObjectBytes: Int = 16 * 1024 * 1024
    var maxVideoHeight: Int = 1080
    var defaultVideoHeight: Int = 720
    var videoBitrate720: Int = 2_500_000
    var videoBitrate1080: Int = 5_000_000
    var photoMaxPixels: Int = 4_000_000
}

/// The enrolled server. The recipient is pinned at enrollment and never silently replaced.
struct ServerConfig: Codable, Equatable {
    var url: String
    var recipient: String
    var name: String
    /// Pseudonym given by the server at registration ("Phone #37").
    var deviceName: String
    var limits: ServerLimits
}

struct UserPrefs: Codable, Equatable {
    var videoHeight: Int = 720
    var attachLocation = false
    var wifiOnly = false
    var cameraID: String?
}

@MainActor
final class Settings: ObservableObject {
    private let defaults = UserDefaults.standard

    @Published private(set) var server: ServerConfig?
    @Published private(set) var user: UserPrefs

    /// This phone's Ed25519 identity on the current server (one key per server joined).
    private(set) var deviceKey: DeviceKey = KeyStore.current()

    init() {
        server = Self.load(ServerConfig.self, "server")
        user = Self.load(UserPrefs.self, "user") ?? UserPrefs()
    }

    func saveServer(_ s: ServerConfig) {
        server = s
        save(s, "server")
        if user.videoHeight > s.limits.maxVideoHeight { updateUser { $0.videoHeight = s.limits.maxVideoHeight } }
    }

    /// Refreshes limits from /info. The recipient is NOT updated here on purpose.
    func updateLimits(_ l: ServerLimits) {
        guard var s = server, s.limits != l else { return }
        s.limits = l
        saveServer(s)
    }

    /// Key to use for joining a server: the current one for the same server (same pinned key,
    /// even at a new address: a home server behind a quick tunnel changes address on restart),
    /// a fresh one otherwise (so two servers cannot link the same phone). Stored only once
    /// registration succeeds.
    func keyFor(url: String, recipient: String) -> DeviceKey {
        if let s = server, s.recipient == recipient { return deviceKey }
        return DeviceKey()
    }

    func adopt(_ key: DeviceKey) {
        KeyStore.save(key)
        deviceKey = key
    }

    func forgetServer() {
        server = nil
        defaults.removeObject(forKey: "server")
    }

    func updateUser(_ f: (inout UserPrefs) -> Void) {
        var u = user
        f(&u)
        user = u
        save(u, "user")
    }

    private func save<T: Encodable>(_ v: T, _ key: String) {
        defaults.set(try? JSONEncoder().encode(v), forKey: key)
    }

    /// Thread-safe read used by the background uploader (UserDefaults is thread-safe).
    nonisolated static func snapshot() -> (server: ServerConfig?, key: DeviceKey?, wifiOnly: Bool) {
        (load(ServerConfig.self, "server"), KeyStore.load(), load(UserPrefs.self, "user")?.wifiOnly ?? false)
    }

    nonisolated private static func load<T: Decodable>(_ t: T.Type, _ key: String) -> T? {
        guard let d = UserDefaults.standard.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(t, from: d)
    }
}
