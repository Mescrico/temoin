package io.github.temoin_app.temoin.capture

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Looper
import androidx.core.content.ContextCompat
import java.time.Instant

/**
 * Optional (off by default): attaches the latest fix to each object's encrypted header.
 * Uses the platform LocationManager only — no Google Play services.
 */
class LocationTracker(private val context: Context) {
    private val lm = context.getSystemService(LocationManager::class.java)
    @Volatile private var fix: Location? = null
    private var started = false
    private val listener = LocationListener { fix = it }

    fun start() {
        if (started) return
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED) return
        started = true
        for (p in listOf(LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER)) {
            runCatching {
                if (lm.isProviderEnabled(p)) {
                    lm.getLastKnownLocation(p)?.let { if (fix == null || it.time > fix!!.time) fix = it }
                    lm.requestLocationUpdates(p, 5000L, 5f, listener, Looper.getMainLooper())
                }
            }
        }
    }

    fun stop() {
        if (!started) return
        started = false
        runCatching { lm.removeUpdates(listener) }
    }

    fun last(): Map<String, Any?>? = fix?.let {
        mapOf(
            "lat" to it.latitude,
            "lon" to it.longitude,
            "acc_m" to if (it.hasAccuracy()) it.accuracy.toDouble() else null,
            "alt_m" to if (it.hasAltitude()) it.altitude else null,
            "at" to Instant.ofEpochMilli(it.time).toString(),
            "provider" to it.provider,
        )
    }
}
