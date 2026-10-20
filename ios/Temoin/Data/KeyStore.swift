import Foundation
import Security
import TemoinCore

/// Keeps the device's Ed25519 key in the Keychain: readable after the first unlock
/// (so background uploads can sign), never synced to iCloud, never in backups.
enum KeyStore {
    private static let service = "io.github.temoin-app.temoin.device-key"

    static func load() -> DeviceKey? {
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return try? DeviceKey(rawRepresentation: data)
    }

    static func save(_ key: DeviceKey) {
        let base: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service]
        SecItemDelete(base as CFDictionary)
        var add = base
        add[kSecValueData as String] = key.rawRepresentation
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(add as CFDictionary, nil)
    }

    /// Current key, created on first use.
    static func current() -> DeviceKey {
        if let k = load() { return k }
        let k = DeviceKey()
        save(k)
        return k
    }
}
