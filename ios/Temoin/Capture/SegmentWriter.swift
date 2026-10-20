import AVFoundation
import UniformTypeIdentifiers

/// AVAssetWriter in "fragmented MP4 segments to memory" mode (no output URL):
/// frames are encoded and handed back as `Data` through the delegate, so the
/// plaintext video never touches the file system.
///
/// All methods must be called on the capture data queue.
final class SegmentWriter: NSObject, AVAssetWriterDelegate {
    struct Segment {
        let seq: Int
        let data: Data
        let isInit: Bool
        let startMs: Int?
        let durationMs: Int?
    }

    private let bitrate: Int
    private let segmentSeconds: Double
    private let withAudio: Bool
    private let onSegment: (Segment) -> Void

    private var writer: AVAssetWriter?
    private var videoInput: AVAssetWriterInput?
    private var audioInput: AVAssetWriterInput?
    private var startTime: CMTime = .invalid
    private var seq = 0
    private(set) var failed: Error?
    private(set) var dimensions: CMVideoDimensions?

    init(bitrate: Int, segmentSeconds: Double, withAudio: Bool, onSegment: @escaping (Segment) -> Void) {
        self.bitrate = bitrate
        self.segmentSeconds = segmentSeconds
        self.withAudio = withAudio
        self.onSegment = onSegment
    }

    func appendVideo(_ sb: CMSampleBuffer) {
        if writer == nil { start(with: sb) }
        guard let w = writer, w.status == .writing, let input = videoInput, input.isReadyForMoreMediaData,
              let rebased = rebase(sb) else { return }
        if !input.append(rebased) { failed = w.error }
    }

    func appendAudio(_ sb: CMSampleBuffer) {
        guard let w = writer, w.status == .writing, let input = audioInput, input.isReadyForMoreMediaData else { return }
        guard CMSampleBufferGetPresentationTimeStamp(sb) >= startTime, let rebased = rebase(sb) else { return }
        if !input.append(rebased) { failed = w.error }
    }

    /// Shifts a sample buffer so the recording starts at 0 (capture buffers carry the
    /// phone's uptime clock, which made players show multi-day durations).
    private func rebase(_ sb: CMSampleBuffer) -> CMSampleBuffer? {
        var count: CMItemCount = 0
        guard CMSampleBufferGetSampleTimingInfoArray(sb, entryCount: 0, arrayToFill: nil, entriesNeededOut: &count) == noErr, count > 0 else { return nil }
        var timing = [CMSampleTimingInfo](repeating: CMSampleTimingInfo(), count: count)
        guard CMSampleBufferGetSampleTimingInfoArray(sb, entryCount: count, arrayToFill: &timing, entriesNeededOut: &count) == noErr else { return nil }
        for i in 0..<count {
            timing[i].presentationTimeStamp = CMTimeSubtract(timing[i].presentationTimeStamp, startTime)
            if timing[i].decodeTimeStamp.isValid { timing[i].decodeTimeStamp = CMTimeSubtract(timing[i].decodeTimeStamp, startTime) }
        }
        var out: CMSampleBuffer?
        guard CMSampleBufferCreateCopyWithNewTiming(allocator: kCFAllocatorDefault, sampleBuffer: sb, sampleTimingEntryCount: count,
                                                    sampleTimingArray: &timing, sampleBufferOut: &out) == noErr else { return nil }
        return out
    }

    func finish(_ done: @escaping () -> Void) {
        guard let w = writer, w.status == .writing else { done(); return }
        videoInput?.markAsFinished()
        audioInput?.markAsFinished()
        w.finishWriting { done() }
    }

    private func start(with sb: CMSampleBuffer) {
        guard let fd = CMSampleBufferGetFormatDescription(sb) else { return }
        let dims = CMVideoFormatDescriptionGetDimensions(fd)
        dimensions = dims
        let w = AVAssetWriter(contentType: UTType.mpeg4Movie)
        w.outputFileTypeProfile = .mpeg4AppleHLS
        w.preferredOutputSegmentInterval = CMTime(seconds: segmentSeconds, preferredTimescale: 600)
        startTime = CMSampleBufferGetPresentationTimeStamp(sb)
        w.initialSegmentStartTime = .zero
        w.delegate = self

        let v = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: Int(dims.width),
            AVVideoHeightKey: Int(dims.height),
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: bitrate,
                AVVideoExpectedSourceFrameRateKey: 30,
                AVVideoMaxKeyFrameIntervalDurationKey: 1,
                AVVideoAllowFrameReorderingKey: false,
                AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
            ],
        ])
        v.expectsMediaDataInRealTime = true
        w.add(v)
        videoInput = v

        if withAudio {
            let a = AVAssetWriterInput(mediaType: .audio, outputSettings: [
                AVFormatIDKey: kAudioFormatMPEG4AAC,
                AVSampleRateKey: 48_000,
                AVNumberOfChannelsKey: 1,
                AVEncoderBitRateKey: 64_000,
            ])
            a.expectsMediaDataInRealTime = true
            if w.canAdd(a) { w.add(a); audioInput = a }
        }
        writer = w
        guard w.startWriting() else { failed = w.error; return }
        w.startSession(atSourceTime: .zero)
    }

    // MARK: AVAssetWriterDelegate

    func assetWriter(_ writer: AVAssetWriter, didOutputSegmentData segmentData: Data, segmentType: AVAssetSegmentType, segmentReport: AVAssetSegmentReport?) {
        var startMs: Int?
        var durMs: Int?
        if let track = segmentReport?.trackReports.first(where: { $0.mediaType == .video }) {
            startMs = Int(CMTimeGetSeconds(track.earliestPresentationTimeStamp) * 1000)
            durMs = Int(CMTimeGetSeconds(track.duration) * 1000)
        }
        onSegment(Segment(seq: seq, data: segmentData, isInit: segmentType == .initialization, startMs: startMs, durationMs: durMs))
        seq += 1
    }

    var nextSeq: Int { seq }
}
