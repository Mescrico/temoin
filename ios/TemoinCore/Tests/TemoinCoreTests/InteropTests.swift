import Foundation
import XCTest
@testable import TemoinCore

/// Writes objects sealed to the committed TEST recipient into $SR_INTEROP_DIR
/// (default: ./build/interop). `tools/interop-check.sh` decrypts them with the
/// reference Go `age` and the server's `sr decrypt`.
final class InteropTests: XCTestCase {
    let testRecipient = "age1vy0mm5yzn08fjcur7l7a58evfd5rptw5035c67ur28uz0p4yxpts788dz0"

    func testRecipientParsing() throws {
        XCTAssertEqual(try Recipient.parse(testRecipient).count, 32)
        XCTAssertFalse(Recipient.isValid(String(testRecipient.dropLast()) + "q"))
        XCTAssertFalse(Recipient.isValid("AGE-SECRET-KEY-1QQQ"))
        XCTAssertNotNil(Recipient.fingerprint(testRecipient).range(of: "^[0-9A-F]{4}( [0-9A-F]{4}){4}$", options: .regularExpression))
    }

    func testEnrollLink() {
        let l = EnrollLink("srec://enroll?u=https%3A%2F%2Frecord.example.org%2F&k=\(testRecipient)&i=ABCDE23456&n=Manif%20test")
        XCTAssertEqual(l?.url, "https://record.example.org")
        XCTAssertEqual(l?.invite, "ABCDE23456")
        XCTAssertEqual(l?.name, "Manif test")
        XCTAssertNil(EnrollLink("https://example.org"))
    }

    func testWritesInteropVectors() throws {
        let dir = URL(fileURLWithPath: ProcessInfo.processInfo.environment["SR_INTEROP_DIR"] ?? "build/interop")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let key = try Recipient.parse(testRecipient)
        for (i, size) in [1, 1000, 65536, 65537, 3 * 65536, 300_001].enumerated() {
            let content = Data((0..<size).map { UInt8(($0 * 31) % 251) })
            let header: [String: Any] = [
                "v": 1, "session": "interop_ios_\(i)", "seq": 0, "kind": "photo", "container": "jpeg",
                "final": true, "created_at": "2026-10-19T12:00:00Z",
                "device": ["platform": "ios-test", "model": "linux \"quoted\""],
            ]
            let sealed = try AgeSealer.seal(recipient: key, plaintext: try Envelope.encode(header: header, content: content))
            XCTAssertEqual(String(data: sealed.prefix(22), encoding: .utf8), "age-encryption.org/v1\n")
            try sealed.write(to: dir.appendingPathComponent("interop_ios_\(i).age"))
            try content.write(to: dir.appendingPathComponent("interop_ios_\(i).bin"))
        }
    }

    func testWritesDeviceSignatureVectors() throws {
        let dir = URL(fileURLWithPath: ProcessInfo.processInfo.environment["SR_INTEROP_DIR"] ?? "build/interop")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let key = DeviceKey()
        XCTAssertEqual(key.deviceID.count, 32)
        XCTAssertEqual(try DeviceKey(rawRepresentation: key.rawRepresentation).deviceID, key.deviceID)
        XCTAssertEqual(DeviceKey.normalizeInvite("abcde fghjk-23"), "ABCDEFGHJK23")
        let msgs = [
            ("req", DeviceKey.requestMessage(method: "PUT", path: "/api/v1/objects/abc12345/0", bodySHA256: String(repeating: "ab", count: 32), timestamp: 1792412345, deviceID: key.deviceID)),
            ("me", DeviceKey.requestMessage(method: "GET", path: "/api/v1/me", bodySHA256: nil, timestamp: 1792412345, deviceID: key.deviceID)),
            ("reg", DeviceKey.registerMessage(invite: "K7QM2ABCDEFGHJKLMNPQ", publicKeyB64: key.publicKeyB64, timestamp: 1792412345)),
        ]
        for (name, msg) in msgs {
            let v: [String: String] = ["public_key": key.publicKeyB64, "device_id": key.deviceID, "message": msg, "signature": try key.sign(msg)]
            try JSONSerialization.data(withJSONObject: v).write(to: dir.appendingPathComponent("sig_ios_\(name).json"))
        }
    }
}
