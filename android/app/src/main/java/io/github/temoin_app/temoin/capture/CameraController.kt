package io.github.temoin_app.temoin.capture

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.params.OutputConfiguration
import android.hardware.camera2.params.SessionConfiguration
import android.media.ImageReader
import android.media.MediaCodec
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.util.Range
import android.util.Size
import android.view.Surface
import kotlinx.coroutines.suspendCancellableCoroutine
import java.util.concurrent.Executor
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

enum class Facing { BACK, FRONT, EXTERNAL }

data class CameraInfo(
    val id: String,
    val facing: Facing,
    val sensorOrientation: Int,
    val timestampRealtime: Boolean,
    val hasFlash: Boolean = false,
)

/** Thin coroutine wrapper over Camera2. One device + one session at a time. */
class CameraController(context: Context) {
    private val manager = context.getSystemService(CameraManager::class.java)
    private val thread = HandlerThread("sr-camera").apply { start() }
    val handler = Handler(thread.looper)
    private val executor = Executor { handler.post(it) }

    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    /** Zoom ratio applied to every request (1 = main lens; < 1 = ultra-wide on multi-cameras). */
    @Volatile var zoom: Float = 1f
        private set
    /** Torch for video; photo flash: 0 off, 1 auto, 2 on. */
    @Volatile var torch: Boolean = false
        private set
    @Volatile var photoFlash: Int = 0
    private var lastRepeating: List<Surface> = emptyList()
    private var lastRecording = false
    private var lastFps = 30
    var current: CameraInfo? = null
        private set
    var onDisconnected: (() -> Unit)? = null

    fun list(): List<CameraInfo> = manager.cameraIdList.mapNotNull { id ->
        runCatching {
            val c = manager.getCameraCharacteristics(id)
            val caps = c.get(CameraCharacteristics.REQUEST_AVAILABLE_CAPABILITIES) ?: intArrayOf()
            if (CameraCharacteristics.REQUEST_AVAILABLE_CAPABILITIES_BACKWARD_COMPATIBLE !in caps) return@runCatching null
            CameraInfo(
                id = id,
                facing = when (c.get(CameraCharacteristics.LENS_FACING)) {
                    CameraCharacteristics.LENS_FACING_FRONT -> Facing.FRONT
                    CameraCharacteristics.LENS_FACING_EXTERNAL -> Facing.EXTERNAL
                    else -> Facing.BACK
                },
                sensorOrientation = c.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0,
                timestampRealtime = c.get(CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE) ==
                    CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE_REALTIME,
                hasFlash = c.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true,
            )
        }.getOrNull()
    }

    /** Hot-plug of USB (UVC) cameras. */
    fun registerAvailability(onChange: () -> Unit): CameraManager.AvailabilityCallback {
        val cb = object : CameraManager.AvailabilityCallback() {
            override fun onCameraAvailable(cameraId: String) = onChange()
            override fun onCameraUnavailable(cameraId: String) = onChange()
        }
        manager.registerAvailabilityCallback(cb, handler)
        return cb
    }

    fun unregisterAvailability(cb: CameraManager.AvailabilityCallback) = manager.unregisterAvailabilityCallback(cb)

