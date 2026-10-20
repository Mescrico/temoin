import AVFoundation
import CoreLocation
import ImageIO
import UniformTypeIdentifiers
import TemoinCore
import UIKit

enum CaptureMode: String, CaseIterable { case photo, video, live }


/// One zoom stop of the lens selector ("0,5×", "1×", "2×"), as a zoom factor of the device.
struct Lens: Equatable, Hashable {
    let label: String
    let factor: CGFloat
}

struct CameraOption: Identifiable, Equatable {
    let id: String
    let name: String
    let position: AVCaptureDevice.Position
    let external: Bool
}

/// Owns the AVCaptureSession, the segment writer and the sealing pipeline.
///
/// iOS does not let third-party apps use the camera in the background, so unlike
/// Android a recording stops when the app leaves the screen. "Live" mode therefore
/// relies on the dark-screen mode (screen on, black, auto-lock disabled).
@MainActor
final class CaptureEngine: NSObject, ObservableObject {
    @Published var mode: CaptureMode = .photo
    @Published private(set) var recording = false
    @Published private(set) var recordingSince: Date?
    @Published private(set) var sealedCount = 0
    @Published private(set) var cameras: [CameraOption] = []
    @Published private(set) var currentCamera: CameraOption?
    @Published var error: String?
    @Published private(set) var flash = 0
    @Published private(set) var busy = false
    @Published private(set) var lenses: [Lens] = []
    @Published private(set) var lens: Lens?
    /// Current zoom factor of the device, and its pinch limits (zoom of the wide lens = "1×").
    @Published private(set) var zoomFactor: CGFloat = 1
    @Published private(set) var wideFactor: CGFloat = 1
    @Published private(set) var minZoom: CGFloat = 1
    @Published private(set) var maxZoom: CGFloat = 1
    /// One light toggle, off by default (light gives the filmer away). Back camera: flash
    /// on photos, torch in video modes. Front camera: the screen lights the face (view side).
    @Published private(set) var flashOn = false
    @Published private(set) var hasFlash = false
    var isFront: Bool { currentCamera?.position == .front }

    let session = AVCaptureSession()
    private let sessionQueue = DispatchQueue(label: "sr.session")
    private let dataQueue = DispatchQueue(label: "sr.data")
    private let sealQueue = DispatchQueue(label: "sr.seal")
    private let photoOutput = AVCapturePhotoOutput()
    private let videoOutput = AVCaptureVideoDataOutput()
    private let audioOutput = AVCaptureAudioDataOutput()
    private var videoInput: AVCaptureDeviceInput?
    private var configured = false

    // touched on dataQueue only
    nonisolated(unsafe) private var writer: SegmentWriter?
    nonisolated(unsafe) private var writerSession: String?
    nonisolated(unsafe) private var writerHeader: [String: Any] = [:]

    private weak var model: AppModel?
    private var photoDelegates: [Int64: PhotoDelegate] = [:]
    private let location = CLLocationManager()

    init(model: AppModel) {
        self.model = model
        super.init()
        NotificationCenter.default.addObserver(forName: AVCaptureSession.wasInterruptedNotification, object: session, queue: .main) { [weak self] _ in
            Task { @MainActor in if self?.recording == true { self?.stopRecording() } }
        }
        NotificationCenter.default.addObserver(forName: AVCaptureSession.runtimeErrorNotification, object: session, queue: .main) { [weak self] n in
            Task { @MainActor in
                self?.error = (n.userInfo?[AVCaptureSessionErrorKey] as? Error)?.localizedDescription
                if self?.recording == true { self?.stopRecording() }
            }
        }
        UIDevice.current.beginGeneratingDeviceOrientationNotifications()
    }

    // MARK: lifecycle

    func start() {
        refreshCameras()
        if model?.settings.user.attachLocation == true {
            location.requestWhenInUseAuthorization()
            location.startUpdatingLocation()
        }
        let preset = presetFor(mode)
        let cam = currentCamera
        sessionQueue.async { [self] in
            if !configured { configure(camera: cam, preset: preset) }
            if !session.isRunning {
                session.startRunning()
                // Starting the session can reset the zoom to the ultra-wide lens: force 1×.
                if let d = videoInput?.device { applyLenses(d) }
            }
        }
    }

    func stop() {
        if recording { stopRecording() }
        location.stopUpdatingLocation()
        sessionQueue.async { [self] in if session.isRunning { session.stopRunning() } }
    }

