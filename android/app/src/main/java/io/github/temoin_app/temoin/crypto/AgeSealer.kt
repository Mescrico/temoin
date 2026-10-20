package io.github.temoin_app.temoin.crypto

import org.bouncycastle.crypto.digests.SHA256Digest
import org.bouncycastle.crypto.generators.HKDFBytesGenerator
import org.bouncycastle.crypto.macs.HMac
import org.bouncycastle.crypto.modes.ChaCha20Poly1305
import org.bouncycastle.crypto.params.AEADParameters
import org.bouncycastle.crypto.params.HKDFParameters
import org.bouncycastle.crypto.params.KeyParameter
import org.bouncycastle.math.ec.rfc7748.X25519
import java.io.ByteArrayOutputStream
import java.io.OutputStream
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64

/**
 * age v1 encryption to a single X25519 recipient (https://age-encryption.org/v1).
 *
 * This is encrypt-only on purpose: the app never holds an identity, so nothing
 * it writes can be read back on the phone. Mirrors `server/src/seal.rs`.
 */
object AgeSealer {
    private const val CHUNK = 64 * 1024
    private val VERSION_LINE = "age-encryption.org/v1\n".toByteArray()
    private val X25519_INFO = "age-encryption.org/v1/X25519".toByteArray()
    private val b64 = Base64.getEncoder().withoutPadding()
    private val rng = SecureRandom()

    fun seal(recipient: ByteArray, plaintext: ByteArray): ByteArray {
        val out = ByteArrayOutputStream(plaintext.size + plaintext.size / CHUNK * 16 + 256)
        seal(recipient, plaintext, out)
        return out.toByteArray()
    }

    fun seal(recipient: ByteArray, plaintext: ByteArray, out: OutputStream) {
        require(recipient.size == 32) { "recipient must be 32 bytes" }
        val fileKey = random(16)

        // X25519 stanza
        val eph = random(32)
        val ephPub = ByteArray(32).also { X25519.scalarMultBase(eph, 0, it, 0) }
        val shared = ByteArray(32)
        check(X25519.calculateAgreement(eph, 0, recipient, 0, shared, 0)) { "low-order recipient key" }
        val wrapKey = hkdf(ephPub + recipient, shared, X25519_INFO)
        val body = aead(wrapKey, ByteArray(12), fileKey)
        eph.fill(0); shared.fill(0); wrapKey.fill(0)

        val header = ByteArrayOutputStream()
        header.write(VERSION_LINE)
        header.write("-> X25519 ${b64.encodeToString(ephPub)}\n".toByteArray())
        header.write("${b64.encodeToString(body)}\n".toByteArray())
        header.write("---".toByteArray())

        val macKey = hkdf(ByteArray(0), fileKey, "header".toByteArray())
        val mac = HMac(SHA256Digest()).run {
            init(KeyParameter(macKey))
            val h = header.toByteArray()
            update(h, 0, h.size)
            ByteArray(macSize).also { doFinal(it, 0) }
        }
        header.write(" ${b64.encodeToString(mac)}\n".toByteArray())
        out.write(header.toByteArray())

        // STREAM payload
        val nonce = random(16)
        out.write(nonce)
        val payloadKey = hkdf(nonce, fileKey, "payload".toByteArray())
        fileKey.fill(0)

        val chunks = if (plaintext.isEmpty()) 1 else (plaintext.size + CHUNK - 1) / CHUNK
        for (i in 0 until chunks) {
            val from = i * CHUNK
            val to = minOf(from + CHUNK, plaintext.size)
            val n = ByteArray(12)
            var c = i.toLong()
            for (b in 10 downTo 0) { n[b] = (c and 0xff).toByte(); c = c ushr 8 }
            if (i == chunks - 1) n[11] = 1
            out.write(aead(payloadKey, n, plaintext, from, to - from))
        }
        payloadKey.fill(0)
    }

    private fun aead(key: ByteArray, nonce: ByteArray, msg: ByteArray, off: Int = 0, len: Int = msg.size): ByteArray {
        val c = ChaCha20Poly1305()
        c.init(true, AEADParameters(KeyParameter(key), 128, nonce))
        val out = ByteArray(c.getOutputSize(len))
        val n = c.processBytes(msg, off, len, out, 0)
        c.doFinal(out, n)
        return out
    }

    private fun hkdf(salt: ByteArray, ikm: ByteArray, info: ByteArray): ByteArray {
        val g = HKDFBytesGenerator(SHA256Digest())
        g.init(HKDFParameters(ikm, salt, info))
        return ByteArray(32).also { g.generateBytes(it, 0, 32) }
    }

    private fun random(n: Int) = ByteArray(n).also { rng.nextBytes(it) }
}

/** `age1…` recipients are bech32 (BIP-173) with HRP "age" and a 32-byte X25519 key. */
object Recipient {
    private const val CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"

    fun parse(s: String): ByteArray {
        val str = s.trim()
        require(str == str.lowercase()) { "recipient must be lowercase" }
        val sep = str.lastIndexOf('1')
        require(sep > 0 && str.substring(0, sep) == "age") { "not an age1… recipient" }
        val data = str.substring(sep + 1).map { ch ->
            CHARSET.indexOf(ch).also { require(it >= 0) { "invalid bech32 character" } }
        }
        require(data.size >= 6) { "recipient too short" }
        require(polymod(hrpExpand("age") + data) == 1) { "recipient checksum mismatch" }
        val bytes = convertBits(data.dropLast(6), 5, 8)
        require(bytes.size == 32) { "recipient must encode 32 bytes" }
        return bytes
    }

    fun isValid(s: String) = runCatching { parse(s) }.isSuccess

    /** Same as `sr fingerprint`: first 10 bytes of SHA-256(recipient), hex, groups of 4. */
    fun fingerprint(s: String): String {
        val d = MessageDigest.getInstance("SHA-256").digest(s.trim().toByteArray())
        return d.copyOf(10).joinToString("") { "%02X".format(it) }.chunked(4).joinToString(" ")
    }

    private fun hrpExpand(hrp: String) =
        hrp.map { it.code shr 5 } + 0 + hrp.map { it.code and 31 }

    private fun polymod(values: List<Int>): Int {
        val gen = intArrayOf(0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3)
        var chk = 1
        for (v in values) {
            val top = chk ushr 25
            chk = (chk and 0x1ffffff) shl 5 xor v
            for (i in 0 until 5) if ((top ushr i) and 1 == 1) chk = chk xor gen[i]
        }
        return chk
    }

    private fun convertBits(data: List<Int>, from: Int, to: Int): ByteArray {
        var acc = 0
        var bits = 0
        val out = ByteArrayOutputStream()
        val maxv = (1 shl to) - 1
        for (v in data) {
            acc = (acc shl from) or v
            bits += from
            while (bits >= to) {
                bits -= to
                out.write((acc ushr bits) and maxv)
            }
        }
        require(bits < from && ((acc shl (to - bits)) and maxv) == 0) { "invalid padding" }
        return out.toByteArray()
    }
}