    fun videoSize(id: String, targetHeight: Int): Size {
        val map = manager.getCameraCharacteristics(id).get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)!!
        val sizes = map.getOutputSizes(MediaCodec::class.java)?.toList().orEmpty()
            .ifEmpty { map.getOutputSizes(android.graphics.SurfaceTexture::class.java).toList() }
        val wide = sizes.filter { it.width * 9 == it.height * 16 }
        return (wide.firstOrNull { it.height == targetHeight }
            ?: wide.filter { it.height <= targetHeight }.maxByOrNull { it.height }
            ?: sizes.filter { it.height <= targetHeight }.maxByOrNull { it.width * it.height }
            ?: sizes.minBy { it.width * it.height })
    }

    fun photoSize(id: String, maxPixels: Int): Size {
        val map = manager.getCameraCharacteristics(id).get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)!!
        val sizes = map.getOutputSizes(ImageFormat.JPEG).toList()
        val fit = sizes.filter { it.width.toLong() * it.height <= maxPixels }
        // Prefer the sensor's native 4:3 when available.
        return (fit.filter { it.width * 3 == it.height * 4 }.maxByOrNull { it.width * it.height }
            ?: fit.maxByOrNull { it.width * it.height }
            ?: sizes.minBy { it.width * it.height })
    }

    fun fpsRange(id: String, fps: Int): Range<Int>? =
        manager.getCameraCharacteristics(id).get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
            ?.filter { it.upper == fps }?.maxByOrNull { it.lower }

    @SuppressLint("MissingPermission")
    suspend fun open(info: CameraInfo) {
        if (current?.id == info.id && device != null) return
        close()
        device = suspendCancellableCoroutine { cont ->
            manager.openCamera(info.id, executor, object : CameraDevice.StateCallback() {
                override fun onOpened(d: CameraDevice) {
                    if (cont.isActive) cont.resume(d) else d.close()
                }
                override fun onDisconnected(d: CameraDevice) {
                    Log.w(TAG, "camera ${info.id} disconnected")
                    d.close()
                    if (device === d) { device = null; session = null; current = null; onDisconnected?.invoke() }
                    if (cont.isActive) cont.resumeWithException(IllegalStateException("camera disconnected"))
                }
                override fun onError(d: CameraDevice, error: Int) {
                    Log.e(TAG, "camera ${info.id} error $error")
                    d.close()
                    if (device === d) { device = null; session = null; current = null; onDisconnected?.invoke() }
                    if (cont.isActive) cont.resumeWithException(IllegalStateException("camera error $error"))
                }
            })
        }
        current = info
    }

    /**
     * (Re)configures the capture session. [repeating] surfaces receive the continuous
     * stream (preview and/or encoder); [extra] surfaces are only configured (JPEG reader).
     */
    suspend fun configure(repeating: List<Surface>, extra: List<Surface> = emptyList(), recording: Boolean, fps: Int = 30) {
        val d = device ?: throw IllegalStateException("camera not open")
        session?.close()
        session = null
        val outputs = (repeating + extra).map { OutputConfiguration(it) }
        val s = suspendCancellableCoroutine { cont ->
            d.createCaptureSession(SessionConfiguration(SessionConfiguration.SESSION_REGULAR, outputs, executor,
                object : CameraCaptureSession.StateCallback() {
                    override fun onConfigured(s: CameraCaptureSession) { if (cont.isActive) cont.resume(s) }
                    override fun onConfigureFailed(s: CameraCaptureSession) {
                        if (cont.isActive) cont.resumeWithException(IllegalStateException("session configuration failed"))
                    }
                }))
        }
        session = s
        lastRepeating = repeating
        lastRecording = recording
        lastFps = fps
        if (repeating.isEmpty()) return
        startRepeating(d, s)
    }

    private fun startRepeating(d: CameraDevice, s: CameraCaptureSession) {
        val req = d.createCaptureRequest(if (lastRecording) CameraDevice.TEMPLATE_RECORD else CameraDevice.TEMPLATE_PREVIEW).apply {
            lastRepeating.forEach { addTarget(it) }
            set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO)
            set(CaptureRequest.CONTROL_AF_MODE,
                if (lastRecording) CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO else CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
            current?.let { c -> fpsRange(c.id, lastFps)?.let { set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it) } }
            if (lastRecording) set(CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE, CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON)
            applyZoom(this)
            applyFlash(this, still = false)
        }.build()
        s.setRepeatingRequest(req, null, handler)
    }

    /** Torch on/off on the running stream (works while recording). */
    fun setTorch(on: Boolean) {
        torch = on
        val d = device ?: return
        val s = session ?: return
        if (lastRepeating.isNotEmpty()) runCatching { startRepeating(d, s) }
    }

    private fun applyFlash(b: CaptureRequest.Builder, still: Boolean) {
        if (current?.hasFlash != true) return
        when {
            torch -> {
                b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
                b.set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_TORCH)
            }
            photoFlash == 1 -> b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON_AUTO_FLASH)
            photoFlash == 2 -> b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON_ALWAYS_FLASH)
            else -> {
                b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
                b.set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_OFF)
            }
        }
        if (still && photoFlash != 0 && !torch) b.set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_SINGLE)
    }

    /** Zoom ratios supported by the camera (Android 11+); null when it cannot zoom. */
    fun zoomRange(id: String): Range<Float>? {
        if (Build.VERSION.SDK_INT < 30) return null
        return manager.getCameraCharacteristics(id).get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)
    }

    /**
     * Changes the zoom ratio of the running stream, without reconfiguring the session:
     * works while recording. On logical multi-cameras ratios below 1 switch to the
     * ultra-wide lens, like the system camera.
     */
    fun setZoom(ratio: Float) {
        zoom = ratio
        val d = device ?: return
        val s = session ?: return
        if (lastRepeating.isNotEmpty()) runCatching { startRepeating(d, s) }
    }

    private fun applyZoom(b: CaptureRequest.Builder) {
        val c = current ?: return
        val range = zoomRange(c.id) ?: return
        if (Build.VERSION.SDK_INT >= 30) b.set(CaptureRequest.CONTROL_ZOOM_RATIO, zoom.coerceIn(range.lower, range.upper))
    }

    /** Captures one JPEG into memory. */
    suspend fun capture(reader: ImageReader, jpegOrientation: Int, previewTargets: List<Surface>): ByteArray {
        val d = device ?: throw IllegalStateException("camera not open")
        val s = session ?: throw IllegalStateException("no session")
        if (photoFlash != 0 && current?.hasFlash == true && previewTargets.isNotEmpty()) {
            // Pre-flash metering so the exposure accounts for the flash.
            val pre = d.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply {
                previewTargets.forEach { addTarget(it) }
                applyZoom(this)
                applyFlash(this, still = false)
                set(CaptureRequest.CONTROL_AE_PRECAPTURE_TRIGGER, CaptureRequest.CONTROL_AE_PRECAPTURE_TRIGGER_START)
            }.build()
            runCatching { s.capture(pre, null, handler) }
            kotlinx.coroutines.delay(450)
        }
        return suspendCancellableCoroutine { cont ->
            reader.setOnImageAvailableListener({ r ->
                val img = r.acquireLatestImage() ?: return@setOnImageAvailableListener
                try {
                    val buf = img.planes[0].buffer
                    val bytes = ByteArray(buf.remaining()).also { buf.get(it) }
                    if (cont.isActive) cont.resume(bytes)
                } finally {
                    img.close()
                    r.setOnImageAvailableListener(null, null)
                }
            }, handler)
            val req = d.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE).apply {
                addTarget(reader.surface)
                previewTargets.forEach { addTarget(it) }
                set(CaptureRequest.JPEG_ORIENTATION, jpegOrientation)
                set(CaptureRequest.JPEG_QUALITY, 90.toByte())
                applyZoom(this)
                applyFlash(this, still = true)
                set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
            }.build()
            s.capture(req, null, handler)
        }
    }

    fun close() {
        zoom = 1f
        torch = false
        runCatching { session?.close() }
        runCatching { device?.close() }
        session = null
        device = null
        current = null
    }

    companion object {
        private const val TAG = "SR.Camera"

        /** Orientation of the captured image relative to the device's natural orientation. */
        fun rotationFor(info: CameraInfo, deviceDegrees: Int): Int {
            if (info.facing == Facing.EXTERNAL) return 0
            var d = ((deviceDegrees + 45) / 90 * 90) % 360
            if (info.facing == Facing.FRONT) d = -d
            return (info.sensorOrientation + d + 360) % 360
        }
    }
}
