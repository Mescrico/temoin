package io.github.temoin_app.temoin.capture

import android.annotation.SuppressLint
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTimestamp
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaRecorder
import android.os.SystemClock
import android.util.Log
import android.view.Surface
import java.util.concurrent.atomic.AtomicBoolean

data class VideoConfig(
    val width: Int,
    val height: Int,
    val fps: Int,
    val bitrate: Int,
    val audio: Boolean,
    val micDevice: AudioDeviceInfo?,
    val segmentMs: Long,
    /** Camera timestamps use CLOCK_BOOTTIME (true) or CLOCK_MONOTONIC (false). */
    val cameraRealtimeClock: Boolean,
)

/** One finished segment: MPEG-TS bytes, still plaintext, only in RAM. */
class Segment(val seq: Int, val data: ByteArray, val startMs: Long, val durationMs: Long, val isFinal: Boolean)

/**
 * H.264 (surface input) + AAC encoders feeding a [TsMuxer]. Segments are cut on
 * key frames once [VideoConfig.segmentMs] has elapsed and handed to [sink]
 * immediately; nothing is written to disk here.
 */
class SegmentRecorder(private val cfg: VideoConfig, private val sink: (Segment) -> Unit) {
    private val muxer = TsMuxer(cfg.audio)
    private val lock = Any()
    private lateinit var video: MediaCodec
    private var audio: MediaCodec? = null
    private var record: AudioRecord? = null
    lateinit var inputSurface: Surface
        private set

    private val running = AtomicBool(false)
    private var videoThread: Thread? = null
    private var audioThread: Thread? = null

    private var csd: ByteArray? = null
    private var basePtsUs = -1L
    private var segStartUs = 0L
    private var lastVideoUs = 0L
    private var seq = 0
    @Volatile private var videoStarted = false
    @Volatile var error: Throwable? = null
        private set

