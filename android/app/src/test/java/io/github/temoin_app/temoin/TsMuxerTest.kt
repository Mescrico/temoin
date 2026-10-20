package io.github.temoin_app.temoin

import io.github.temoin_app.temoin.capture.TsMuxer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.File

/**
 * Feeds real H.264 (Annex-B) and AAC (ADTS) produced by ffmpeg through the muxer,
 * cutting segments on key frames like the recorder does. Output lands in
 * build/ts/ — `tools/ts-check.sh` validates it with ffprobe.
 */
class TsMuxerTest {
    private fun res(name: String) = javaClass.classLoader!!.getResourceAsStream(name)!!.readBytes()

    /** Splits Annex-B into access units on AUD (type 9) NALs; returns AU payloads without the AUD. */
    private fun accessUnits(h264: ByteArray): List<ByteArray> {
        val starts = mutableListOf<Int>()
        var i = 0
        while (i + 3 < h264.size) {
            if (h264[i].toInt() == 0 && h264[i + 1].toInt() == 0 && h264[i + 2].toInt() == 1) {
                val s = if (i > 0 && h264[i - 1].toInt() == 0) i - 1 else i
                starts += s; i += 3
            } else i++
        }
        val nals = starts.mapIndexed { k, s -> h264.copyOfRange(s, if (k + 1 < starts.size) starts[k + 1] else h264.size) }
        val aus = mutableListOf<ByteArray>()
        var cur = ByteArrayOutputStream()
        for (n in nals) {
            val off = if (n[2].toInt() == 1) 3 else 4
            if ((n[off].toInt() and 0x1F) == 9) {
                if (cur.size() > 0) aus += cur.toByteArray()
                cur = ByteArrayOutputStream()
            } else cur.write(n)
        }
        if (cur.size() > 0) aus += cur.toByteArray()
        return aus
    }

    private fun isKey(au: ByteArray): Boolean {
        var i = 0
        while (i + 3 < au.size) {
            if (au[i].toInt() == 0 && au[i + 1].toInt() == 0 && au[i + 2].toInt() == 1 && (au[i + 3].toInt() and 0x1F) == 5) return true
            i++
        }
        return false
    }

    private fun adtsFrames(aac: ByteArray): List<ByteArray> {
        val out = mutableListOf<ByteArray>()
        var p = 0
        while (p + 7 <= aac.size) {
            val len = ((aac[p + 3].toInt() and 3) shl 11) or ((aac[p + 4].toInt() and 0xFF) shl 3) or ((aac[p + 5].toInt() and 0xFF) ushr 5)
            out += aac.copyOfRange(p + 7, p + len) // raw frame; the muxer side re-adds the header
            p += len
        }
        return out
    }

    @Test
    fun muxesSegmentsThatConcatenate() {
        val video = accessUnits(res("sample.h264"))
        val audio = adtsFrames(res("sample.aac"))
        assertEquals(90, video.size)
        assertTrue(isKey(video[0]))

        val mux = TsMuxer(hasAudio = true)
        mux.cut()
        val segments = mutableListOf<ByteArray>()
        var ai = 0
        for ((n, au) in video.withIndex()) {
            val pts = 90_000L + n * 3000L // 30 fps
            if (n > 0 && isKey(au)) segments += mux.cut()
            mux.writeVideo(au, pts, isKey(au))
            // interleave audio up to the video time (1024 samples @ 48 kHz = 1920 ticks)
            while (ai < audio.size && 90_000L + ai * 1920L <= pts) {
                mux.writeAudio(TsMuxer.adtsHeader(audio[ai].size, 48_000, 1) + audio[ai], 90_000L + ai * 1920L)
                ai++
            }
        }
        segments += mux.cut()
        assertEquals(3, segments.size)
        segments.forEach { assertEquals(0, it.size % 188) }

        val dir = File("build/ts").apply { deleteRecursively(); mkdirs() }
        segments.forEachIndexed { i, s -> File(dir, "seg$i.ts").writeBytes(s) }
        File(dir, "all.ts").writeBytes(segments.reduce { a, b -> a + b })
    }

    @Test
    fun crcMatchesMpeg2() {
        // CRC-32/MPEG-2 of "123456789" is 0x0376E6E7.
        assertEquals(0x0376E6E7, TsMuxer.crc32("123456789".toByteArray()))
    }
}
