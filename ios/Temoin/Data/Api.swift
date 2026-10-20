import Foundation

import TemoinCore

struct ServerInfo {
    let name: String
    let recipient: String
    let accepting: Bool
    let limits: ServerLimits
}

struct DeviceStatus {
    let name: String
    let quotaBytes: Int64
    let usedBytes: Int64
    let objects: Int
    let lastUploadAt: String?
    var request: HandBack? = nil
}

/// "Get my files back": `code` is passed to the operator by any channel; `link` is the state
/// of the one-time link they create (nil = not yet, ready, opened, done, expired, revoked).
struct HandBack {
    let code: String
    let expiresAt: String?
    let link: String?

    /// The link is gone: asking again gives a new code.
    var closed: Bool { ["done", "expired", "revoked"].contains(link ?? "") }

    init?(_ j: [String: Any]?) {
        guard let j, let code = j["code"] as? String, !code.isEmpty else { return nil }
        self.code = code
        expiresAt = j["expires_at"] as? String
        link = j["link"] as? String
    }
}

enum RegisterResult {
    case ok(name: String, quotaBytes: Int64)
    /// `error` = server error code: unknown_invite, invite_expired, invite_revoked, invite_exhausted, banned, network…
    case failed(error: String, message: String)
}

enum ApiError: LocalizedError {
    case notServer(String)
    case refused(status: Int, error: String)
    var errorDescription: String? {
        switch self {
        case .notServer(let m): return m
        case .refused(let s, let e): return "\(s) \(e)"
        }
    }
}

enum Api {
    static func normalize(_ input: String) throws -> String {
        var u = input.trimmingCharacters(in: .whitespacesAndNewlines)
        while u.hasSuffix("/") { u.removeLast() }
        if !u.hasPrefix("http://") && !u.hasPrefix("https://") { u = "https://" + u }
        guard let url = URL(string: u), url.host != nil else { throw ApiError.notServer("invalid URL") }
        return u
    }

