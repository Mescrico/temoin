package io.github.temoin_app.temoin.ui

import android.Manifest
import android.media.AudioDeviceInfo
import android.media.AudioManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.temoin_app.temoin.R
import io.github.temoin_app.temoin.TemoinApp
import io.github.temoin_app.temoin.capture.Facing
import io.github.temoin_app.temoin.crypto.Recipient
import io.github.temoin_app.temoin.net.Api
import io.github.temoin_app.temoin.net.ApiException
import io.github.temoin_app.temoin.net.DeviceStatus
import io.github.temoin_app.temoin.net.HandBack
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import android.widget.Toast
import kotlinx.coroutines.launch
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.runtime.LaunchedEffect
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

private val Muted = Color(0xFF9AA0A6)

@Composable
fun SettingsScreen(app: TemoinApp, onBack: () -> Unit) {
    val server by app.settings.server.collectAsState()
    val prefs by app.settings.user.collectAsState()
    val engine by app.engine.state.collectAsState()
    val queue by app.outbox.stats.collectAsState()
    val upload by app.uploader.status.collectAsState()
    var warn1080 by remember { mutableStateOf(false) }
    var confirmForget by remember { mutableStateOf(false) }
    val s = server ?: return

    val locationPerm = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        if (ok) app.settings.updateUser { it.copy(attachLocation = true) }
    }

    Column(Modifier.fillMaxSize().background(Color.Black).safeDrawingPadding()) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(4.dp)) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back)) }
            Text(stringResource(R.string.settings), style = MaterialTheme.typography.titleLarge)
        }
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            DeviceSection(app)
            HandBackSection(app)

            Section(stringResource(R.string.s_server)) {
                Text(s.name, fontWeight = FontWeight.SemiBold)
                Text(s.url, color = Muted, fontSize = 13.sp)
                Spacer(Modifier.width(4.dp))
                Text(stringResource(R.string.s_fingerprint), color = Muted, fontSize = 12.sp)
                Text(Recipient.fingerprint(s.recipient), fontFamily = FontFamily.Monospace)
            }

            Section(stringResource(R.string.s_quality)) {
                Choice(stringResource(R.string.q_720), prefs.videoHeight <= 720) {
                    app.settings.updateUser { it.copy(videoHeight = 720) }; app.engine.settingsChanged()
                }
                val allowed1080 = s.limits.maxVideoHeight >= 1080
                Choice(stringResource(R.string.q_1080), prefs.videoHeight >= 1080, enabled = allowed1080) { warn1080 = true }
                if (!allowed1080) Text(stringResource(R.string.q_1080_blocked, s.limits.maxVideoHeight), color = Muted, fontSize = 12.sp)
            }

            Section(stringResource(R.string.s_camera)) {
                engine.cameras.forEach { c ->
                    val label = when (c.facing) {
                        Facing.BACK -> stringResource(R.string.cam_back)
                        Facing.FRONT -> stringResource(R.string.cam_front)
                        Facing.EXTERNAL -> stringResource(R.string.cam_external)
                    } + "  ·  #${c.id}"
                    Choice(label, engine.camera?.id == c.id) { app.engine.selectCamera(c.id) }
                }
                Text(stringResource(R.string.cam_external_help), color = Muted, fontSize = 12.sp)
            }

            Section(stringResource(R.string.s_mic)) {
                val am = app.getSystemService(AudioManager::class.java)
                val mics = remember {
                    am.getDevices(AudioManager.GET_DEVICES_INPUTS).filter {
                        it.type in setOf(
                            AudioDeviceInfo.TYPE_BUILTIN_MIC, AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_HEADSET,
                            AudioDeviceInfo.TYPE_WIRED_HEADSET, AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
                        )
                    }
                }
                Choice(stringResource(R.string.mic_default), prefs.micDeviceId == null) { app.settings.updateUser { it.copy(micDeviceId = null) } }
                mics.forEach { m ->
                    val kind = when (m.type) {
                        AudioDeviceInfo.TYPE_BUILTIN_MIC -> "📱"
                        AudioDeviceInfo.TYPE_BLUETOOTH_SCO -> "Bluetooth"
                        AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_HEADSET -> "USB"
                        else -> "Jack"
                    }
                    Choice("$kind · ${m.productName}", prefs.micDeviceId == m.id) { app.settings.updateUser { it.copy(micDeviceId = m.id) } }
                }
            }

            Section(null) {
                Toggle(stringResource(R.string.s_location), stringResource(R.string.s_location_help), prefs.attachLocation) { on ->
                    if (on) locationPerm.launch(Manifest.permission.ACCESS_FINE_LOCATION)
                    else app.settings.updateUser { it.copy(attachLocation = false) }
                }
                Toggle(stringResource(R.string.s_wifi), null, prefs.wifiOnly) { on -> app.settings.updateUser { it.copy(wifiOnly = on) } }
            }

            Section(stringResource(R.string.s_queue)) {
                Text("${queue.count} · ${formatBytes(queue.bytes)}")
                TextButton(onClick = { app.uploader.kick() }) { Text(stringResource(R.string.s_send_now)) }
                if (upload.rejected > 0) {
                    Text(stringResource(R.string.s_rejected, upload.rejected), color = Accent, fontSize = 13.sp)
                    TextButton(onClick = { app.uploader.clearRejected() }) { Text(stringResource(R.string.s_clear)) }
                }
            }

            Section(stringResource(R.string.s_about)) {
                Text(stringResource(R.string.s_about_body), color = Muted, fontSize = 13.sp)
            }

            TextButton(onClick = { confirmForget = true }) { Text(stringResource(R.string.s_forget), color = Accent) }
            Spacer(Modifier.width(32.dp))
        }
    }

    if (warn1080) {
        AlertDialog(
            onDismissRequest = { warn1080 = false },
            title = { Text(stringResource(R.string.q_1080_warn_title)) },
            text = { Text(stringResource(R.string.q_1080_warn)) },
            confirmButton = {
                TextButton(onClick = {
                    app.settings.updateUser { it.copy(videoHeight = 1080) }; app.engine.settingsChanged(); warn1080 = false
                }) { Text(stringResource(R.string.q_1080)) }
            },
            dismissButton = { TextButton(onClick = { warn1080 = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
    if (confirmForget) {
        AlertDialog(
            onDismissRequest = { confirmForget = false },
            title = { Text(stringResource(R.string.s_forget)) },
            text = { if (queue.count > 0) Text(stringResource(R.string.s_forget_warn, queue.count)) },
            confirmButton = {
                TextButton(onClick = {
                    app.outbox.list().forEach { app.outbox.remove(it) }
                    app.uploader.clearRejected()
                    app.settings.forgetServer()
                    confirmForget = false
                    onBack()
                }) { Text(stringResource(R.string.s_forget_confirm), color = Accent) }
            },
            dismissButton = { TextButton(onClick = { confirmForget = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

@Composable
private fun Section(title: String?, content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(Color(0xFF14161B)).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (title != null) Text(title.uppercase(), color = Muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        content()
    }
}

@Composable
private fun Choice(label: String, selected: Boolean, enabled: Boolean = true, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).clickable(enabled = enabled, onClick = onClick),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RadioButton(selected = selected, onClick = onClick, enabled = enabled)
        Text(label, color = if (enabled) Color.White else Muted)
    }
}

@Composable
private fun Toggle(label: String, help: String?, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(label)
            if (help != null) Text(help, color = Muted, fontSize = 12.sp)
        }
        Switch(checked = checked, onCheckedChange = onChange)
    }
}

/** "Mon appareil": pseudonym and usage counters from GET /me (never the list of files). */
@Composable
private fun DeviceSection(app: TemoinApp) {
    val server by app.settings.server.collectAsState()
    val s = server ?: return
    var status by remember { mutableStateOf<DeviceStatus?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(s.url) {
        runCatching { withContext(Dispatchers.IO) { Api.me(s.url, app.settings.deviceKey) } }
            .onSuccess { status = it; failure = null }
            .onFailure { failure = (it as? ApiException)?.error ?: "offline" }
    }
    Section(stringResource(R.string.s_device)) {
        Text(stringResource(R.string.s_device_name), color = Muted, fontSize = 12.sp)
        Text(status?.name ?: s.deviceName, fontWeight = FontWeight.SemiBold)
        when (failure) {
            "banned" -> Text(stringResource(R.string.err_banned), color = Accent, fontSize = 13.sp)
            "unknown_device" -> Text(stringResource(R.string.banner_not_registered), color = Accent, fontSize = 13.sp)
            null -> {}
            else -> Text(stringResource(R.string.s_device_offline), color = Muted, fontSize = 12.sp)
        }
        status?.let { d ->
            val frac = if (d.quotaBytes > 0) (d.usedBytes.toFloat() / d.quotaBytes).coerceIn(0f, 1f) else 0f
            LinearProgressIndicator(progress = { frac }, modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp))
            Text(stringResource(R.string.s_device_quota, formatBytes(d.usedBytes), formatBytes(d.quotaBytes)), fontSize = 13.sp)
            Text(stringResource(R.string.s_device_objects, d.objects.toInt()), fontSize = 13.sp)
            Text(
                d.lastUploadAt?.let {
                    val t = runCatching {
                        DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT).withZone(ZoneId.systemDefault()).format(Instant.parse(it))
                    }.getOrDefault(it)
                    stringResource(R.string.s_device_last, t)
                } ?: stringResource(R.string.s_device_never),
                fontSize = 13.sp, color = Muted,
            )
        }
        Text(stringResource(R.string.s_device_key), color = Muted, fontSize = 12.sp)
        Text(app.settings.deviceKey.deviceId.chunked(4).take(4).joinToString(" "), fontFamily = FontFamily.Monospace, fontSize = 13.sp)
        Text(stringResource(R.string.s_device_help), color = Muted, fontSize = 12.sp)
    }
}

private fun shortDate(iso: String?): String? = iso?.let {
    runCatching { DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT).withZone(ZoneId.systemDefault()).format(Instant.parse(it)) }.getOrDefault(it)
}

/**
 * "Récupérer mes fichiers": the server gives a short code that the person passes to the
 * operator by any channel; the operator answers with a one-time link through another one.
 */
@Composable
private fun HandBackSection(app: TemoinApp) {
    val server by app.settings.server.collectAsState()
    val s = server ?: return
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboardManager.current
    var hb by remember { mutableStateOf<HandBack?>(null) }
    var busy by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(s.url) {
        runCatching { withContext(Dispatchers.IO) { Api.me(s.url, app.settings.deviceKey) } }.onSuccess { hb = it.request }
    }
    val ask: () -> Unit = {
        busy = true
        scope.launch {
            runCatching { withContext(Dispatchers.IO) { Api.requestFiles(s.url, app.settings.deviceKey) } }
                .onSuccess { hb = it; failure = null }
                .onFailure { failure = (it as? ApiException)?.error ?: it.message ?: "offline" }
            busy = false
        }
    }
    Section(stringResource(R.string.hb_title)) {
        val current = hb
        if (current == null || current.closed) {
            Text(stringResource(R.string.hb_help), color = Muted, fontSize = 13.sp)
            current?.let { Text(stringResource(linkText(it.link)), fontSize = 13.sp) }
            TextButton(onClick = ask, enabled = !busy) {
                Text(stringResource(if (current == null) R.string.hb_ask else R.string.hb_again))
            }
        } else {
            Text(stringResource(R.string.hb_code), color = Muted, fontSize = 12.sp)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(current.code, fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold, fontSize = 28.sp, modifier = Modifier.weight(1f))
                TextButton(onClick = {
                    clipboard.setText(AnnotatedString(current.code))
                    Toast.makeText(app, R.string.hb_copied, Toast.LENGTH_SHORT).show()
                }) { Text(stringResource(R.string.hb_copy)) }
            }
            Text(stringResource(linkText(current.link)), fontSize = 13.sp)
            Text(stringResource(R.string.hb_how), color = Muted, fontSize = 12.sp)
            shortDate(current.expiresAt)?.let { Text(stringResource(R.string.hb_valid, it), color = Muted, fontSize = 12.sp) }
        }
        failure?.let { Text(stringResource(R.string.hb_failed, it), color = Accent, fontSize = 13.sp) }
    }
}

private fun linkText(link: String?) = when (link) {
    null -> R.string.hb_pending
    "uploading", "ready" -> R.string.hb_ready
    "opened" -> R.string.hb_opened
    "done" -> R.string.hb_done
    "expired" -> R.string.hb_expired
    else -> R.string.hb_revoked
}