    func setMode(_ m: CaptureMode) {
        guard !recording, m != mode else { return }
        lightOff()
        mode = m
        let preset = presetFor(m)
        sessionQueue.async { [self] in
            session.beginConfiguration()
            if session.canSetSessionPreset(preset) { session.sessionPreset = preset }
            session.commitConfiguration()
            // A new format resets the zoom (to the ultra-wide lens on multi-cameras): force 1×.
            if let d = videoInput?.device { applyLenses(d) }
        }
    }

    func selectCamera(_ id: String) {
        guard !recording, let opt = cameras.first(where: { $0.id == id }) else { return }
        lightOff()
        model?.settings.updateUser { $0.cameraID = id }
        currentCamera = opt
        sessionQueue.async { [self] in
            guard let device = AVCaptureDevice(uniqueID: id), let input = try? AVCaptureDeviceInput(device: device) else { return }
            session.beginConfiguration()
            if let old = videoInput { session.removeInput(old) }
            if session.canAddInput(input) { session.addInput(input); videoInput = input }
            session.commitConfiguration()
            applyLenses(device)
        }
    }

    /// Switches lens / zoom like the system camera. Works while recording: on a
    /// multi-camera device iOS hands over between physical lenses seamlessly.
    func setLens(_ l: Lens) {
        lens = l
        zoomFactor = l.factor
        sessionQueue.async { [self] in
            guard let device = videoInput?.device, (try? device.lockForConfiguration()) != nil else { return }
            device.ramp(toVideoZoomFactor: min(max(l.factor, device.minAvailableVideoZoomFactor), device.maxAvailableVideoZoomFactor), withRate: 12)
            device.unlockForConfiguration()
        }
    }

    /// Lightning button (also while recording). Back camera in video modes drives the
    /// torch; photos fire the flash at capture; the front camera uses the screen (UI side).
    func toggleFlash() {
        flashOn.toggle()
        if !isFront && mode != .photo { setTorch(flashOn) }
    }

    private func lightOff() {
        if flashOn && !isFront && mode != .photo { setTorch(false) }
        flashOn = false
    }

    private func setTorch(_ on: Bool) {
        sessionQueue.async { [self] in
            guard let device = videoInput?.device, device.hasTorch, device.isTorchModeSupported(on ? .on : .off),
                  (try? device.lockForConfiguration()) != nil else { return }
            device.torchMode = on ? .on : .off
            device.unlockForConfiguration()
        }
    }

    /// Continuous zoom (pinch). Works while recording.
    func setZoom(_ factor: CGFloat) {
        let f = min(max(factor, minZoom), maxZoom)
        zoomFactor = f
        lens = lenses.last { $0.factor <= f + 0.01 } ?? lenses.first
        sessionQueue.async { [self] in
            guard let device = videoInput?.device, (try? device.lockForConfiguration()) != nil else { return }
            device.videoZoomFactor = min(max(f, device.minAvailableVideoZoomFactor), device.maxAvailableVideoZoomFactor)
            device.unlockForConfiguration()
        }
    }

    /// "0,5×", "1×", "3,4×": a zoom factor relative to the wide lens.
    func zoomLabel(_ factor: CGFloat) -> String {
        let f = NumberFormatter()
        f.maximumFractionDigits = 1
        f.minimumFractionDigits = 0
        return (f.string(from: NSNumber(value: Double(factor / max(wideFactor, 0.01)))) ?? "1") + "×"
    }

    // runs on sessionQueue: computes the stops of `device` and selects 1×
    nonisolated private func applyLenses(_ device: AVCaptureDevice) {
        let (stops, wide) = Self.lensStops(for: device)
        let lo = device.minAvailableVideoZoomFactor
        let hi = min(device.maxAvailableVideoZoomFactor, wide * 10)
        if (try? device.lockForConfiguration()) != nil {
            device.videoZoomFactor = min(max(wide, lo), device.maxAvailableVideoZoomFactor)
            device.unlockForConfiguration()
        }
        let flash = device.hasFlash || device.hasTorch
        DispatchQueue.main.async {
            self.hasFlash = flash
            self.lenses = stops
            self.lens = stops.first { $0.factor == wide }
            self.wideFactor = wide
            self.zoomFactor = wide
            self.minZoom = lo
            self.maxZoom = max(hi, lo)
        }
    }

