package io.github.temoin_app.temoin.ui

import android.app.KeyguardManager
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.unit.dp
import io.github.temoin_app.temoin.TemoinApp
import io.github.temoin_app.temoin.net.EnrollLink

val Accent = Color(0xFFE5484D)
val Ok = Color(0xFF30A46C)

class MainActivity : ComponentActivity() {
    private val pendingLink = mutableStateOf<EnrollLink?>(null)
    /** Phone locked (activity shown over the lock screen): no settings, no enrollment. */
    private val locked = mutableStateOf(false)

    companion object {
        const val ACTION_QUICK_RECORD = "io.github.temoin_app.temoin.QUICK_RECORD"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(SystemBarStyle.dark(android.graphics.Color.TRANSPARENT), SystemBarStyle.dark(android.graphics.Color.TRANSPARENT))
        val app = application as TemoinApp
        // A recreated activity (process death, recents) still carries the original
        // enrollment intent: only a fresh launch may process it.
        if (savedInstanceState == null && intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY == 0) handle(intent)
        setContent {
            MaterialTheme(
                colorScheme = darkColorScheme(
                    primary = Color.White, onPrimary = Color.Black,
                    secondary = Accent, background = Color.Black, surface = Color(0xFF14161B),
                    surfaceVariant = Color(0xFF1E2128), error = Accent,
                ),
            ) {
                Surface(color = Color.Black, contentColor = Color.White) {
                    Root(app, pendingLink.value, locked.value) { pendingLink.value = null }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    override fun onResume() {
        super.onResume()
        locked.value = getSystemService(KeyguardManager::class.java).isKeyguardLocked
    }

    private fun handle(intent: Intent?) {
        if (intent?.action == ACTION_QUICK_RECORD) {
            // Like a camera app: usable over the lock screen, capture only.
            setShowWhenLocked(true)
            setTurnScreenOn(true)
            (application as TemoinApp).engine.requestAutoRecord()
            return
        }
        val link = intent?.dataString?.let { EnrollLink.parse(it) } ?: return
        val current = (application as TemoinApp).settings.server.value
        // Same server and key already pinned: nothing to do.
        if (current != null && current.url == link.url.trimEnd('/') && current.recipient == link.recipient) return
        pendingLink.value = link
    }
}

@Composable
private fun Root(app: TemoinApp, link: EnrollLink?, locked: Boolean, consumeLink: () -> Unit) {
    val server by app.settings.server.collectAsState()
    var settingsOpen by remember { mutableStateOf(false) }

    when {
        // Over the lock screen: capture only, never enrollment (nobody may bind a locked phone to a server).
        server == null && locked -> LockedNotReady()
        server == null || (link != null && !locked) -> OnboardingScreen(app, link, onDone = consumeLink)
        settingsOpen && !locked -> {
            BackHandler { settingsOpen = false }
            SettingsScreen(app, onBack = { settingsOpen = false })
        }
        else -> CaptureScreen(app, onSettings = { if (!locked) settingsOpen = true }, settingsAllowed = !locked)
    }
}

@Composable
private fun LockedNotReady() {
    androidx.compose.foundation.layout.Box(
        androidx.compose.ui.Modifier.fillMaxSize(),
        contentAlignment = androidx.compose.ui.Alignment.Center,
    ) {
        androidx.compose.material3.Text(
            androidx.compose.ui.res.stringResource(io.github.temoin_app.temoin.R.string.unlock_to_setup),
            modifier = androidx.compose.ui.Modifier.padding(32.dp),
            color = Color(0xFFBDBDC2),
        )
    }
}
