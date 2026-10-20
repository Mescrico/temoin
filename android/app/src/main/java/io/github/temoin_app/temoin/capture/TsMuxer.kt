package io.github.temoin_app.temoin.capture

import java.io.ByteArrayOutputStream

/**
 * Minimal MPEG-TS muxer for one H.264 video stream and one AAC (ADTS) audio stream,
 * writing to memory only.
 *
 * Why TS: Android's MediaMuxer can only write MP4 to a seekable *file*, which would
 * put plaintext video on flash storage. TS is a byte stream: we keep it in RAM,
 * cut it into segments, and seal each one before anything reaches the disk.
 * Continuity counters and timestamps run across segments, so the decrypted
 * segments concatenated byte-for-byte form one valid stream.
 */
class TsMuxer(private val hasAudio: Boolean) {
    private var out = ByteArrayOutputStream(1 shl 21)
    private val cc = IntArray(0x2000)

    /** Starts a new in-memory segment beginning with PAT/PMT. Returns the previous segment's bytes. */
    fun cut(): ByteArray {
        val prev = out.toByteArray()
        out = ByteArrayOutputStream(maxOf(prev.size + (prev.size shr 2), 1 shl 20))
        writePsi()
        return prev
    }

    fun size() = out.size()

    /** [annexB] = one access unit in Annex-B format (SPS/PPS already prepended on key frames). */
    fun writeVideo(annexB: ByteArray, pts90k: Long, key: Boolean) {
        // Access unit delimiter first, as required for H.264 in TS.
        val au = ByteArray(6 + annexB.size)
        byteArrayOf(0, 0, 0, 1, 0x09, 0xF0.toByte()).copyInto(au)
        annexB.copyInto(au, 6)
        writePes(VIDEO_PID, 0xE0, au, pts90k, pcr = true, randomAccess = key, boundedLength = false)
    }

    /** [adtsFrame] = one AAC frame including its 7-byte ADTS header. */
    fun writeAudio(adtsFrame: ByteArray, pts90k: Long) {
        writePes(AUDIO_PID, 0xC0, adtsFrame, pts90k, pcr = false, randomAccess = true, boundedLength = true)
    }

    private fun writePsi() {
        // PAT: program 1 -> PMT PID
        val pat = byteArrayOf(
            0x00, 0x01, 0xC1.toByte(), 0x00, 0x00, // transport_stream_id, version, section numbers
            0x00, 0x01, (0xE0 or (PMT_PID shr 8)).toByte(), (PMT_PID and 0xFF).toByte(),
        )
        writeSection(PAT_PID, 0x00, pat)

        val streams = ByteArrayOutputStream()
        streams.write(byteArrayOf(0x1B, (0xE0 or (VIDEO_PID shr 8)).toByte(), (VIDEO_PID and 0xFF).toByte(), 0xF0.toByte(), 0x00))
        if (hasAudio) streams.write(byteArrayOf(0x0F, (0xE0 or (AUDIO_PID shr 8)).toByte(), (AUDIO_PID and 0xFF).toByte(), 0xF0.toByte(), 0x00))
        val pmt = byteArrayOf(
            0x00, 0x01, 0xC1.toByte(), 0x00, 0x00,
            (0xE0 or (VIDEO_PID shr 8)).toByte(), (VIDEO_PID and 0xFF).toByte(), // PCR PID
            0xF0.toByte(), 0x00, // no program descriptors
        ) + streams.toByteArray()
        writeSection(PMT_PID, 0x02, pmt)
    }

    /** [body] = section bytes after section_length (table-id specific fields), CRC appended here. */
    private fun writeSection(pid: Int, tableId: Int, body: ByteArray) {
        val len = body.size + 4 // + CRC
        val sec = ByteArray(3 + body.size)
        sec[0] = tableId.toByte()
        sec[1] = (0xB0 or (len shr 8)).toByte()
        sec[2] = (len and 0xFF).toByte()
        body.copyInto(sec, 3)
        val crc = crc32(sec)
        val payload = ByteArray(1 + sec.size + 4) // pointer_field + section + crc
        sec.copyInto(payload, 1)
        payload[payload.size - 4] = (crc ushr 24).toByte()
        payload[payload.size - 3] = (crc ushr 16).toByte()
        payload[payload.size - 2] = (crc ushr 8).toByte()
        payload[payload.size - 1] = crc.toByte()
        val pkt = ByteArray(188) { 0xFF.toByte() }
        pkt[0] = 0x47
        pkt[1] = (0x40 or (pid shr 8)).toByte()
        pkt[2] = (pid and 0xFF).toByte()
        pkt[3] = (0x10 or nextCc(pid)).toByte()
        payload.copyInto(pkt, 4)
        out.write(pkt)
    }

