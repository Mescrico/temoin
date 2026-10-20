package io.github.temoin_app.temoin.net

import io.github.temoin_app.temoin.crypto.DeviceKey
import io.github.temoin_app.temoin.data.ServerLimits
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import java.net.URLDecoder
import java.security.MessageDigest

data class ServerInfo(
    val name: String,
    val recipient: String,
    val accepting: Boolean,
    val protocol: Int,
    val limits: ServerLimits,
)

/** Parsed `srec://enroll?u=…&k=…&i=…&n=…` link from an invitation QR code. */
data class EnrollLink(val url: String, val recipient: String, val invite: String?, val name: String?) {
    companion object {
        fun parse(raw: String): EnrollLink? {
            val s = raw.trim()
            if (!s.startsWith("srec://enroll?")) return null
            val params = s.substringAfter('?').split('&').mapNotNull {
                val i = it.indexOf('=')
                if (i <= 0) null else it.substring(0, i) to URLDecoder.decode(it.substring(i + 1), "UTF-8")
            }.toMap()
            val url = params["u"]?.trimEnd('/') ?: return null
            val k = params["k"] ?: return null
            return EnrollLink(url, k, params["i"]?.ifBlank { null }, params["n"])
        }
    }
}

/** Result of joining a server with an invitation. */
sealed class RegisterResult {
    data class Ok(val name: String, val quotaBytes: Long) : RegisterResult()
    /** [error] = server error code: unknown_invite, invite_expired, invite_revoked, invite_exhausted, banned… */
    data class Failed(val error: String, val message: String) : RegisterResult()
}

data class DeviceStatus(
    val name: String,
    val quotaBytes: Long,
    val usedBytes: Long,
    val objects: Long,
    val lastUploadAt: String?,
    val accepting: Boolean,
    val request: HandBack? = null,
)

/**
 * "Get my files back": [code] is passed to the operator by any channel; [link] is the
 * state of the one-time link they create (null = not yet, ready, opened, done, expired, revoked).
 */
data class HandBack(val code: String, val expiresAt: String?, val link: String?) {
    /** The link is gone: asking again gives a new code. */
    val closed get() = link in setOf("done", "expired", "revoked")

    companion object {
        fun parse(j: JSONObject?): HandBack? {
            if (j == null) return null
            val code = j.optString("code").ifBlank { return null }
            fun str(k: String) = if (j.isNull(k)) null else j.optString(k)
            return HandBack(code, str("expires_at"), str("link"))
        }
    }
}

sealed class UploadResult {
    data object Stored : UploadResult()
    /** Retry later (network, 5xx, paused, quota, banned — a ban may be lifted). */
    data class Retry(val reason: String, val paused: Boolean = false, val error: String? = null) : UploadResult()
    /** Server will never accept this object; keep it but stop retrying it in this pass. */
    data class Rejected(val code: Int, val reason: String) : UploadResult()
}

object Api {
    private const val TIMEOUT_MS = 20_000

    fun normalizeUrl(input: String): String {
        var u = input.trim().trimEnd('/')
        if (!u.startsWith("http://") && !u.startsWith("https://")) u = "https://$u"
        URI(u).toURL() // validates
        return u
    }

    /** Invitation codes are shown in groups of 5 for reading; strip spaces/dashes. */
    fun normalizeInvite(code: String) = code.filter { it.isLetterOrDigit() }.uppercase()

    /**
     * Server description. A discreet server only answers a phone that shows an
     * invitation code ([invite]) or signs with its registered device [key].
     */
    @Throws(IOException::class)
    fun info(baseUrl: String, invite: String? = null, key: DeviceKey? = null): ServerInfo {
        val c = open("$baseUrl/api/v1/info", "GET")
        invite?.let { c.setRequestProperty("X-SR-Invite", normalizeInvite(it)) }
        key?.let { sign(c, it, "GET", "/api/v1/info", null) }
        try {
            if (c.responseCode != 200) throw ApiException(c.responseCode, "HTTP ${c.responseCode}")
            val j = JSONObject(c.inputStream.bufferedReader().readText())
            if (j.optString("server") != "temoin") throw IOException("not a Témoin server")
            val l = j.optJSONObject("limits") ?: JSONObject()
            val d = ServerLimits()
            return ServerInfo(
                name = j.optString("name", baseUrl),
                recipient = j.getString("recipient"),
                accepting = j.optBoolean("accepting", true),
                protocol = j.optInt("protocol", 1),
                limits = ServerLimits(
                    maxObjectBytes = l.optLong("max_object_bytes", d.maxObjectBytes),
                    maxVideoHeight = l.optInt("max_video_height", d.maxVideoHeight),
                    defaultVideoHeight = l.optInt("default_video_height", d.defaultVideoHeight),
                    videoBitrate720 = l.optInt("video_bitrate_720", d.videoBitrate720),
                    videoBitrate1080 = l.optInt("video_bitrate_1080", d.videoBitrate1080),
                    photoMaxPixels = l.optInt("photo_max_pixels", d.photoMaxPixels),
                ),
            )
        } finally {
            c.disconnect()
        }
    }

