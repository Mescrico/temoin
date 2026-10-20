import Foundation

/// Plaintext inside every age object: `"SRP1" | u32 BE length | JSON header | content`.
/// See server/docs/PROTOCOL.md §2. All metadata lives here, i.e. inside the encryption.
public enum Envelope {
    public static func encode(header: [String: Any], content: Data) throws -> Data {
        let json = try JSONSerialization.data(withJSONObject: header, options: [.sortedKeys, .withoutEscapingSlashes])
        var out = Data("SRP1".utf8)
        var len = UInt32(json.count).bigEndian
        out += Data(bytes: &len, count: 4)
        out += json
        out += content
        return out
    }
}

/// Parsed `srec://enroll?u=…&k=…&i=…&n=…` link from an invitation QR code.
public struct EnrollLink: Equatable {
    public let url: String
    public let recipient: String
    public let invite: String?
    public let name: String?

    public init?(_ raw: String) {
        let s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard s.hasPrefix("srec://enroll?"), let comps = URLComponents(string: s) else { return nil }
        var p: [String: String] = [:]
        for item in comps.queryItems ?? [] { p[item.name] = item.value }
        guard var u = p["u"], let k = p["k"] else { return nil }
        while u.hasSuffix("/") { u.removeLast() }
        url = u
        recipient = k
        invite = (p["i"]?.isEmpty ?? true) ? nil : p["i"]
        name = p["n"]
    }
}

public enum Ids {
    /// 32 lowercase hex chars (128 random bits).
    public static func random() -> String {
        var g = SystemRandomNumberGenerator()
        return (0..<16).map { _ in String(format: "%02x", UInt8.random(in: 0...255, using: &g)) }.joined()
    }
}
