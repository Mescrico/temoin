import Foundation
#if canImport(CryptoKit)
import CryptoKit
#else
import Crypto
#endif

/// age v1 encryption to a single X25519 recipient (https://age-encryption.org/v1).
///
/// Encrypt-only on purpose: the app never holds an identity, so nothing it
/// writes can be read back on the phone. Mirrors `server/src/seal.rs` and the
/// Android `AgeSealer.kt`.
public enum AgeSealer {
    static let chunkSize = 64 * 1024
    static let versionLine = Data("age-encryption.org/v1\n".utf8)
    static let x25519Info = Data("age-encryption.org/v1/X25519".utf8)

    public static func seal(recipient: Data, plaintext: Data) throws -> Data {
        precondition(recipient.count == 32, "recipient must be 32 bytes")
        let fileKey = SymmetricKey(size: .bits128)

        // X25519 stanza
        let eph = Curve25519.KeyAgreement.PrivateKey()
        let ephPub = eph.publicKey.rawRepresentation
        let shared = try eph.sharedSecretFromKeyAgreement(with: Curve25519.KeyAgreement.PublicKey(rawRepresentation: recipient))
        let wrapKey = shared.hkdfDerivedSymmetricKey(using: SHA256.self, salt: ephPub + recipient, sharedInfo: x25519Info, outputByteCount: 32)
        let fileKeyBytes = fileKey.withUnsafeBytes { Data($0) }
        let wrapped = try ChaChaPoly.seal(fileKeyBytes, using: wrapKey, nonce: ChaChaPoly.Nonce(data: Data(count: 12)))
        let body = wrapped.ciphertext + wrapped.tag

        var header = versionLine
        header += Data("-> X25519 \(b64(ephPub))\n".utf8)
        header += Data("\(b64(body))\n".utf8)
        header += Data("---".utf8)

        let macKey = HKDF<SHA256>.deriveKey(inputKeyMaterial: fileKey, salt: Data(), info: Data("header".utf8), outputByteCount: 32)
        let mac = HMAC<SHA256>.authenticationCode(for: header, using: macKey)
        header += Data(" \(b64(Data(mac)))\n".utf8)

        // STREAM payload
        var g = SystemRandomNumberGenerator()
        let nonce = Data((0..<16).map { _ in UInt8.random(in: 0...255, using: &g) })
        let payloadKey = HKDF<SHA256>.deriveKey(inputKeyMaterial: fileKey, salt: nonce, info: Data("payload".utf8), outputByteCount: 32)

        var out = Data(capacity: header.count + 16 + plaintext.count + (plaintext.count / chunkSize + 1) * 16)
        out += header
        out += nonce
        let chunks = plaintext.isEmpty ? 1 : (plaintext.count + chunkSize - 1) / chunkSize
        for i in 0..<chunks {
            let from = plaintext.startIndex + i * chunkSize
            let to = min(from + chunkSize, plaintext.endIndex)
            var n = Data(count: 12)
            var c = UInt64(i)
            for b in stride(from: 10, through: 3, by: -1) { n[b] = UInt8(c & 0xff); c >>= 8 }
            if i == chunks - 1 { n[11] = 1 }
            let box = try ChaChaPoly.seal(plaintext[from..<to], using: payloadKey, nonce: ChaChaPoly.Nonce(data: n))
            out += box.ciphertext
            out += box.tag
        }
        return out
    }

    static func b64(_ d: Data) -> String {
        d.base64EncodedString().replacingOccurrences(of: "=", with: "")
    }
}

/// `age1…` recipients: bech32 (BIP-173) with HRP "age" encoding a 32-byte X25519 key.
public enum Recipient {
    static let charset = Array("qpzry9x8gf2tvdw0s3jn54khce6mua7l")

    public enum ParseError: Error { case invalid(String) }

    public static func parse(_ s: String) throws -> Data {
        let str = s.trimmingCharacters(in: .whitespacesAndNewlines)
        guard str == str.lowercased() else { throw ParseError.invalid("recipient must be lowercase") }
        guard let sep = str.lastIndex(of: "1"), str[..<sep] == "age" else { throw ParseError.invalid("not an age1… recipient") }
        var data: [Int] = []
        for ch in str[str.index(after: sep)...] {
            guard let v = charset.firstIndex(of: ch) else { throw ParseError.invalid("invalid bech32 character") }
            data.append(v)
        }
        guard data.count >= 6, polymod(hrpExpand("age") + data) == 1 else { throw ParseError.invalid("checksum mismatch") }
        let bytes = try convertBits(Array(data.dropLast(6)), from: 5, to: 8)
        guard bytes.count == 32 else { throw ParseError.invalid("recipient must encode 32 bytes") }
        return Data(bytes)
    }

    public static func isValid(_ s: String) -> Bool { (try? parse(s)) != nil }

    /// Same as `sr fingerprint`: first 10 bytes of SHA-256(recipient), uppercase hex, groups of 4.
    public static func fingerprint(_ s: String) -> String {
        let digest = SHA256.hash(data: Data(s.trimmingCharacters(in: .whitespacesAndNewlines).utf8))
        let hex = digest.prefix(10).map { String(format: "%02X", $0) }.joined()
        var groups: [String] = []
        var i = hex.startIndex
        while i < hex.endIndex {
            let j = hex.index(i, offsetBy: 4, limitedBy: hex.endIndex) ?? hex.endIndex
            groups.append(String(hex[i..<j]))
            i = j
        }
        return groups.joined(separator: " ")
    }

    static func hrpExpand(_ hrp: String) -> [Int] {
        let v = hrp.unicodeScalars.map { Int($0.value) }
        return v.map { $0 >> 5 } + [0] + v.map { $0 & 31 }
    }

    static func polymod(_ values: [Int]) -> Int {
        let gen = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
        var chk = 1
        for v in values {
            let top = chk >> 25
            chk = ((chk & 0x1ffffff) << 5) ^ v
            for i in 0..<5 where (top >> i) & 1 == 1 { chk ^= gen[i] }
        }
        return chk
    }

    static func convertBits(_ data: [Int], from: Int, to: Int) throws -> [UInt8] {
        var acc = 0, bits = 0
        var out: [UInt8] = []
        let maxv = (1 << to) - 1
        for v in data {
            acc = (acc << from) | v
            bits += from
            while bits >= to {
                bits -= to
                out.append(UInt8((acc >> bits) & maxv))
            }
        }
        guard bits < from, ((acc << (to - bits)) & maxv) == 0 else { throw ParseError.invalid("invalid padding") }
        return out
    }
}
