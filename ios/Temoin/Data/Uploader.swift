import CryptoKit
import Foundation
import TemoinCore

/// Drains the outbox through a *background* URLSession: iOS keeps uploading even
/// if the app is suspended or killed. The server is idempotent, so a repeated
/// upload of the same object is harmless.
final class Uploader: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    enum State: Equatable { case idle, uploading, waitingNetwork, pausedByServer, banned, notRegistered, error(String) }

    static let sessionID = "io.github.temoin-app.temoin.upload"

    private let outbox: Outbox
    private let lock = NSLock()
    private var bodies: [Int: Data] = [:]
    private var retryScheduled = false
    var config: () -> (server: ServerConfig?, key: DeviceKey?, wifiOnly: Bool) = { (nil, nil, false) }
    var onState: ((State) -> Void)?
    var backgroundCompletion: (() -> Void)?

    private lazy var session: URLSession = {
        let c = URLSessionConfiguration.background(withIdentifier: Self.sessionID)
        c.isDiscretionary = false
        c.sessionSendsLaunchEvents = true
        c.timeoutIntervalForRequest = 60
        c.httpMaximumConnectionsPerHost = 2
        return URLSession(configuration: c, delegate: self, delegateQueue: nil)
    }()

    init(outbox: Outbox) {
        self.outbox = outbox
        super.init()
    }

    /// Schedules every outbox file that is not already in flight.
    func kick() {
        let (server, key, wifiOnly) = config()
        guard let server, let key else { return }
        session.getAllTasks { tasks in
            let inFlight = Set(tasks.compactMap { $0.taskDescription?.components(separatedBy: "|").first })
            let items = self.outbox.list().filter { !inFlight.contains($0.name) }
            if items.isEmpty && tasks.isEmpty { self.onState?(.idle); return }
            for item in items {
                guard let body = try? Data(contentsOf: item.url) else { continue }
                let sha = SHA256.hash(data: body).map { String(format: "%02x", $0) }.joined()
                let path = "/api/v1/objects/\(item.session)/\(item.seq)"
                guard let url = URL(string: server.url + path) else { continue }
                var req = URLRequest(url: url)
                req.httpMethod = "PUT"
                req.allowsCellularAccess = !wifiOnly
                req.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
                req.setValue(sha, forHTTPHeaderField: "X-SR-Content-SHA256")
                req.setValue("Temoin-iOS", forHTTPHeaderField: "User-Agent")
                Api.sign(&req, key: key, method: "PUT", path: path, bodySHA256: sha)
                let task = self.session.uploadTask(with: req, fromFile: item.url)
                task.taskDescription = "\(item.name)|\(sha)"
                task.resume()
            }
            self.onState?(.uploading)
        }
    }

    // MARK: URLSessionDataDelegate

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock(); bodies[dataTask.taskIdentifier, default: Data()].append(data); lock.unlock()
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.lock(); let body = bodies.removeValue(forKey: task.taskIdentifier); lock.unlock()
        let parts = task.taskDescription?.components(separatedBy: "|") ?? []
        guard parts.count == 2 else { return }
        let (name, sha) = (parts[0], parts[1])

        if let error {
            onState?((error as NSError).code == NSURLErrorNotConnectedToInternet ? .waitingNetwork : .error(error.localizedDescription))
            scheduleRetry()
            return
        }
        let status = (task.response as? HTTPURLResponse)?.statusCode ?? 0
        let json = body.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
        switch status {
        case 200, 201:
            // Only trust the ack if the server hashed exactly what we sent.
            if json?["sha256"] as? String == sha { outbox.remove(name) } else { scheduleRetry() }
        case 503, 401, 403:
            // Paused, banned, unknown device: keep the object (a ban may be lifted) and retry later.
            switch json?["error"] as? String {
            case "banned": onState?(.banned)
            case "unknown_device": onState?(.notRegistered)
            default: onState?(.pausedByServer)
            }
            scheduleRetry()
        case 429, 500...599:
            onState?(.error("HTTP \(status)")); scheduleRetry()
        default:
            outbox.reject(name)
        }
        session.getAllTasks { tasks in
            if tasks.isEmpty && self.outbox.list().isEmpty { self.onState?(.idle) }
        }
    }

    func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        DispatchQueue.main.async {
            self.backgroundCompletion?()
            self.backgroundCompletion = nil
        }
    }

    private func scheduleRetry() {
        lock.lock()
        let already = retryScheduled
        retryScheduled = true
        lock.unlock()
        if already { return }
        DispatchQueue.global().asyncAfter(deadline: .now() + 15) {
            self.lock.lock(); self.retryScheduled = false; self.lock.unlock()
            self.kick()
        }
    }
}
