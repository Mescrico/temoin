package io.github.temoin_app.temoin.capture

import android.content.Context
import android.content.Intent
import android.graphics.ImageFormat
import android.hardware.camera2.CameraManager
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.media.ImageReader
import android.os.Build
import android.os.StatFs
import android.util.Log
import android.util.Size
import android.view.OrientationEventListener
import android.view.Surface
import androidx.core.content.ContextCompat
import io.github.temoin_app.temoin.BuildConfig
import io.github.temoin_app.temoin.TemoinApp
import io.github.temoin_app.temoin.crypto.AgeSealer
import io.github.temoin_app.temoin.crypto.Envelope
import io.github.temoin_app.temoin.crypto.Recipient
import io.github.temoin_app.temoin.service.RecordingService
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.security.SecureRandom
import java.time.Instant
import java.util.concurrent.Executors

enum class Mode { PHOTO, VIDEO, LIVE }

data class EngineState(
    val mode: Mode = Mode.PHOTO,
    val cameras: List<CameraInfo> = emptyList(),
    val camera: CameraInfo? = null,
    val previewSize: Size? = null,
    val sensorOrientation: Int = 90,
    val recording: Boolean = false,
    val recordingSince: Long? = null,
    val sealedObjects: Int = 0,
    val sealedBytes: Long = 0,
    val flash: Long = 0,
    val error: String? = null,
    val busy: Boolean = false,
    /** Lens selector stops (zoom ratios), e.g. [0.6, 1, 2]; empty when the camera cannot zoom. */
    val zoomStops: List<Float> = emptyList(),
    val zoom: Float = 1f,
    /** Pinch limits. */
    val minZoom: Float = 1f,
    val maxZoom: Float = 1f,
    /**
     * One light toggle, off by default (light gives the filmer away). Back camera: flash on
     * photos, torch in video modes. Front camera: the screen lights the face (UI side).
     */
    val flashOn: Boolean = false,
)

