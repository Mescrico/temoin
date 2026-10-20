package io.github.temoin_app.temoin

import io.github.temoin_app.temoin.crypto.AgeSealer
import io.github.temoin_app.temoin.crypto.DeviceKey
import io.github.temoin_app.temoin.crypto.Json
import io.github.temoin_app.temoin.crypto.Envelope
import io.github.temoin_app.temoin.crypto.Recipient
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Interop vectors: writes objects sealed to the committed TEST recipient into
 * build/interop/. `tools/interop-check.sh` then decrypts them with the reference
 * `age` CLI and the server's `sr decrypt` (in Docker) to prove compatibility.
 */
class CryptoTest {
    // Test-only key pair. The identity lives in tools/testdata/identity.txt.
    private val testRecipient = "age1vy0mm5yzn08fjcur7l7a58evfd5rptw5035c67ur28uz0p4yxpts788dz0"

    @Test
    fun parsesRecipientAndFingerprint() {
        assertEquals(32, Recipient.parse(testRecipient).size)
        assertFalse(Recipient.isValid(testRecipient.dropLast(1) + "q"))
        assertFalse(Recipient.isValid("AGE-SECRET-KEY-1QQQ"))
        assertTrue(Recipient.fingerprint(testRecipient).matches(Regex("[0-9A-F]{4}( [0-9A-F]{4}){4}")))
    }

    @Test
    fun writesInteropVectors() {
        val dir = File("build/interop").apply { deleteRecursively(); mkdirs() }
        val key = Recipient.parse(testRecipient)
        val sizes = listOf(1, 1000, 65536, 65537, 3 * 65536, 300_001)
        for ((i, size) in sizes.withIndex()) {
            val content = ByteArray(size) { ((it * 31) % 251).toByte() }
            val header = mapOf(
                "v" to 1, "session" to "interop_android_$i", "seq" to 0, "kind" to "photo",
                "container" to "jpeg", "final" to true, "created_at" to "2026-10-19T12:00:00Z",
                "device" to mapOf("platform" to "android-test", "model" to "jvm \"quoted\""),
            )
            val sealed = AgeSealer.seal(key, Envelope.encode(header, content))
            File(dir, "interop_android_$i.age").writeBytes(sealed)
            File(dir, "interop_android_$i.bin").writeBytes(content)
            assertTrue(String(sealed, 0, 22) == "age-encryption.org/v1\n")
        }
    }

    @Test
    fun writesDeviceSignatureVectors() {
        val dir = File("build/interop").apply { mkdirs() }
        val key = DeviceKey.generate()
        assertEquals(32, key.deviceId.length)
        val msgs = listOf(
            "req" to DeviceKey.requestMessage("PUT", "/api/v1/objects/abc12345/0", "ab".repeat(32), 1792412345, key.deviceId),
            "me" to DeviceKey.requestMessage("GET", "/api/v1/me", null, 1792412345, key.deviceId),
            "reg" to DeviceKey.registerMessage("K7QM2ABCDEFGHJKLMNPQ", key.publicKeyB64, 1792412345),
        )
        for ((name, msg) in msgs) {
            val v = mapOf("public_key" to key.publicKeyB64, "device_id" to key.deviceId, "message" to msg, "signature" to key.sign(msg))
            File(dir, "sig_android_$name.json").writeText(Json.write(v))
        }
        // persistence roundtrip keeps the identity
        val f = File(dir, "device.key")
        key.save(f)
        assertEquals(key.deviceId, DeviceKey.loadOrCreate(f).deviceId)
    }
}
