package io.github.temoin_app.temoin.service

import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import io.github.temoin_app.temoin.TemoinApp
import io.github.temoin_app.temoin.ui.MainActivity

/**
 * Quick Settings tile: one tap, even with the phone locked, opens the capture screen over
 * the lock screen and starts a recording right away (no unlock, no extra tap).
 */
class QuickRecordTile : TileService() {

    override fun onStartListening() {
        val app = application as TemoinApp
        qsTile?.apply {
            state = if (app.engine.state.value.recording) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
            updateTile()
        }
    }

    override fun onClick() {
        val intent = Intent(this, MainActivity::class.java)
            .setAction(MainActivity.ACTION_QUICK_RECORD)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(
                PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            )
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }
}
