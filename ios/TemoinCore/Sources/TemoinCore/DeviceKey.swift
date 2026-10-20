import Foundation
#if canImport(CryptoKit)
import CryptoKit
#else
import Crypto
#endif

/// This phone's identity on the enrolled server: an Ed25519 key pair generated on
/// the phone. The server only sees the public key and derives a pseudonymous device
/// id from it. A new key is generated for every server joined, so two servers
/// cannot link the same phone. Mirrors Android `DeviceKey.kt` and server `devices.rs`.
public struct DeviceKey {
    let key: Curve25519.Signing.PrivateKey

    public init() { key = Curve25519.Signing.PrivateKey() }

    public init(rawRepresentation: Data) throws {
        key = try Curve25519.Signing.PrivateKey(rawRepresentation: rawRepresentation)
    }

    /// 32-byte seed, to store in the Keychain.
    public var rawRepresentation: Data { key.rawRepresentation }
    public var publicKey: Data { key.publicKey.rawRepresentation }
    public var publicKeyB64: String { publicKey.base64EncodedString() }
    public var deviceID: String { DeviceKey.deviceID(publicKey: publicKey) }

    /// Base64 Ed25519 signature of `message` (UTF-8).
    public func sign(_ message: String) throws -> String {
        try key.signature(for: Data(message.utf8)).base64EncodedString()
    }

    /// First 16 bytes of SHA-256(public key), lowercase hex — same as the server.
    public static func deviceID(publicKey: Data) -> String {
        SHA256.hash(data: publicKey).prefix(16).map { String(format: "%02x", $0) }.joined()
    }

    /// String signed for every authenticated request (PROTOCOL.md section 4).
    public static func requestMessage(method: String, path: String, bodySHA256: String?, timestamp: Int64, deviceID: String) -> String {
        "SRAUTH1\n\(method)\n\(path)\n\(bodySHA256 ?? "-")\n\(timestamp)\n\(deviceID)"
    }

    /// String signed when registering with an invitation.
    public static func registerMessage(invite: String, publicKeyB64: String, timestamp: Int64) -> String {
        "SRREG1\n\(invite)\n\(publicKeyB64)\n\(timestamp)"
    }

    /// Invitation codes are displayed in groups; keep letters and digits only, uppercase.
    public static func normalizeInvite(_ code: String) -> String {
        String(code.uppercased().filter { $0.isLetter || $0.isNumber })
    }
}