    /// Button stops: 0,5× when the camera has an ultra-wide lens, 1× (wide lens) and 2×.
    /// Anything else is reached by pinching. Labels are relative to the wide lens.
    nonisolated static func lensStops(for d: AVCaptureDevice) -> ([Lens], CGFloat) {
        var wide: CGFloat = 1
        if d.isVirtualDevice, d.constituentDevices.first?.deviceType == .builtInUltraWideCamera,
           let first = d.virtualDeviceSwitchOverVideoZoomFactors.first {
            wide = CGFloat(truncating: first)
        }
        var factors: [CGFloat] = wide > 1 ? [1, wide] : [wide]
        if d.maxAvailableVideoZoomFactor >= wide * 2 { factors.append(wide * 2) }
        let f = NumberFormatter()
        f.maximumFractionDigits = 1
        f.minimumFractionDigits = 0
        let stops = factors.sorted().map { Lens(label: (f.string(from: NSNumber(value: Double($0 / wide))) ?? "1") + "×", factor: $0) }
        return (stops, wide)
    }

    func settingsChanged() {
        guard !recording, mode != .photo else { return }
        setPresetNow()
    }

    private func setPresetNow() {
        let preset = presetFor(mode)
        sessionQueue.async { [self] in
            session.beginConfiguration()
            if session.canSetSessionPreset(preset) { session.sessionPreset = preset }
            session.commitConfiguration()
            if let d = videoInput?.device { applyLenses(d) }
        }
    }

