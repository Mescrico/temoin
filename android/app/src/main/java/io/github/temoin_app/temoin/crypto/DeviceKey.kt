package io.github.temoin_app.temoin.crypto

import org.bouncycastle.math.ec.rfc8032.Ed25519
import java.io.File
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64

/**
 * This phone's identity on the enrolled server: an Ed25519 key pair generated on
 * the phone. The server only ever sees the public key (and derives a pseudonymous
 * device id from it). A new key is generated for every server joined, so two
 * servers cannot link the same phone.
 */
class DeviceKey private constructor(private val secret: ByteArray) {
    val publicKey: ByteArray = ByteArray(Ed25519.PUBLIC_KEY_SIZE).also { Ed25519.generatePublicKey(secret, 0, it, 0) }
    val publicKeyB64: String = Base64.getEncoder().encodeToString(publicKey)
    val deviceId: String = deviceId(publicKey)

    /** Atomically stores the secret key in [file] (no-backup storage). */
    fun save(file: File) {
        val tmp = File(file.path + ".tmp")
        tmp.writeBytes(secret)
        check(tmp.renameTo(file)) { "cannot store device key" }
    }

    /** Base64 Ed25519 signature of [message] (UTF-8). */
    fun sign(message: String): String {
        val m = message.toByteArray(Charsets.UTF_8)
        val sig = ByteArray(Ed25519.SIGNATURE_SIZE)
        Ed25519.sign(secret, 0, m, 0, m.size, sig, 0)
        return Base64.getEncoder().encodeToString(sig)
    }

    companion object {
        /** First 16 bytes of SHA-256(public key), lowercase hex — same as the server. */
        fun deviceId(publicKey: ByteArray): String =
            MessageDigest.getInstance("SHA-256").digest(publicKey).copyOf(16).joinToString("") { "%02x".format(it) }

        fun generate(): DeviceKey = DeviceKey(ByteArray(Ed25519.SECRET_KEY_SIZE).also { Ed25519.generatePrivateKey(SecureRandom(), it) })

        fun fromSecret(secret: ByteArray): DeviceKey = DeviceKey(secret.copyOf())

        /** Loads the key stored in [file], creating it if needed. The file lives in no-backup storage. */
        fun loadOrCreate(file: File): DeviceKey {
            if (file.length() == Ed25519.SECRET_KEY_SIZE.toLong()) return DeviceKey(file.readBytes())
            return generate().also { it.save(file) }
        }

        /** String signed for every authenticated request (PROTOCOL.md §4). */
        fun requestMessage(method: String, path: String, bodySha256: String?, timestamp: Long, deviceId: String) =
            "SRAUTH1\n$method\n$path\n${bodySha256 ?: "-"}\n$timestamp\n$deviceId"

        /** String signed when registering with an invitation. */
        fun registerMessage(invite: String, publicKeyB64: String, timestamp: Long) =
            "SRREG1\n$invite\n$publicKeyB64\n$timestamp"
    }
}
