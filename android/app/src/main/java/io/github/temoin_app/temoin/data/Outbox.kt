package io.github.temoin_app.temoin.data

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File
import java.io.FileOutputStream

/**
 * Queue of sealed objects waiting for upload. Only ciphertext is ever written
 * here: the phone cannot decrypt any of it. Files are deleted once the server
 * acknowledges them.
 */
class Outbox(context: Context) {
    data class Item(val file: File, val session: String, val seq: Int)
    data class Stats(val count: Int, val bytes: Long)

    private val dir = File(context.noBackupFilesDir, "outbox").apply { mkdirs() }
    private val _stats = MutableStateFlow(computeStats())
    val stats: StateFlow<Stats> = _stats.asStateFlow()

    init {
        dir.listFiles { f -> f.name.endsWith(".part") }?.forEach { it.delete() }
    }

    /** Atomically adds a sealed object (write to .part, fsync, rename). */
    fun put(session: String, seq: Int, sealed: ByteArray) {
        val name = "%s_%06d.age".format(session, seq)
        val part = File(dir, "$name.part")
        FileOutputStream(part).use { out ->
            out.write(sealed)
            out.fd.sync()
        }
        check(part.renameTo(File(dir, name))) { "outbox rename failed" }
        refresh()
    }

    /** Oldest first, so a recording arrives in order. */
    fun list(): List<Item> = (dir.listFiles { f -> f.name.endsWith(".age") } ?: emptyArray())
        .sortedWith(compareBy({ it.lastModified() }, { it.name }))
        .mapNotNull { f ->
            val base = f.name.removeSuffix(".age")
            val i = base.lastIndexOf('_')
            if (i <= 0) null else Item(f, base.substring(0, i), base.substring(i + 1).toIntOrNull() ?: return@mapNotNull null)
        }

    fun remove(item: Item) {
        item.file.delete()
        refresh()
    }

    fun refresh() {
        _stats.value = computeStats()
    }

    private fun computeStats(): Stats {
        val files = dir.listFiles { f -> f.name.endsWith(".age") } ?: emptyArray()
        return Stats(files.size, files.sumOf { it.length() })
    }
}