    fun prepare(): Surface {
        val fmt = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, cfg.width, cfg.height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, cfg.bitrate)
            setInteger(MediaFormat.KEY_FRAME_RATE, cfg.fps)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
            setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR)
            // Repeat the last frame if the camera stalls, so segments keep their cadence.
            setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 1_000_000L / cfg.fps * 3)
        }
        video = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        video.configure(fmt, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        inputSurface = video.createInputSurface()

        if (cfg.audio) {
            val afmt = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, SAMPLE_RATE, 1).apply {
                setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
                setInteger(MediaFormat.KEY_BIT_RATE, 64_000)
                setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 16 * 1024)
            }
            audio = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).apply {
                configure(afmt, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            }
        }
        return inputSurface
    }

    @SuppressLint("MissingPermission")
    fun start() {
        running.set(true)
        video.start()
        videoThread = Thread({ runCatching { drainVideo() }.onFailure { fail(it) } }, "sr-venc").apply { start() }
        audio?.let { enc ->
            val minBuf = AudioRecord.getMinBufferSize(SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
            val rec = AudioRecord(MediaRecorder.AudioSource.CAMCORDER, SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuf * 4, 32 * 1024))
            if (rec.state != AudioRecord.STATE_INITIALIZED) {
                Log.w(TAG, "microphone unavailable, recording without audio")
                rec.release()
                return@let
            }
            cfg.micDevice?.let { rec.preferredDevice = it }
            record = rec
            enc.start()
            rec.startRecording()
            audioThread = Thread({ runCatching { runAudio(rec, enc) }.onFailure { fail(it) } }, "sr-aenc").apply { start() }
        }
    }

    /** Stops encoders, flushes the last (final) segment and releases everything. Blocking. */
    fun stop() {
        if (!running.getAndSet(false)) return
        runCatching { video.signalEndOfInputStream() }
        audioThread?.join(3000)
        videoThread?.join(5000)
        runCatching { record?.stop() }
        record?.release()
        runCatching { audio?.stop() }
        audio?.release()
        runCatching { video.stop() }
        video.release()
        inputSurface.release()
        synchronized(lock) {
            val data = muxer.cut()
            val dur = ((lastVideoUs - segStartUs) / 1000).coerceAtLeast(0)
            sink(Segment(seq++, data, ((segStartUs - basePtsUs.coerceAtLeast(0)) / 1000), dur, isFinal = true))
        }
    }

    private fun fail(t: Throwable) {
        Log.e(TAG, "encoder failure", t)
        error = t
        running.set(false)
    }

    private fun to90k(us: Long) = (us - basePtsUs) * 9 / 100 + PTS_OFFSET_90K

    private fun drainVideo() {
        val info = MediaCodec.BufferInfo()
        var stopDeadline = 0L
        while (true) {
            val idx = video.dequeueOutputBuffer(info, 10_000)
            if (idx < 0) {
                // Some encoders never emit an EOS buffer: give up 2 s after stop() was requested.
                if (!running.get()) {
                    if (stopDeadline == 0L) stopDeadline = SystemClock.elapsedRealtime() + 2000
                    else if (SystemClock.elapsedRealtime() > stopDeadline) break
                }
                continue
            }
            val buf = video.getOutputBuffer(idx)!!
            val bytes = ByteArray(info.size)
            buf.position(info.offset); buf.get(bytes)
            val flags = info.flags
            val pts = info.presentationTimeUs
            video.releaseOutputBuffer(idx, false)

            if (flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                csd = bytes
            } else if (bytes.isNotEmpty()) {
                val key = flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                synchronized(lock) {
                    if (basePtsUs < 0) {
                        if (!key) return@synchronized // wait for the first IDR
                        basePtsUs = pts
                        segStartUs = pts
                        muxer.cut() // writes PAT/PMT into the first segment
                        videoStarted = true
                    } else if (key && pts - segStartUs >= cfg.segmentMs * 1000) {
                        val data = muxer.cut()
                        sink(Segment(seq++, data, (segStartUs - basePtsUs) / 1000, (pts - segStartUs) / 1000, isFinal = false))
                        segStartUs = pts
                    }
                    // Every key frame carries SPS/PPS so each segment decodes on its own.
                    val au = if (key && csd != null && !startsWithSps(bytes)) csd!! + bytes else bytes
                    muxer.writeVideo(au, to90k(pts), key)
                    lastVideoUs = pts
                }
            }
            if (flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) break
        }
    }

    private fun runAudio(rec: AudioRecord, enc: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        val ts = AudioTimestamp()
        val pcm = ByteArray(2048 * 2)
        var totalFrames = 0L
        var startNs = -1L
        // Camera frames are stamped with BOOTTIME or MONOTONIC; express audio on the same clock.
        val clockOffsetNs = if (cfg.cameraRealtimeClock) SystemClock.elapsedRealtimeNanos() - System.nanoTime() else 0L
        var eos = false
        while (!eos) {
            val stopping = !running.get()
            val inIdx = enc.dequeueInputBuffer(10_000)
            if (inIdx >= 0) {
                val n = if (stopping) 0 else rec.read(pcm, 0, pcm.size)
                if (startNs < 0 && n > 0) {
                    startNs = if (rec.getTimestamp(ts, AudioTimestamp.TIMEBASE_MONOTONIC) == AudioRecord.SUCCESS)
                        ts.nanoTime - ts.framePosition * 1_000_000_000L / SAMPLE_RATE
                    else System.nanoTime() - (n / 2) * 1_000_000_000L / SAMPLE_RATE
                }
                val ptsUs = (startNs + clockOffsetNs) / 1000 + totalFrames * 1_000_000L / SAMPLE_RATE
                val ib = enc.getInputBuffer(inIdx)!!
                ib.clear()
                if (n > 0) {
                    ib.put(pcm, 0, n)
                    totalFrames += n / 2
                    enc.queueInputBuffer(inIdx, 0, n, ptsUs, 0)
                } else if (stopping) {
                    enc.queueInputBuffer(inIdx, 0, 0, ptsUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                } else {
                    enc.queueInputBuffer(inIdx, 0, 0, ptsUs, 0)
                }
            }
            while (true) {
                val outIdx = enc.dequeueOutputBuffer(info, 0)
                if (outIdx < 0) break
                val ob = enc.getOutputBuffer(outIdx)!!
                if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0 && info.size > 0 && videoStarted) {
                    val frame = ByteArray(info.size)
                    ob.position(info.offset); ob.get(frame)
                    synchronized(lock) {
                        if (info.presentationTimeUs >= basePtsUs) {
                            muxer.writeAudio(TsMuxer.adtsHeader(frame.size, SAMPLE_RATE, 1) + frame, to90k(info.presentationTimeUs))
                        }
                    }
                }
                if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) eos = true
                enc.releaseOutputBuffer(outIdx, false)
            }
        }
    }

    private fun startsWithSps(b: ByteArray): Boolean {
        val off = when {
            b.size > 4 && b[0].toInt() == 0 && b[1].toInt() == 0 && b[2].toInt() == 0 && b[3].toInt() == 1 -> 4
            b.size > 3 && b[0].toInt() == 0 && b[1].toInt() == 0 && b[2].toInt() == 1 -> 3
            else -> return false
        }
        return (b[off].toInt() and 0x1F) == 7
    }

    companion object {
        private const val TAG = "SR.Recorder"
        const val SAMPLE_RATE = 48_000
        /** Start timestamps at 1 s so the PCR (slightly earlier) never goes negative. */
        private const val PTS_OFFSET_90K = 90_000L
    }
}

private typealias AtomicBool = AtomicBoolean
