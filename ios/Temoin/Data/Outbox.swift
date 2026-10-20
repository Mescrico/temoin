import Foundation

/// Queue of sealed objects waiting for upload. Only ciphertext is ever written here:
/// the phone cannot decrypt any of it. Files are deleted once the server acknowledges them.
final class Outbox: @unchecked Sendable {
    struct Item {
        let url: URL
        let session: String
        let seq: Int
        var name: String { url.lastPathComponent }
    }

    let dir: URL
    let rejectedDir: URL
    var onChange: (() -> Void)?
    private let fm = FileManager.default

    init() {
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        dir = base.appendingPathComponent("outbox", isDirectory: true)
        rejectedDir = base.appendingPathComponent("rejected", isDirectory: true)
        for d in [dir, rejectedDir] {
            try? fm.createDirectory(at: d, withIntermediateDirectories: true, attributes: [
                // Readable while the phone is locked, so background uploads keep going.
                .protectionKey: FileProtectionType.completeUntilFirstUserAuthentication,
            ])
            var v = URLResourceValues()
            v.isExcludedFromBackup = true
            var u = d
            try? u.setResourceValues(v)
        }
        for f in (try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)) ?? [] where f.pathExtension == "part" {
            try? fm.removeItem(at: f)
        }
    }

    func put(session: String, seq: Int, sealed: Data) throws {
        let name = String(format: "%@_%06d.age", session, seq)
        let part = dir.appendingPathComponent(name + ".part")
        try sealed.write(to: part, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        try fm.moveItem(at: part, to: dir.appendingPathComponent(name))
        onChange?()
    }

    /// Oldest first, so a recording arrives in order.
    func list() -> [Item] {
        let keys: [URLResourceKey] = [.contentModificationDateKey]
        let files = ((try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: keys)) ?? [])
            .filter { $0.pathExtension == "age" }
        let dated = files.map { ($0, (try? $0.resourceValues(forKeys: Set(keys)).contentModificationDate) ?? .distantPast) }
        return dated.sorted { $0.1 == $1.1 ? $0.0.lastPathComponent < $1.0.lastPathComponent : $0.1 < $1.1 }.compactMap { url, _ in
            let base = url.deletingPathExtension().lastPathComponent
            guard let i = base.lastIndex(of: "_"), let seq = Int(base[base.index(after: i)...]) else { return nil }
            return Item(url: url, session: String(base[..<i]), seq: seq)
        }
    }

    func remove(_ name: String) {
        try? fm.removeItem(at: dir.appendingPathComponent(name))
        onChange?()
    }

    func reject(_ name: String) {
        try? fm.moveItem(at: dir.appendingPathComponent(name), to: rejectedDir.appendingPathComponent(name))
        onChange?()
    }

    func stats() -> (count: Int, bytes: Int64) {
        let files = ((try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: [.fileSizeKey])) ?? []).filter { $0.pathExtension == "age" }
        let bytes = files.reduce(Int64(0)) { $0 + Int64((try? $1.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0) }
        return (files.count, bytes)
    }

    func rejectedCount() -> Int { ((try? fm.contentsOfDirectory(atPath: rejectedDir.path)) ?? []).count }

    func clearRejected() {
        for f in (try? fm.contentsOfDirectory(at: rejectedDir, includingPropertiesForKeys: nil)) ?? [] { try? fm.removeItem(at: f) }
        onChange?()
    }

    func clearAll() {
        for f in (try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)) ?? [] { try? fm.removeItem(at: f) }
        clearRejected()
    }
}