    private fun writePes(pid: Int, streamId: Int, data: ByteArray, pts90k: Long, pcr: Boolean, randomAccess: Boolean, boundedLength: Boolean) {
        val header = ByteArray(14)
        header[0] = 0; header[1] = 0; header[2] = 1
        header[3] = streamId.toByte()
        val pesLen = if (boundedLength) data.size + 8 else 0
        header[4] = (pesLen shr 8).toByte(); header[5] = pesLen.toByte()
        header[6] = 0x84.toByte() // marker '10', data_alignment_indicator
        header[7] = 0x80.toByte() // PTS only
        header[8] = 5
        writeTimestamp(header, 9, 0x2, pts90k)
        val pes = header + data

        var pos = 0
        var first = true
        while (pos < pes.size) {
            val pkt = ByteArray(188)
            pkt[0] = 0x47
            pkt[1] = ((if (first) 0x40 else 0) or (pid shr 8)).toByte()
            pkt[2] = (pid and 0xFF).toByte()

            // Adaptation field: PCR / random access flag on the first packet, stuffing on the last.
            var af: ByteArray? = null
            if (first && (pcr || randomAccess)) {
                af = if (pcr) {
                    val a = ByteArray(8)
                    a[1] = ((if (randomAccess) 0x40 else 0) or 0x10).toByte()
                    val base = pts90k - PCR_LEAD_90K
                    a[2] = (base shr 25).toByte(); a[3] = (base shr 17).toByte()
                    a[4] = (base shr 9).toByte(); a[5] = (base shr 1).toByte()
                    a[6] = (((base and 1) shl 7) or 0x7E).toByte(); a[7] = 0
                    a
                } else byteArrayOf(0, 0x40)
            }
            var afLen = af?.size ?: 0
            val remaining = pes.size - pos
            var room = 184 - afLen
            if (remaining < room) {
                // Need stuffing: grow (or create) the adaptation field.
                val stuff = room - remaining
                af = if (af == null) {
                    if (stuff == 1) byteArrayOf(0) else ByteArray(stuff).also { it[1] = 0; it.fill(0xFF.toByte(), 2) }
                } else af + ByteArray(stuff) { 0xFF.toByte() }
                afLen = af.size
                room = 184 - afLen
            }
            if (af != null) {
                af[0] = (afLen - 1).toByte()
                af.copyInto(pkt, 4)
            }
            pkt[3] = ((if (af != null) 0x30 else 0x10) or nextCc(pid)).toByte()
            val n = minOf(room, remaining)
            pes.copyInto(pkt, 4 + afLen, pos, pos + n)
            pos += n
            first = false
            out.write(pkt)
        }
    }

    private fun writeTimestamp(b: ByteArray, off: Int, prefix: Int, ts: Long) {
        b[off] = ((prefix shl 4) or (((ts shr 30) and 0x07).toInt() shl 1) or 1).toByte()
        b[off + 1] = (ts shr 22).toByte()
        b[off + 2] = ((((ts shr 15) and 0x7F).toInt() shl 1) or 1).toByte()
        b[off + 3] = (ts shr 7).toByte()
        b[off + 4] = (((ts and 0x7F).toInt() shl 1) or 1).toByte()
    }

    private fun nextCc(pid: Int): Int {
        val v = cc[pid]
        cc[pid] = (v + 1) and 0x0F
        return v
    }

    companion object {
        const val PAT_PID = 0x0000
        const val PMT_PID = 0x1000
        const val VIDEO_PID = 0x0100
        const val AUDIO_PID = 0x0101
        /** PCR runs slightly ahead of PTS so decoders have time to buffer. */
        const val PCR_LEAD_90K = 9000L

        private val CRC_TABLE = IntArray(256) { i ->
            var c = i shl 24
            repeat(8) { c = if (c and 0x80000000.toInt() != 0) (c shl 1) xor 0x04C11DB7 else c shl 1 }
            c
        }

        fun crc32(data: ByteArray): Int {
            var crc = -1
            for (b in data) crc = (crc shl 8) xor CRC_TABLE[((crc ushr 24) xor (b.toInt() and 0xFF)) and 0xFF]
            return crc
        }

        /** 7-byte ADTS header for AAC-LC. */
        fun adtsHeader(frameLen: Int, sampleRate: Int, channels: Int): ByteArray {
            val freqIdx = intArrayOf(96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350)
                .indexOf(sampleRate).also { require(it >= 0) { "unsupported sample rate $sampleRate" } }
            val full = frameLen + 7
            val profile = 1 // AAC LC (object type 2) - 1
            return byteArrayOf(
                0xFF.toByte(), 0xF1.toByte(),
                ((profile shl 6) or (freqIdx shl 2) or (channels shr 2)).toByte(),
                (((channels and 3) shl 6) or (full shr 11)).toByte(),
                ((full shr 3) and 0xFF).toByte(),
                (((full and 7) shl 5) or 0x1F).toByte(),
                0xFC.toByte(),
            )
        }
    }
}
