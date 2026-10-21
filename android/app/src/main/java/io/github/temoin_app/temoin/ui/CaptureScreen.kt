package io.github.temoin_app.temoin.ui

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import android.graphics.SurfaceTexture
import android.view.Surface
import android.view.TextureView
import android.view.WindowManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Block
import androidx.compose.material.icons.filled.Cameraswitch
import androidx.compose.material.icons.filled.CloudDone
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.CloudUpload
import androidx.compose.material.icons.filled.DarkMode
import androidx.compose.material.icons.filled.FlashOff
import androidx.compose.material.icons.filled.FlashOn
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.PauseCircle
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import io.github.temoin_app.temoin.R
import io.github.temoin_app.temoin.TemoinApp
import io.github.temoin_app.temoin.capture.Facing
import io.github.temoin_app.temoin.capture.Mode
import io.github.temoin_app.temoin.net.Api
import io.github.temoin_app.temoin.net.Uploader
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

@Composable
fun CaptureScreen(app: TemoinApp, onSettings: () -> Unit, settingsAllowed: Boolean = true) {
    val context = LocalContext.current
    val engine = app.engine
    val st by engine.state.collectAsState()
    val server by app.settings.server.collectAsState()
    val queue by app.outbox.stats.collectAsState()
    val upload by app.uploader.status.collectAsState()
    var serverAccepting by remember { mutableStateOf(true) }
    var keyChanged by remember { mutableStateOf(false) }

    // ---- permissions
    fun granted(p: String) = ContextCompat.checkSelfPermission(context, p) == PackageManager.PERMISSION_GRANTED
    var hasCamera by remember { mutableStateOf(granted(Manifest.permission.CAMERA)) }
    val perms = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        hasCamera = granted(Manifest.permission.CAMERA)
    }
    val wanted = buildList {
        add(Manifest.permission.CAMERA); add(Manifest.permission.RECORD_AUDIO)
        if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
        if (Build.VERSION.SDK_INT >= 31) add(Manifest.permission.BLUETOOTH_CONNECT)
    }.toTypedArray()
    LaunchedEffect(Unit) { if (wanted.any { !granted(it) }) perms.launch(wanted) }
    LaunchedEffect(hasCamera) { if (hasCamera) engine.prepare() }

    // ---- server health + limits (the pinned key is never replaced)
    LaunchedEffect(server?.url) {
        val s = server ?: return@LaunchedEffect
        while (true) {
            runCatching { withContext(Dispatchers.IO) { Api.info(s.url, key = app.settings.deviceKey) } }.onSuccess { info ->
                keyChanged = info.recipient != s.recipient
                serverAccepting = info.accepting
                if (!keyChanged) app.settings.updateLimits(info.limits)
            }
            app.uploader.kick()
            delay(60_000)
        }
    }

    // ---- keep screen on while recording; dark (discreet) mode
    var discreet by remember { mutableStateOf(false) }
    val view = LocalView.current
    DisposableEffect(st.recording) {
        view.keepScreenOn = st.recording
        if (!st.recording) discreet = false
        onDispose { view.keepScreenOn = false }
    }
    // Window brightness only (reverts by itself when the app leaves the screen or the phone locks).
    val frontLight = st.flashOn && st.camera?.facing == Facing.FRONT
    DisposableEffect(discreet, frontLight) {
        val w = (context as Activity).window
        val lp = w.attributes
        lp.screenBrightness = when {
            discreet -> 0.01f
            frontLight -> 1f
            else -> WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE
        }
        w.attributes = lp
        onDispose {
            val l = w.attributes; l.screenBrightness = WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE; w.attributes = l
        }
    }

    // ---- shutter flash
    val flash = remember { Animatable(0f) }
    LaunchedEffect(st.flash) {
        if (st.flash != 0L) { flash.snapTo(0.8f); flash.animateTo(0f, tween(250)) }
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        // ---- preview
        val size = st.previewSize
        if (hasCamera && size != null) {
            val portrait = st.sensorOrientation % 180 == 90
            val ratio = if (portrait) size.height.toFloat() / size.width else size.width.toFloat() / size.height
            Box(
                Modifier.fillMaxSize().pointerInput(Unit) {
                    // Pinch to zoom, also while recording.
                    detectTransformGestures { _, _, zoomChange, _ ->
                        if (zoomChange != 1f) engine.setZoom(engine.state.value.zoom * zoomChange)
                    }
                },
                contentAlignment = Alignment.Center,
            ) {
                AndroidView(
                    modifier = Modifier.fillMaxWidth().aspectRatio(ratio),
                    factory = { ctx ->
                        // TextureView (not SurfaceView) so the Compose controls stay on top of the preview.
                        TextureView(ctx).apply tv@{
                            surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                                private var surface: Surface? = null
                                override fun onSurfaceTextureAvailable(t: SurfaceTexture, w: Int, h: Int) {
                                    val want = engine.state.value.previewSize ?: return
                                    t.setDefaultBufferSize(want.width, want.height)
                                    this@tv.tag = want
                                    surface = Surface(t).also { engine.attachPreview(it) }
                                }
                                override fun onSurfaceTextureSizeChanged(t: SurfaceTexture, w: Int, h: Int) {}
                                override fun onSurfaceTextureDestroyed(t: SurfaceTexture): Boolean {
                                    engine.detachPreview()
                                    surface?.release()
                                    surface = null
                                    return true
                                }
                                override fun onSurfaceTextureUpdated(t: SurfaceTexture) {}
                            }
                        }
                    },
                    update = { v ->
                        // Preview size changed (camera / quality switch): resize the buffer and reattach.
                        val t = v.surfaceTexture ?: return@AndroidView
                        if (v.tag != size) {
                            v.tag = size
                            t.setDefaultBufferSize(size.width, size.height)
                            engine.attachPreview(Surface(t))
                        }
                    },
                )
            }
        } else if (!hasCamera) {
            Column(Modifier.align(Alignment.Center).padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(stringResource(R.string.perm_needed), textAlign = TextAlign.Center)
                Spacer(Modifier.height(16.dp))
                Button(onClick = { perms.launch(wanted) }) { Text(stringResource(R.string.grant)) }
            }
        }
        // Front "flash": a translucent white veil, the scene stays visible.
        if (st.flashOn && st.camera?.facing == Facing.FRONT) Box(Modifier.fillMaxSize().background(Color.White.copy(alpha = 0.6f)))
        Box(Modifier.fillMaxSize().background(Color.White.copy(alpha = flash.value)))

        Column(Modifier.fillMaxSize().safeDrawingPadding()) {
            // ---- top bar
            Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                StatusChip(
                    server?.name ?: "",
                    queue.count, queue.bytes, upload.state, serverAccepting, Modifier.weight(1f),
                )
                if (st.camera?.hasFlash == true || st.camera?.facing == Facing.FRONT) {
                    IconButton(onClick = { engine.toggleFlash() }) {
                        Icon(
                            if (st.flashOn) Icons.Default.FlashOn else Icons.Default.FlashOff, stringResource(R.string.flash),
                            tint = if (st.flashOn) Color(0xFFFFD60A) else Color.White,
                        )
                    }
                }
                if (settingsAllowed) {
                    IconButton(onClick = onSettings, enabled = !st.recording) { Icon(Icons.Default.Settings, stringResource(R.string.settings)) }
                }
            }
            if (keyChanged) Banner(stringResource(R.string.err_key_changed))
            if (upload.state == Uploader.State.BANNED) Banner(stringResource(R.string.banner_banned))
            if (upload.state == Uploader.State.NOT_REGISTERED) Banner(stringResource(R.string.banner_not_registered))
            st.error?.let { e ->
                val msg = when {
                    e == "storage" -> stringResource(R.string.err_storage)
                    e.startsWith("camera") -> stringResource(R.string.err_camera, e.removePrefix("camera").trimStart(':', ' '))
                    else -> e
                }
                Banner(msg) { engine.clearError() }
            }
            Spacer(Modifier.weight(1f))

            // ---- recording indicator
            if (st.recording) {
                RecordingTimer(st.recordingSince ?: System.currentTimeMillis(), st.sealedObjects)
            } else if (st.mode == Mode.LIVE) {
                Text(stringResource(R.string.live_hint), color = Color(0xFFDDDDDD), fontSize = 13.sp, textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 32.dp, vertical = 8.dp))
            }

            // ---- lens selector (also while recording)
            if (st.zoomStops.size > 1) {
                Row(Modifier.fillMaxWidth().padding(bottom = 6.dp), horizontalArrangement = Arrangement.Center) {
                    Row(
                        Modifier.clip(RoundedCornerShape(50)).background(Color.Black.copy(alpha = 0.3f)).padding(6.dp),
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        val current = st.zoomStops.lastOrNull { it <= st.zoom + 0.01f } ?: st.zoomStops.first()
                        st.zoomStops.forEach { z ->
                            val sel = z == current
                            Box(
                                Modifier.size(if (sel) 40.dp else 32.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.55f))
                                    .clickable { engine.setZoom(z) },
                                contentAlignment = Alignment.Center,
                            ) {
                                // The selected bubble shows the real zoom while pinching ("3,4×").
                                Text(zoomLabel(if (sel) st.zoom else z), color = if (sel) Color(0xFFFFD60A) else Color.White,
                                    fontSize = if (sel) 13.sp else 11.sp, fontWeight = FontWeight.Bold)
                            }
                        }
                    }
                }
            }

            // ---- modes
            if (!st.recording) {
                // Equal widths, and the chosen mode slides right above the shutter (like the system camera).
                val modes = listOf(Mode.PHOTO to R.string.mode_photo, Mode.VIDEO to R.string.mode_video, Mode.LIVE to R.string.mode_live)
                val cell = 96.dp
                val index = modes.indexOfFirst { it.first == st.mode }.coerceAtLeast(0)
                val shift by animateDpAsState(cell * ((modes.size - 1) / 2f - index), label = "modes")
                Row(Modifier.fillMaxWidth().padding(vertical = 8.dp).offset(x = shift), horizontalArrangement = Arrangement.Center) {
                    modes.forEach { (m, label) ->
                        val sel = st.mode == m
                        Text(
                            stringResource(label).uppercase(),
                            color = if (sel) (if (m == Mode.PHOTO) Color.White else Accent) else Color(0xFF9AA0A6),
                            fontWeight = if (sel) FontWeight.Bold else FontWeight.Medium,
                            fontSize = 14.sp,
                            textAlign = TextAlign.Center,
                            modifier = Modifier.width(cell).clip(RoundedCornerShape(50)).clickable { engine.setMode(m) }.padding(vertical = 8.dp),
                        )
                    }
                }
            }

            // ---- shutter row
            Row(
                Modifier.fillMaxWidth().padding(bottom = 24.dp, top = 8.dp, start = 32.dp, end = 32.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Box(Modifier.size(52.dp), contentAlignment = Alignment.Center) {
                    if (st.recording && st.mode != Mode.PHOTO) {
                        IconButton(onClick = { discreet = true }) { Icon(Icons.Default.DarkMode, stringResource(R.string.discreet)) }
                    }
                }
                Shutter(
                    mode = st.mode, recording = st.recording, enabled = hasCamera && !st.busy && !keyChanged && st.camera != null,
                    onClick = {
                        when {
                            st.mode == Mode.PHOTO -> engine.takePhoto()
                            st.recording -> engine.stopRecording()
                            else -> engine.startRecording()
                        }
                    },
                )
                Box(Modifier.size(52.dp), contentAlignment = Alignment.Center) {
                    if (!st.recording && st.cameras.size > 1) {
                        IconButton(onClick = {
                            val i = st.cameras.indexOfFirst { it.id == st.camera?.id }
                            engine.selectCamera(st.cameras[(i + 1) % st.cameras.size].id)
                        }) { Icon(Icons.Default.Cameraswitch, stringResource(R.string.switch_camera)) }
                    }
                }
            }
        }

        if (discreet) DiscreetOverlay { discreet = false }
    }
}

