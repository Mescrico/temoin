package io.github.temoin_app.temoin.data

import android.content.Context
import android.content.SharedPreferences
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import io.github.temoin_app.temoin.crypto.DeviceKey
import java.io.File

/** The enrolled server. The recipient is pinned at enrollment and never silently replaced. */
data class ServerConfig(
    val url: String,
    val recipient: String,
    val name: String,
    /** Pseudonym given by the server at registration ("Phone #37"). */
    val deviceName: String,
    val limits: ServerLimits,
)

data class ServerLimits(
    val maxObjectBytes: Long = 16L * 1024 * 1024,
    val maxVideoHeight: Int = 1080,
    val defaultVideoHeight: Int = 720,
    val videoBitrate720: Int = 2_500_000,
    val videoBitrate1080: Int = 5_000_000,
    val photoMaxPixels: Int = 4_000_000,
)

data class UserPrefs(
    val videoHeight: Int = 720,
    val attachLocation: Boolean = false,
    val wifiOnly: Boolean = false,
    val cameraId: String? = null,
    val micDeviceId: Int? = null,
)

class Settings(context: Context) {
    private val keyFile = File(context.noBackupFilesDir, "device.key")
    private val prefs: SharedPreferences = context.getSharedPreferences("temoin", Context.MODE_PRIVATE)

    private val _server = MutableStateFlow(loadServer())
    val server: StateFlow<ServerConfig?> = _server.asStateFlow()

    private val _user = MutableStateFlow(loadUser())
    val user: StateFlow<UserPrefs> = _user.asStateFlow()

    /** This phone's Ed25519 identity on the current server (one key per server joined). */
    @Volatile var deviceKey: DeviceKey = DeviceKey.loadOrCreate(keyFile)
        private set

    /** Key to use for joining a server: the current one for the same server (same pinned key,
     *  even at a new address: a home server behind a quick tunnel changes address on restart),
     *  a fresh one otherwise (so two servers cannot link the same phone). Stored only once
     *  registration succeeds. */
    @Suppress("UNUSED_PARAMETER")
    fun keyFor(url: String, recipient: String): DeviceKey {
        val s = _server.value
        return if (s != null && s.recipient == recipient) deviceKey else DeviceKey.generate()
    }

    fun adoptDeviceKey(k: DeviceKey) {
        k.save(keyFile)
        deviceKey = k
    }

    fun saveServer(s: ServerConfig) {
        prefs.edit {
            putString("server_url", s.url)
            putString("server_recipient", s.recipient)
            putString("server_name", s.name)
            putString("server_device_name", s.deviceName)
            putLong("lim_max_object", s.limits.maxObjectBytes)
            putInt("lim_max_height", s.limits.maxVideoHeight)
            putInt("lim_default_height", s.limits.defaultVideoHeight)
            putInt("lim_br720", s.limits.videoBitrate720)
            putInt("lim_br1080", s.limits.videoBitrate1080)
            putInt("lim_photo_px", s.limits.photoMaxPixels)
        }
        _server.value = s
        // Never let the saved quality exceed what the server accepts.
        if (_user.value.videoHeight > s.limits.maxVideoHeight) updateUser { it.copy(videoHeight = s.limits.maxVideoHeight) }
    }

    /** Refreshes limits from /info. The recipient is NOT updated here on purpose. */
    fun updateLimits(limits: ServerLimits) {
        _server.value?.let { saveServer(it.copy(limits = limits)) }
    }

    fun forgetServer() {
        prefs.edit {
            listOf("server_url", "server_recipient", "server_name", "server_device_name").forEach { remove(it) }
        }
        _server.value = null
    }

    fun updateUser(f: (UserPrefs) -> UserPrefs) {
        val u = f(_user.value)
        prefs.edit {
            putInt("video_height", u.videoHeight)
            putBoolean("attach_location", u.attachLocation)
            putBoolean("wifi_only", u.wifiOnly)
            if (u.cameraId != null) putString("camera_id", u.cameraId) else remove("camera_id")
            if (u.micDeviceId != null) putInt("mic_id", u.micDeviceId) else remove("mic_id")
        }
        _user.value = u
    }

    private fun loadServer(): ServerConfig? {
        val url = prefs.getString("server_url", null) ?: return null
        val recipient = prefs.getString("server_recipient", null) ?: return null
        val d = ServerLimits()
        return ServerConfig(
            url = url,
            recipient = recipient,
            name = prefs.getString("server_name", null) ?: url,
            deviceName = prefs.getString("server_device_name", null) ?: "",
            limits = ServerLimits(
                maxObjectBytes = prefs.getLong("lim_max_object", d.maxObjectBytes),
                maxVideoHeight = prefs.getInt("lim_max_height", d.maxVideoHeight),
                defaultVideoHeight = prefs.getInt("lim_default_height", d.defaultVideoHeight),
                videoBitrate720 = prefs.getInt("lim_br720", d.videoBitrate720),
                videoBitrate1080 = prefs.getInt("lim_br1080", d.videoBitrate1080),
                photoMaxPixels = prefs.getInt("lim_photo_px", d.photoMaxPixels),
            ),
        )
    }

    private fun loadUser() = UserPrefs(
        videoHeight = prefs.getInt("video_height", 720),
        attachLocation = prefs.getBoolean("attach_location", false),
        wifiOnly = prefs.getBoolean("wifi_only", false),
        cameraId = prefs.getString("camera_id", null),
        micDeviceId = if (prefs.contains("mic_id")) prefs.getInt("mic_id", 0) else null,
    )
}
