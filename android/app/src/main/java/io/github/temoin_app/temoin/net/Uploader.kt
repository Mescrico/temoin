package io.github.temoin_app.temoin.net

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.util.Log
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import io.github.temoin_app.temoin.TemoinApp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.util.concurrent.TimeUnit

/**
 * Drains the outbox. Two drivers share one mutex: an in-process loop (fast, used
 * while the app/recording service is alive) and a WorkManager job (survives the
 * app being killed, waits for connectivity). The server is idempotent, so a
 * double upload of the same object is harmless anyway.
 */
class Uploader(private val app: TemoinApp) {
    enum class State { IDLE, UPLOADING, WAITING_NETWORK, PAUSED_BY_SERVER, BANNED, NOT_REGISTERED, ERROR }
    data class Status(val state: State = State.IDLE, val message: String? = null, val uploadedBytes: Long = 0, val rejected: Int = 0)

    private val mutex = Mutex()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val _status = MutableStateFlow(Status(rejected = rejectedDir().listFiles()?.size ?: 0))
    val status: StateFlow<Status> = _status.asStateFlow()

    /** Called whenever a new object lands in the outbox. */
    fun kick() {
        scope.launch { drain() }
        schedule(app)
    }

    /** One pass over the outbox. Returns true when it is empty. */
    suspend fun drain(): Boolean = withContext(Dispatchers.IO) {
        if (!mutex.tryLock()) return@withContext false
        try {
            val server = app.settings.server.value ?: return@withContext false
            if (!networkOk()) {
                set(State.WAITING_NETWORK)
                return@withContext false
            }
            var backoff = 1000L
            while (true) {
                val items = app.outbox.list()
                if (items.isEmpty()) {
                    set(State.IDLE)
                    return@withContext true
                }
                set(State.UPLOADING)
                var progressed = false
                for (item in items) {
                    when (val r = Api.put(server.url, app.settings.deviceKey, item.session, item.seq, item.file)) {
                        UploadResult.Stored -> {
                            val size = item.file.length()
                            app.outbox.remove(item)
                            _status.value = _status.value.copy(uploadedBytes = _status.value.uploadedBytes + size)
                            progressed = true
                        }
                        is UploadResult.Rejected -> {
                            Log.w(TAG, "rejected ${item.file.name}: ${r.code} ${r.reason}")
                            rejectedDir().mkdirs()
                            item.file.renameTo(File(rejectedDir(), item.file.name))
                            app.outbox.refresh()
                            _status.value = _status.value.copy(rejected = rejectedDir().listFiles()?.size ?: 0)
                        }
                        is UploadResult.Retry -> {
                            set(
                                when {
                                    r.error == "banned" -> State.BANNED
                                    r.error == "unknown_device" -> State.NOT_REGISTERED
                                    r.paused -> State.PAUSED_BY_SERVER
                                    else -> State.ERROR
                                },
                                r.reason,
                            )
                            if (!networkOk()) {
                                set(State.WAITING_NETWORK)
                                return@withContext false
                            }
                            // Short in-process backoff; WorkManager takes over for long outages.
                            if (backoff > 30_000) return@withContext false
                            delay(backoff)
                            backoff *= 2
                            break
                        }
                    }
                }
                if (progressed) backoff = 1000L
            }
            @Suppress("UNREACHABLE_CODE")
            false
        } finally {
            mutex.unlock()
        }
    }

    fun rejectedDir() = File(app.noBackupFilesDir, "rejected")

    fun clearRejected() {
        rejectedDir().listFiles()?.forEach { it.delete() }
        _status.value = _status.value.copy(rejected = 0)
    }

    private fun set(state: State, message: String? = null) {
        _status.value = _status.value.copy(state = state, message = message)
    }

    private fun networkOk(): Boolean {
        val cm = app.getSystemService(ConnectivityManager::class.java)
        val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
        if (!caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) return false
        if (app.settings.user.value.wifiOnly && !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)) return false
        return true
    }

    companion object {
        private const val TAG = "SR.Uploader"
        private const val WORK = "sr-upload"

        fun schedule(context: Context) {
            val app = context.applicationContext as TemoinApp
            val constraints = Constraints.Builder()
                .setRequiredNetworkType(if (app.settings.user.value.wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED)
                .build()
            val req = OneTimeWorkRequestBuilder<UploadWorker>()
                .setConstraints(constraints)
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(WORK, ExistingWorkPolicy.APPEND_OR_REPLACE, req)
        }
    }
}

class UploadWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        val app = applicationContext as TemoinApp
        if (app.settings.server.value == null) return Result.success()
        return if (app.uploader.drain() || app.outbox.list().isEmpty()) Result.success() else Result.retry()
    }
}