    /** Registers this phone's public key with an invitation code. */
    fun register(baseUrl: String, key: DeviceKey, invite: String): RegisterResult {
        val code = normalizeInvite(invite)
        val ts = System.currentTimeMillis() / 1000
        val body = JSONObject()
            .put("invite", code)
            .put("public_key", key.publicKeyB64)
            .put("timestamp", ts)
            .put("signature", key.sign(DeviceKey.registerMessage(code, key.publicKeyB64, ts)))
            .toString().toByteArray()
        val c = try {
            open("$baseUrl/api/v1/devices", "POST").apply {
                doOutput = true
                setRequestProperty("Content-Type", "application/json")
                outputStream.use { it.write(body) }
            }
        } catch (e: IOException) {
            return RegisterResult.Failed("network", e.message ?: "network error")
        }
        return try {
            val status = c.responseCode
            val j = readJson(c, status)
            if (status == 200 && j != null) RegisterResult.Ok(j.optString("name"), j.optLong("quota_bytes"))
            else RegisterResult.Failed(j?.optString("error") ?: "HTTP $status", j?.optString("message") ?: "HTTP $status")
        } catch (e: IOException) {
            RegisterResult.Failed("network", e.message ?: "network error")
        } finally {
            c.disconnect()
        }
    }

    /** Usage counters of this device. Throws [ApiException] with the server error code on refusal. */
    @Throws(IOException::class)
    fun me(baseUrl: String, key: DeviceKey): DeviceStatus {
        val path = "/api/v1/me"
        val c = open(baseUrl + path, "GET")
        sign(c, key, "GET", path, null)
        try {
            val status = c.responseCode
            val j = readJson(c, status)
            if (status != 200 || j == null) throw ApiException(status, j?.optString("error") ?: "HTTP $status")
            return DeviceStatus(
                name = j.optString("name"),
                quotaBytes = j.optLong("quota_bytes"),
                usedBytes = j.optLong("used_bytes"),
                objects = j.optLong("objects"),
                lastUploadAt = if (j.isNull("last_upload_at")) null else j.optString("last_upload_at"),
                accepting = j.optBoolean("accepting", true),
                request = HandBack.parse(j.optJSONObject("request")),
            )
        } finally {
            c.disconnect()
        }
    }

    /** Asks for this device's recordings back: returns the code to pass to the operator. */
    @Throws(IOException::class)
    fun requestFiles(baseUrl: String, key: DeviceKey): HandBack {
        val path = "/api/v1/me/request"
        val c = open(baseUrl + path, "POST")
        sign(c, key, "POST", path, null)
        c.doOutput = true
        c.setFixedLengthStreamingMode(0)
        try {
            c.outputStream.close()
            val status = c.responseCode
            val j = readJson(c, status)
            if (status != 200 || j == null) throw ApiException(status, j?.optString("error") ?: "HTTP $status")
            return HandBack.parse(j) ?: throw IOException("bad answer")
        } finally {
            c.disconnect()
        }
    }

    fun put(baseUrl: String, key: DeviceKey, session: String, seq: Int, file: File): UploadResult {
        val body = file.readBytes()
        val sha = MessageDigest.getInstance("SHA-256").digest(body).joinToString("") { "%02x".format(it) }
        val path = "/api/v1/objects/$session/$seq"
        val c = try {
            open(baseUrl + path, "PUT").apply {
                doOutput = true
                setFixedLengthStreamingMode(body.size)
                setRequestProperty("Content-Type", "application/octet-stream")
                setRequestProperty("X-SR-Content-SHA256", sha)
                sign(this, key, "PUT", path, sha)
                outputStream.use { it.write(body) }
            }
        } catch (e: IOException) {
            return UploadResult.Retry(e.message ?: "network error")
        }
        return try {
            val code = c.responseCode
            val json = readJson(c, code)
            val error = json?.optString("error")
            val msg = json?.optString("message") ?: "HTTP $code"
            when {
                code == 200 || code == 201 ->
                    // Only trust the ack if the server hashed exactly what we sent.
                    if (json?.optString("sha256") == sha) UploadResult.Stored
                    else UploadResult.Retry("ack hash mismatch")
                code == 503 -> UploadResult.Retry(msg, paused = true, error = error)
                // Banned, unknown device, clock skew: keep the object — it can still be sent later.
                code == 401 || code == 403 -> UploadResult.Retry(msg, paused = true, error = error)
                code == 429 || code >= 500 -> UploadResult.Retry(msg, error = error)
                else -> UploadResult.Rejected(code, msg)
            }
        } catch (e: IOException) {
            UploadResult.Retry(e.message ?: "network error")
        } finally {
            c.disconnect()
        }
    }

    private fun sign(c: HttpURLConnection, key: DeviceKey, method: String, path: String, sha: String?) {
        val ts = System.currentTimeMillis() / 1000
        c.setRequestProperty("X-SR-Device", key.deviceId)
        c.setRequestProperty("X-SR-Timestamp", ts.toString())
        c.setRequestProperty("X-SR-Signature", key.sign(DeviceKey.requestMessage(method, path, sha, ts, key.deviceId)))
    }

    private fun readJson(c: HttpURLConnection, status: Int): JSONObject? {
        val text = (if (status < 400) c.inputStream else c.errorStream)?.bufferedReader()?.readText().orEmpty()
        return runCatching { JSONObject(text) }.getOrNull()
    }

    private fun open(url: String, method: String): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = method
            connectTimeout = TIMEOUT_MS
            readTimeout = TIMEOUT_MS * 3
            instanceFollowRedirects = false
            useCaches = false
            setRequestProperty("User-Agent", "Temoin-Android")
        }
}

class ApiException(val status: Int, val error: String) : IOException("$status $error")