    func refreshCameras() {
        // Back: the virtual multi-camera when available (lens selector), front: wide, plus USB cameras.
        let backTypes: [AVCaptureDevice.DeviceType] = [.builtInTripleCamera, .builtInDualWideCamera, .builtInDualCamera, .builtInWideAngleCamera]
        let back = backTypes.lazy.compactMap { AVCaptureDevice.default($0, for: .video, position: .back) }.first
        let front = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front)
        var externals: [AVCaptureDevice] = []
        if #available(iOS 17.0, *) {
            externals = AVCaptureDevice.DiscoverySession(deviceTypes: [.external], mediaType: .video, position: .unspecified).devices
        }
        let devices = [back, front].compactMap { $0 } + externals
        cameras = devices.map { d in
            var external = false
            if #available(iOS 17.0, *) { external = d.deviceType == .external }
            return CameraOption(id: d.uniqueID, name: d.localizedName, position: d.position, external: external)
        }
        let wanted = model?.settings.user.cameraID
        currentCamera = cameras.first { $0.id == wanted } ?? cameras.first { $0.position == .back } ?? cameras.first
    }

    // runs on sessionQueue
    nonisolated private func configure(camera: CameraOption?, preset: AVCaptureSession.Preset) {
        session.beginConfiguration()
        defer { session.commitConfiguration() }
        if session.canSetSessionPreset(preset) { session.sessionPreset = preset }
        if let cam = camera, let device = AVCaptureDevice(uniqueID: cam.id), let input = try? AVCaptureDeviceInput(device: device), session.canAddInput(input) {
            session.addInput(input)
            DispatchQueue.main.async { self.videoInput = input }
            applyLenses(device)
        }
        if AVCaptureDevice.authorizationStatus(for: .audio) == .authorized,
           let mic = AVCaptureDevice.default(for: .audio), let input = try? AVCaptureDeviceInput(device: mic), session.canAddInput(input) {
            session.addInput(input)
        }
        if session.canAddOutput(photoOutput) { session.addOutput(photoOutput) }
        videoOutput.alwaysDiscardsLateVideoFrames = false
        videoOutput.setSampleBufferDelegate(self, queue: dataQueue)
        if session.canAddOutput(videoOutput) { session.addOutput(videoOutput) }
        audioOutput.setSampleBufferDelegate(self, queue: dataQueue)
        if session.canAddOutput(audioOutput) { session.addOutput(audioOutput) }
        DispatchQueue.main.async { self.configured = true }
    }

    private func presetFor(_ m: CaptureMode) -> AVCaptureSession.Preset {
        if m == .photo { return .photo }
        let limits = model?.settings.server?.limits ?? ServerLimits()
        let h = min(model?.settings.user.videoHeight ?? 720, limits.maxVideoHeight)
        return h > 720 ? .hd1920x1080 : .hd1280x720
    }

    // MARK: photo

    func takePhoto() {
        guard let model, let server = model.settings.server, !busy else { return }
        busy = true
        let orientation = Self.currentOrientation()
        let maxPixels = server.limits.photoMaxPixels
        let id = Ids.random()
        let header = baseHeader(session: id, seq: 0, kind: "photo", container: "jpeg", final: true)
        let delegate = PhotoDelegate { [weak self] data in
            Task { @MainActor in
                guard let self else { return }
                self.busy = false
                guard let data else { self.error = "photo failed"; return }
                self.flash += 1
                // The sensor only offers 12 MP on many iPhones: downscale in memory to the server's limit.
                DispatchQueue.global(qos: .userInitiated).async {
                    let small = CaptureEngine.downscaleJPEG(data, maxPixels: maxPixels)
                    Task { @MainActor in self.seal(recipient: server.recipient, session: id, seq: 0, header: header, content: small) }
                }
            }
        }
        let settings = AVCapturePhotoSettings(format: [AVVideoCodecKey: AVVideoCodecType.jpeg])
        let wantedFlash: AVCaptureDevice.FlashMode = flashOn && !isFront ? .on : .off
        if photoOutput.supportedFlashModes.contains(wantedFlash) { settings.flashMode = wantedFlash }
        sessionQueue.async { [self] in
            if let conn = photoOutput.connection(with: .video), conn.isVideoOrientationSupported { conn.videoOrientation = orientation }
            if let device = videoInput?.device {
                let dims = device.activeFormat.supportedMaxPhotoDimensions
                let fit = dims.filter { Int($0.width) * Int($0.height) <= maxPixels }.max { $0.width * $0.height < $1.width * $1.height }
                    ?? dims.min { $0.width * $0.height < $1.width * $1.height }
                if let fit {
                    photoOutput.maxPhotoDimensions = fit
                    settings.maxPhotoDimensions = fit
                }
            }
            DispatchQueue.main.async { self.photoDelegates[settings.uniqueID] = delegate }
            photoOutput.capturePhoto(with: settings, delegate: delegate)
        }
        delegate.onDone = { [weak self] in Task { @MainActor in self?.photoDelegates[settings.uniqueID] = nil } }
    }

    // MARK: recording

    func startRecording() {
        guard !recording, let model, let server = model.settings.server else { return }
        let live = mode == .live
        let limits = server.limits
        let height = min(model.settings.user.videoHeight, limits.maxVideoHeight)
        let bitrate = height > 720 ? limits.videoBitrate1080 : limits.videoBitrate720
        let orientation = Self.currentOrientation()
        let id = Ids.random()
        let recipient = server.recipient
        let hasAudio = AVCaptureDevice.authorizationStatus(for: .audio) == .authorized
        var header = baseHeader(session: id, seq: 0, kind: "video", container: "fmp4-media", final: false)
        header["video"] = ["codec": "avc", "height": height, "fps": 30, "bitrate": bitrate, "live": live, "rotation": 0]

        recording = true
        recordingSince = Date()
        sealedCount = 0
        UIApplication.shared.isIdleTimerDisabled = true

        sessionQueue.async { [self] in
            // The microphone may have been allowed after the session was configured.
            if hasAudio && !session.inputs.contains(where: { ($0 as? AVCaptureDeviceInput)?.device.hasMediaType(.audio) == true }),
               let mic = AVCaptureDevice.default(for: .audio), let input = try? AVCaptureDeviceInput(device: mic) {
                session.beginConfiguration()
                if session.canAddInput(input) { session.addInput(input) }
                session.commitConfiguration()
            }
            if let conn = videoOutput.connection(with: .video) {
                if conn.isVideoOrientationSupported { conn.videoOrientation = orientation }
                if conn.isVideoMirroringSupported { conn.isVideoMirrored = videoInput?.device.position == .front }
            }
            dataQueue.async { [self] in
                writerSession = id
                writerHeader = header
                writer = SegmentWriter(bitrate: bitrate, segmentSeconds: live ? 2 : 4, withAudio: hasAudio) { [weak self] seg in
                    guard let self else { return }
                    var h = header
                    h["seq"] = seg.seq
                    h["container"] = seg.isInit ? "fmp4-init" : "fmp4-media"
                    h["created_at"] = ISO8601DateFormatter().string(from: Date())
                    if let s = seg.startMs { h["start_ms"] = s }
                    if let d = seg.durationMs { h["duration_ms"] = d }
                    Task { @MainActor in
                        h["location"] = self.locationHeader()
                        self.seal(recipient: recipient, session: id, seq: seg.seq, header: h, content: seg.data)
                    }
                }
            }
        }
    }

    func stopRecording() {
        guard recording else { return }
        recording = false
        recordingSince = nil
        UIApplication.shared.isIdleTimerDisabled = false
        let recipient = model?.settings.server?.recipient
        dataQueue.async { [self] in
            guard let w = writer, let id = writerSession else { return }
            var header = writerHeader
            writer = nil
            writerSession = nil
            w.finish {
                // Explicit end marker: an empty final object (concatenation is unaffected).
                header["seq"] = w.nextSeq
                header["final"] = true
                header["created_at"] = ISO8601DateFormatter().string(from: Date())
                Task { @MainActor in
                    if let recipient { self.seal(recipient: recipient, session: id, seq: w.nextSeq, header: header, content: Data()) }
                }
            }
        }
    }

    // MARK: sealing

    /// Seal + enqueue. Plaintext only exists in RAM; the outbox only ever receives ciphertext.
    private func seal(recipient: String, session: String, seq: Int, header: [String: Any], content: Data) {
        guard let model else { return }
        let outbox = model.outbox
        sealQueue.async {
            do {
                let key = try Recipient.parse(recipient)
                let sealed = try AgeSealer.seal(recipient: key, plaintext: try Envelope.encode(header: header, content: content))
                try outbox.put(session: session, seq: seq, sealed: sealed)
                Task { @MainActor in
                    self.sealedCount += 1
                    model.uploader.kick()
                }
            } catch {
                Task { @MainActor in self.error = "seal: \(error.localizedDescription)" }
            }
        }
    }

    private func baseHeader(session: String, seq: Int, kind: String, container: String, final: Bool) -> [String: Any] {
        var h: [String: Any] = [
            "v": 1, "session": session, "seq": seq, "kind": kind, "container": container, "final": final,
            "created_at": ISO8601DateFormatter().string(from: Date()),
            "device": [
                "platform": "ios",
                "os": UIDevice.current.systemVersion,
                "model": Self.modelIdentifier(),
                "app": Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "?",
                "camera": currentCamera.map { $0.external ? "external" : ($0.position == .front ? "front" : "back") } ?? "?",
            ] as [String: Any],
        ]
        if let loc = locationHeader() { h["location"] = loc }
        return h
    }

    private func locationHeader() -> [String: Any]? {
        guard model?.settings.user.attachLocation == true, let l = location.location else { return nil }
        return [
            "lat": l.coordinate.latitude, "lon": l.coordinate.longitude,
            "acc_m": l.horizontalAccuracy, "alt_m": l.altitude,
            "at": ISO8601DateFormatter().string(from: l.timestamp),
        ]
    }

    static func currentOrientation() -> AVCaptureVideoOrientation {
        switch UIDevice.current.orientation {
        case .landscapeLeft: return .landscapeRight
        case .landscapeRight: return .landscapeLeft
        case .portraitUpsideDown: return .portraitUpsideDown
        default: return .portrait
        }
    }

    static func modelIdentifier() -> String {
        var info = utsname()
        uname(&info)
        return withUnsafeBytes(of: &info.machine) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
    }
}

