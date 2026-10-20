package io.github.temoin_app.temoin.crypto

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer

/**
 * Plaintext carried inside every age object: `"SRP1" | u32 BE length | JSON header | content`.
 * See docs/PROTOCOL.md §2. All metadata lives here, i.e. inside the encryption.
 */
object Envelope {
    private val MAGIC = "SRP1".toByteArray()

    fun encode(header: Map<String, Any?>, content: ByteArray): ByteArray {
        val json = Json.write(header).toByteArray(Charsets.UTF_8)
        val out = ByteArrayOutputStream(8 + json.size + content.size)
        out.write(MAGIC)
        out.write(ByteBuffer.allocate(4).putInt(json.size).array())
        out.write(json)
        out.write(content)
        return out.toByteArray()
    }
}

/** Tiny JSON writer (maps, lists, strings, numbers, booleans, null) — enough for envelope headers. */
object Json {
    fun write(v: Any?): String = StringBuilder().also { append(it, v) }.toString()

    private fun append(sb: StringBuilder, v: Any?) {
        when (v) {
            null -> sb.append("null")
            is String -> string(sb, v)
            is Boolean -> sb.append(v)
            is Int, is Long, is Short, is Byte -> sb.append(v.toString())
            is Float, is Double -> {
                val d = (v as Number).toDouble()
                require(d.isFinite()) { "non-finite number" }
                sb.append(d.toString())
            }
            is Map<*, *> -> {
                sb.append('{')
                var first = true
                for ((k, value) in v) {
                    if (value == null) continue
                    if (!first) sb.append(',')
                    first = false
                    string(sb, k.toString())
                    sb.append(':')
                    append(sb, value)
                }
                sb.append('}')
            }
            is Iterable<*> -> {
                sb.append('[')
                v.forEachIndexed { i, e -> if (i > 0) sb.append(','); append(sb, e) }
                sb.append(']')
            }
            else -> string(sb, v.toString())
        }
    }

    private fun string(sb: StringBuilder, s: String) {
        sb.append('"')
        for (c in s) {
            when {
                c == '"' -> sb.append("\\\"")
                c == '\\' -> sb.append("\\\\")
                c == '\n' -> sb.append("\\n")
                c == '\r' -> sb.append("\\r")
                c == '\t' -> sb.append("\\t")
                c < ' ' -> sb.append("\\u%04x".format(c.code))
                else -> sb.append(c)
            }
        }
        sb.append('"')
    }
}