/**
 * Owns the camera, the encoders and the sealing pipeline. Lives as long as the
 * process; the activity only lends it a preview surface, and [RecordingService]
 * keeps the process in the foreground while a recording runs (screen locked,
 * app in background — the recording continues).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class CaptureEngine(private val app: TemoinApp) {
    private val camera = CameraController(app)
    private val serial = Dispatchers.Default.limitedParallelism(1)
    private val scope = CoroutineScope(SupervisorJob() + serial)
    private val sealer = Executors.newSingleThreadExecutor { Thread(it, "sr-seal") }
    private val rng = SecureRandom()
    /** Serializes camera open/close/configure across suspension points (the serial dispatcher alone does not). */
    private val camLock = Mutex()

    private val _state = MutableStateFlow(EngineState())
    val state: StateFlow<EngineState> = _state.asStateFlow()

    private var preview: Surface? = null
    private var jpeg: ImageReader? = null
    private var recorder: SegmentRecorder? = null
    private var session: String? = null
    private var recordingInfo: Map<String, Any?> = emptyMap()
    private var deviceDegrees = 0
    private val location = LocationTracker(app)
    private var availability: CameraManager.AvailabilityCallback? = null

    private val orientation = object : OrientationEventListener(app) {
        override fun onOrientationChanged(o: Int) {
            if (o != ORIENTATION_UNKNOWN) deviceDegrees = o
        }
    }

    init {
        camera.onDisconnected = { scope.launch { onCameraLost() } }
    }

    // ------------------------------------------------------------------ UI hooks

    /** Lists cameras and computes the preview size, before any surface exists. */
    fun prepare(): Job = scope.launch { refreshCameras() }

    fun attachPreview(surface: Surface): Job = scope.launch {
        preview = surface
        if (availability == null) availability = camera.registerAvailability { scope.launch { refreshCameras() } }
        orientation.enable()
        if (app.settings.user.value.attachLocation) location.start()
        refreshCameras()
        reconfigure()
        if (autoRecord) startAutoRecording()
    }

    @Volatile private var autoRecord = false

    /**
     * Quick launch (Quick Settings tile, launcher shortcut): switch to Live and start
     * recording as soon as the camera runs, without any tap.
     */
    fun requestAutoRecord(): Job = scope.launch {
        if (recorder != null || app.settings.server.value == null) return@launch
        autoRecord = true
        if (preview != null) startAutoRecording()
    }

    private suspend fun startAutoRecording() {
        autoRecord = false
        if (recorder != null) return
        if (_state.value.mode != Mode.LIVE) {
            _state.update { it.copy(mode = Mode.LIVE) }
            reconfigure()
        }
        startRecording().join()
    }

    fun detachPreview(): Job = scope.launch {
        preview = null
        if (recorder == null) {
            orientation.disable()
            location.stop()
            camLock.withLock {
                camera.close()
                jpeg?.close(); jpeg = null
            }
        } else {
            reconfigure() // keep recording, encoder only
        }
    }

    fun setMode(mode: Mode): Job = scope.launch {
        if (recorder != null || _state.value.mode == mode) return@launch
        lightOff()
        _state.update { it.copy(mode = mode) }
        reconfigure()
    }

    fun selectCamera(id: String): Job = scope.launch {
        if (recorder != null) return@launch
        app.settings.updateUser { it.copy(cameraId = id) }
        camera.photoFlash = 0
        camLock.withLock { camera.close() }
        _state.update { it.copy(flashOn = false) }
        refreshCameras()
        reconfigure()
    }

    /** Re-reads quality settings (resolution change => new preview size). */
    fun settingsChanged(): Job = scope.launch {
        if (recorder != null) return@launch
        refreshCameras()
        if (preview != null) reconfigure()
    }

    fun clearError() = _state.update { it.copy(error = null) }

    /** Lightning button, also while recording. */
    fun toggleFlash(): Job = scope.launch {
        val st = _state.value
        val on = !st.flashOn
        _state.update { it.copy(flashOn = on) }
        if (st.camera?.facing == Facing.FRONT) return@launch // screen light, drawn by the UI
        if (st.mode == Mode.PHOTO) camera.photoFlash = if (on) 2 else 0
        else camLock.withLock { camera.setTorch(on) }
    }

    private suspend fun lightOff() {
        if (!_state.value.flashOn) return
        camera.photoFlash = 0
        camLock.withLock { camera.setTorch(false) }
        _state.update { it.copy(flashOn = false) }
    }

    /** Lens selector: allowed while recording (the stream keeps running, only the zoom changes). */
    fun setZoom(ratio: Float): Job {
        val st = _state.value
        val r = ratio.coerceIn(st.minZoom, maxOf(st.maxZoom, st.minZoom))
        _state.update { it.copy(zoom = r) }
        return scope.launch {
            // Pinch fires many events: only the latest value matters.
            if (_state.value.zoom == r) camLock.withLock { camera.setZoom(r) }
        }
    }

    /** Buttons: ultra-wide (ratio < 1) when available, 1× and 2×; the rest by pinching. */
    private fun zoomStops(id: String): List<Float> {
        val r = camera.zoomRange(id) ?: return emptyList()
        val stops = buildList {
            if (r.lower < 0.95f) add(r.lower)
            add(1f)
            if (r.upper >= 2f) add(2f)
        }
        return if (stops.size > 1) stops else emptyList()
    }

    // ------------------------------------------------------------------ actions

    fun takePhoto(): Job = scope.launch {
        val reader = jpeg ?: return@launch
        val info = camera.current ?: return@launch
        val server = app.settings.server.value ?: return@launch
        _state.update { it.copy(busy = true) }
        try {
            val bytes = camera.capture(reader, CameraController.rotationFor(info, deviceDegrees), listOfNotNull(preview))
            _state.update { it.copy(flash = System.currentTimeMillis()) }
            val id = newSessionId()
            val header = baseHeader(id, 0, "photo", "jpeg", final = true) + mapOf(
                "photo" to mapOf("width" to reader.width, "height" to reader.height),
            )
            seal(server.recipient, id, 0, header, bytes)
        } catch (t: Throwable) {
            Log.e(TAG, "photo failed", t)
            _state.update { it.copy(error = "photo: ${t.message}") }
        } finally {
            _state.update { it.copy(busy = false) }
        }
    }

    fun startRecording(): Job = scope.launch {
        if (recorder != null) return@launch
        val info = camera.current ?: return@launch
        if (app.settings.server.value == null) return@launch
        if (lowStorage()) {
            _state.update { it.copy(error = "storage") }
            return@launch
        }
        val live = _state.value.mode == Mode.LIVE
        val prefs = app.settings.user.value
        val limits = app.settings.server.value!!.limits
        val height = prefs.videoHeight.coerceAtMost(limits.maxVideoHeight)
        val size = camera.videoSize(info.id, height)
        val bitrate = if (size.height > 720) limits.videoBitrate1080 else limits.videoBitrate720
        val cfg = VideoConfig(
            width = size.width, height = size.height, fps = 30, bitrate = bitrate,
            audio = ContextCompat.checkSelfPermission(app, android.Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED,
            micDevice = selectedMic(prefs.micDeviceId),
            segmentMs = if (live) 2000 else 4000,
            cameraRealtimeClock = info.timestampRealtime,
        )
        val id = newSessionId()
        session = id
        recordingInfo = mapOf(
            "codec" to "avc", "width" to size.width, "height" to size.height, "fps" to 30, "bitrate" to bitrate,
            "rotation" to CameraController.rotationFor(info, deviceDegrees), "live" to live,
        )
        val server = app.settings.server.value!!
        val rec = SegmentRecorder(cfg) { seg ->
            val header = baseHeader(id, seg.seq, "video", "mpegts", seg.isFinal) + mapOf(
                "start_ms" to seg.startMs, "duration_ms" to seg.durationMs, "video" to recordingInfo,
            )
            seal(server.recipient, id, seg.seq, header, seg.data)
            if (!seg.isFinal && lowStorage()) scope.launch { stopRecording(error = "storage") }
        }
        try {
            rec.prepare()
            recorder = rec
            reconfigure()
            rec.start()
            _state.update { it.copy(recording = true, recordingSince = System.currentTimeMillis(), error = null, sealedObjects = 0, sealedBytes = 0) }
            ContextCompat.startForegroundService(app, Intent(app, RecordingService::class.java))
            if (prefs.attachLocation) location.start()
            watchdog(rec)
        } catch (t: Throwable) {
            Log.e(TAG, "start recording failed", t)
            recorder = null
            runCatching { rec.stop() }
            _state.update { it.copy(error = "record: ${t.message}") }
            reconfigure()
        }
    }

    fun stopRecording(error: String? = null): Job = scope.launch {
        val rec = recorder ?: return@launch
        recorder = null
        _state.update { it.copy(busy = true) }
        // Stop the camera stream first so the encoder sees no new frames, then flush.
        runCatching { reconfigure() }
        rec.stop()
        session = null
        _state.update { it.copy(recording = false, recordingSince = null, busy = false, error = error ?: it.error) }
        app.stopService(Intent(app, RecordingService::class.java))
        if (preview == null) {
            camLock.withLock { camera.close() }
            orientation.disable()
            location.stop()
        }
    }

    // ------------------------------------------------------------------ internals

    private fun watchdog(rec: SegmentRecorder): Job = scope.launch {
        while (recorder === rec) {
            delay(1000)
            if (rec.error != null && recorder === rec) {
                val live = _state.value.mode == Mode.LIVE
                stopRecording(error = "encoder").join()
                if (live) { delay(1000); startRecording() } // live mode: keep going no matter what
            }
        }
    }

    private suspend fun onCameraLost() {
        val wasRecording = recorder != null
        val live = _state.value.mode == Mode.LIVE
        if (wasRecording) stopRecording(error = "camera").join()
        refreshCameras()
        if (preview != null || (wasRecording && live)) {
            delay(800)
            runCatching { reconfigure() }
            if (wasRecording && live) startRecording()
        }
    }

    private fun refreshCameras() {
        val list = runCatching { camera.list() }.getOrDefault(emptyList())
        val wanted = app.settings.user.value.cameraId
        val chosen = list.firstOrNull { it.id == wanted } ?: list.firstOrNull { it.facing == Facing.BACK } ?: list.firstOrNull()
        val size = chosen?.let { camera.videoSize(it.id, app.settings.user.value.videoHeight.coerceAtMost(app.settings.server.value?.limits?.maxVideoHeight ?: 1080)) }
        val stops = chosen?.let { zoomStops(it.id) } ?: emptyList()
        val range = chosen?.let { camera.zoomRange(it.id) }
        _state.update {
            it.copy(cameras = list, camera = chosen, previewSize = size, sensorOrientation = chosen?.sensorOrientation ?: 90,
                zoomStops = stops, zoom = 1f,
                minZoom = range?.lower ?: 1f, maxZoom = range?.upper?.coerceAtMost(10f) ?: 1f)
        }
    }

    /** One retry: quick camera/quality switches can race with a closing device or a resized preview. */
    private suspend fun reconfigure() = camLock.withLock {
        if (!reconfigureOnce(reportError = false)) {
            camera.close()
            delay(300)
            reconfigureOnce(reportError = true)
        }
    }

    private suspend fun reconfigureOnce(reportError: Boolean): Boolean {
        val info = _state.value.camera
        val rec = recorder
        if (info == null || (preview == null && rec == null)) {
            camera.close()
            return true
        }
        try {
            camera.open(info)
            val extra = mutableListOf<Surface>()
            if (rec == null && _state.value.mode == Mode.PHOTO) {
                val limits = app.settings.server.value?.limits
                val ps = camera.photoSize(info.id, limits?.photoMaxPixels ?: 4_000_000)
                if (jpeg?.width != ps.width || jpeg?.height != ps.height) {
                    jpeg?.close()
                    jpeg = ImageReader.newInstance(ps.width, ps.height, ImageFormat.JPEG, 2)
                }
                extra += jpeg!!.surface
            }
            camera.configure(listOfNotNull(preview, rec?.inputSurface), extra, recording = rec != null)
            return true
        } catch (t: Throwable) {
            Log.w(TAG, "camera configure failed (report=$reportError)", t)
            if (reportError) _state.update { it.copy(error = "camera: ${t.message}") }
            return false
        }
    }

    private fun selectedMic(id: Int?): AudioDeviceInfo? {
        if (id == null) return null
        val am = app.getSystemService(AudioManager::class.java)
        val dev = am.getDevices(AudioManager.GET_DEVICES_INPUTS).firstOrNull { it.id == id } ?: return null
        if (dev.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO && Build.VERSION.SDK_INT >= 31) {
            runCatching { am.setCommunicationDevice(dev) }
        }
        return dev
    }

    private fun baseHeader(session: String, seq: Int, kind: String, container: String, final: Boolean): Map<String, Any?> = mapOf(
        "v" to 1,
        "session" to session,
        "seq" to seq,
        "kind" to kind,
        "container" to container,
        "final" to final,
        "created_at" to Instant.now().toString(),
        "device" to mapOf(
            "platform" to "android",
            "os" to Build.VERSION.RELEASE,
            "model" to "${Build.MANUFACTURER} ${Build.MODEL}",
            "app" to BuildConfig.VERSION_NAME,
            "camera" to _state.value.camera?.facing?.name?.lowercase(),
        ),
        "location" to if (app.settings.user.value.attachLocation) location.last() else null,
    )

    /** Seal + enqueue. Plaintext only exists in RAM; the outbox only ever receives ciphertext. */
    private fun seal(recipient: String, session: String, seq: Int, header: Map<String, Any?>, content: ByteArray) {
        sealer.execute {
            try {
                val sealed = AgeSealer.seal(Recipient.parse(recipient), Envelope.encode(header, content))
                content.fill(0)
                app.outbox.put(session, seq, sealed)
                _state.update { it.copy(sealedObjects = it.sealedObjects + 1, sealedBytes = it.sealedBytes + sealed.size) }
                app.uploader.kick()
            } catch (t: Throwable) {
                Log.e(TAG, "sealing failed", t)
                _state.update { it.copy(error = "seal: ${t.message}") }
            }
        }
    }

    private fun lowStorage(): Boolean = StatFs(app.noBackupFilesDir.path).availableBytes < MIN_FREE_BYTES

    private fun newSessionId(): String {
        val b = ByteArray(16).also { rng.nextBytes(it) }
        return b.joinToString("") { "%02x".format(it) }
    }

    companion object {
        private const val TAG = "SR.Engine"
        private const val MIN_FREE_BYTES = 300L * 1024 * 1024
    }
}