extension CaptureEngine: AVCaptureVideoDataOutputSampleBufferDelegate, AVCaptureAudioDataOutputSampleBufferDelegate {
    nonisolated func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard let w = writer else { return }
        if output is AVCaptureVideoDataOutput { w.appendVideo(sampleBuffer) } else { w.appendAudio(sampleBuffer) }
        if let err = w.failed {
            writer = nil
            Task { @MainActor in
                self.error = err.localizedDescription
                self.recording = false
                self.recordingSince = nil
            }
        }
    }
}

extension CaptureEngine {
    /// Re-encodes a JPEG at most `maxPixels` large, orientation applied, entirely in memory.
    nonisolated static func downscaleJPEG(_ data: Data, maxPixels: Int) -> Data {
        guard let src = CGImageSourceCreateWithData(data as CFData, nil),
              let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any],
              let w = props[kCGImagePropertyPixelWidth] as? Int, let h = props[kCGImagePropertyPixelHeight] as? Int,
              w * h > maxPixels, maxPixels > 0
        else { return data }
        let scale = (Double(maxPixels) / Double(w * h)).squareRoot()
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: Int(Double(max(w, h)) * scale),
        ]
        guard let img = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else { return data }
        let out = NSMutableData()
        guard let dst = CGImageDestinationCreateWithData(out, UTType.jpeg.identifier as CFString, 1, nil) else { return data }
        CGImageDestinationAddImage(dst, img, [kCGImageDestinationLossyCompressionQuality: 0.88] as CFDictionary)
        return CGImageDestinationFinalize(dst) ? out as Data : data
    }
}

/// Keeps the JPEG in memory (`fileDataRepresentation`), never in the photo library.
final class PhotoDelegate: NSObject, AVCapturePhotoCaptureDelegate {
    private let completion: (Data?) -> Void
    var onDone: (() -> Void)?

    init(_ completion: @escaping (Data?) -> Void) { self.completion = completion }

    func photoOutput(_ output: AVCapturePhotoOutput, didFinishProcessingPhoto photo: AVCapturePhoto, error: Error?) {
        completion(error == nil ? photo.fileDataRepresentation() : nil)
        onDone?()
    }
}