@Composable
private fun Shutter(mode: Mode, recording: Boolean, enabled: Boolean, onClick: () -> Unit) {
    Box(
        Modifier.size(78.dp).clip(CircleShape).border(4.dp, Color.White, CircleShape)
            .clickable(enabled = enabled, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        val inner = when {
            recording -> Modifier.size(30.dp).clip(RoundedCornerShape(6.dp)).background(Accent)
            mode == Mode.PHOTO -> Modifier.size(62.dp).clip(CircleShape).background(if (enabled) Color.White else Color.Gray)
            else -> Modifier.size(62.dp).clip(CircleShape).background(if (enabled) Accent else Color.Gray)
        }
        Box(inner)
    }
}

@Composable
private fun StatusChip(name: String, count: Int, bytes: Long, state: Uploader.State, accepting: Boolean, modifier: Modifier) {
    val (icon, tint) = when {
        state == Uploader.State.BANNED || state == Uploader.State.NOT_REGISTERED -> Icons.Default.Block to Accent
        !accepting || state == Uploader.State.PAUSED_BY_SERVER -> Icons.Default.PauseCircle to Color(0xFFF5A524)
        count == 0 -> Icons.Default.CloudDone to Ok
        state == Uploader.State.WAITING_NETWORK -> Icons.Default.CloudOff to Color(0xFFF5A524)
        else -> Icons.Default.CloudUpload to Color.White
    }
    val detail = when {
        count == 0 -> stringResource(R.string.queue_empty)
        else -> stringResource(
            R.string.queue_pending, count,
            when (state) {
                Uploader.State.WAITING_NETWORK -> stringResource(R.string.up_waiting)
                Uploader.State.PAUSED_BY_SERVER -> stringResource(R.string.up_paused)
                Uploader.State.BANNED -> stringResource(R.string.up_banned)
                Uploader.State.NOT_REGISTERED -> stringResource(R.string.up_not_registered)
                Uploader.State.ERROR -> stringResource(R.string.up_error)
                else -> stringResource(R.string.up_uploading)
            },
        ) + " · " + formatBytes(bytes)
    }
    Row(
        modifier.clip(RoundedCornerShape(50)).background(Color.Black.copy(alpha = 0.55f)).padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, tint = tint, modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(8.dp))
        Column {
            Text(name, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1)
            Text(detail, fontSize = 11.sp, color = Color(0xFFBDBDC2), maxLines = 1)
        }
    }
}

