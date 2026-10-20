package io.github.temoin_app.temoin

import android.app.Application
import io.github.temoin_app.temoin.capture.CaptureEngine
import io.github.temoin_app.temoin.data.Outbox
import io.github.temoin_app.temoin.data.Settings
import io.github.temoin_app.temoin.net.Uploader

class TemoinApp : Application() {
    lateinit var settings: Settings
        private set
    lateinit var outbox: Outbox
        private set
    lateinit var uploader: Uploader
        private set
    lateinit var engine: CaptureEngine
        private set

    override fun onCreate() {
        super.onCreate()
        settings = Settings(this)
        outbox = Outbox(this)
        uploader = Uploader(this)
        engine = CaptureEngine(this)
        // Anything left from a previous run (offline, killed app) goes out as soon as possible.
        if (outbox.list().isNotEmpty()) uploader.kick()
    }
}