    /// Server description. A discreet server only answers a phone that shows an
    /// invitation code or signs with its registered device key.
    static func info(_ base: String, invite: String? = nil, key: DeviceKey? = nil) async throws -> ServerInfo {
        guard let url = URL(string: base + "/api/v1/info") else { throw ApiError.notServer("invalid URL") }
        var req = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 20)
        req.setValue("Temoin-iOS", forHTTPHeaderField: "User-Agent")
        if let invite { req.setValue(DeviceKey.normalizeInvite(invite), forHTTPHeaderField: "X-SR-Invite") }
        if let key { sign(&req, key: key, method: "GET", path: "/api/v1/info", bodySHA256: nil) }
        let (data, resp) = try await URLSession.shared.data(for: req)
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        if status == 404 { throw ApiError.refused(status: 404, error: "no_answer") }
        guard status == 200,
              let j = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              j["server"] as? String == "temoin",
              let recipient = j["recipient"] as? String
        else { throw ApiError.notServer("not a Témoin server") }
        let l = j["limits"] as? [String: Any] ?? [:]
        let d = ServerLimits()
        return ServerInfo(
            name: j["name"] as? String ?? base,
            recipient: recipient,
            accepting: j["accepting"] as? Bool ?? true,
            limits: ServerLimits(
                maxObjectBytes: l["max_object_bytes"] as? Int ?? d.maxObjectBytes,
                maxVideoHeight: l["max_video_height"] as? Int ?? d.maxVideoHeight,
                defaultVideoHeight: l["default_video_height"] as? Int ?? d.defaultVideoHeight,
                videoBitrate720: l["video_bitrate_720"] as? Int ?? d.videoBitrate720,
                videoBitrate1080: l["video_bitrate_1080"] as? Int ?? d.videoBitrate1080,
                photoMaxPixels: l["photo_max_pixels"] as? Int ?? d.photoMaxPixels
            )
        )
    }

    /// Registers this phone's public key with an invitation code.
    static func register(_ base: String, key: DeviceKey, invite: String) async -> RegisterResult {
        let code = DeviceKey.normalizeInvite(invite)
        let ts = Int64(Date().timeIntervalSince1970)
        guard let url = URL(string: base + "/api/v1/devices"),
              let sig = try? key.sign(DeviceKey.registerMessage(invite: code, publicKeyB64: key.publicKeyB64, timestamp: ts)),
              let body = try? JSONSerialization.data(withJSONObject: ["invite": code, "public_key": key.publicKeyB64, "timestamp": ts, "signature": sig] as [String: Any])
        else { return .failed(error: "bad_request", message: "cannot build request") }
        var req = URLRequest(url: url, timeoutInterval: 20)
        req.httpMethod = "POST"
        req.httpBody = body
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.setValue("Temoin-iOS", forHTTPHeaderField: "User-Agent")
        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
            let j = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
            if status == 200 {
                return .ok(name: j["name"] as? String ?? "", quotaBytes: (j["quota_bytes"] as? NSNumber)?.int64Value ?? 0)
            }
            return .failed(error: j["error"] as? String ?? "HTTP \(status)", message: j["message"] as? String ?? "HTTP \(status)")
        } catch {
            return .failed(error: "network", message: error.localizedDescription)
        }
    }

    /// Adds the device signature headers (PROTOCOL.md section 4).
    static func sign(_ req: inout URLRequest, key: DeviceKey, method: String, path: String, bodySHA256: String?) {
        let ts = Int64(Date().timeIntervalSince1970)
        let msg = DeviceKey.requestMessage(method: method, path: path, bodySHA256: bodySHA256, timestamp: ts, deviceID: key.deviceID)
        req.setValue(key.deviceID, forHTTPHeaderField: "X-SR-Device")
        req.setValue(String(ts), forHTTPHeaderField: "X-SR-Timestamp")
        req.setValue((try? key.sign(msg)) ?? "", forHTTPHeaderField: "X-SR-Signature")
    }

    /// Usage counters of this device (never the list of its files).
    static func me(_ base: String, key: DeviceKey) async throws -> DeviceStatus {
        let path = "/api/v1/me"
        guard let url = URL(string: base + path) else { throw ApiError.notServer("invalid URL") }
        var req = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 20)
        sign(&req, key: key, method: "GET", path: path, bodySHA256: nil)
        let (data, resp) = try await URLSession.shared.data(for: req)
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        let j = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        guard status == 200 else { throw ApiError.refused(status: status, error: j["error"] as? String ?? "HTTP \(status)") }
        return DeviceStatus(
            name: j["name"] as? String ?? "",
            quotaBytes: (j["quota_bytes"] as? NSNumber)?.int64Value ?? 0,
            usedBytes: (j["used_bytes"] as? NSNumber)?.int64Value ?? 0,
            objects: (j["objects"] as? NSNumber)?.intValue ?? 0,
            lastUploadAt: j["last_upload_at"] as? String,
            request: HandBack(j["request"] as? [String: Any])
        )
    }

    /// Asks for this device's recordings back: returns the code to pass to the operator.
    static func requestFiles(_ base: String, key: DeviceKey) async throws -> HandBack {
        let path = "/api/v1/me/request"
        guard let url = URL(string: base + path) else { throw ApiError.notServer("invalid URL") }
        var req = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 20)
        req.httpMethod = "POST"
        req.setValue("Temoin-iOS", forHTTPHeaderField: "User-Agent")
        sign(&req, key: key, method: "POST", path: path, bodySHA256: nil)
        let (data, resp) = try await URLSession.shared.data(for: req)
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        let j = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        guard status == 200, let hb = HandBack(j) else {
            throw ApiError.refused(status: status, error: j["error"] as? String ?? "HTTP \(status)")
        }
        return hb
    }
}