@Composable
private fun RecordingTimer(since: Long, sealed: Int) {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(since) { while (true) { now = System.currentTimeMillis(); delay(500) } }
    val s = ((now - since) / 1000).coerceAtLeast(0)
    Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
        Row(
            Modifier.clip(RoundedCornerShape(50)).background(Accent).padding(horizontal = 14.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.size(8.dp).clip(CircleShape).background(Color.White))
            Spacer(Modifier.width(8.dp))
            Text("%02d:%02d:%02d".format(s / 3600, s / 60 % 60, s % 60), fontWeight = FontWeight.Bold)
        }
        Row(Modifier.padding(top = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Default.Lock, null, Modifier.size(12.dp), tint = Color(0xFFBDBDC2))
            Spacer(Modifier.width(4.dp))
            Text(stringResource(R.string.sealed_info, sealed), fontSize = 11.sp, color = Color(0xFFBDBDC2))
        }
    }
}

@Composable
private fun Banner(text: String, onClose: (() -> Unit)? = null) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp).clip(RoundedCornerShape(12.dp))
            .background(Accent.copy(alpha = 0.9f)).clickable(enabled = onClose != null) { onClose?.invoke() }.padding(12.dp),
    ) { Text(text, fontSize = 13.sp, style = MaterialTheme.typography.bodyMedium) }
}

@Composable
private fun DiscreetOverlay(onExit: () -> Unit) {
    var taps by remember { mutableIntStateOf(0) }
    var last by remember { mutableLongStateOf(0L) }
    Box(
        Modifier.fillMaxSize().background(Color.Black).pointerInput(Unit) {
            detectTapGestures {
                val t = System.currentTimeMillis()
                taps = if (t - last < 600) taps + 1 else 1
                last = t
                if (taps >= 3) onExit()
            }
        },
        contentAlignment = Alignment.BottomCenter,
    ) {
        Text(stringResource(R.string.discreet_hint), color = Color(0xFF2A2A2A), fontSize = 11.sp, modifier = Modifier.padding(32.dp))
    }
}

fun formatBytes(n: Long): String {
    val units = listOf("B", "KB", "MB", "GB", "TB")
    var v = n.toDouble()
    var i = 0
    while (v >= 1024 && i < units.size - 1) { v /= 1024; i++ }
    return if (i == 0) "$n B" else "%.1f %s".format(v, units[i])
}

/** "0,6×", "1×", "2×" — decimal only when needed, in the user's locale. */
private fun zoomLabel(z: Float): String {
    val f = java.text.NumberFormat.getNumberInstance().apply { maximumFractionDigits = 1; minimumFractionDigits = 0 }
    return f.format(z) + "×"
}
