package io.github.temoin_app.temoin.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.background
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import androidx.compose.foundation.Image
import androidx.compose.ui.res.painterResource
import androidx.compose.foundation.shape.RoundedCornerShape
import io.github.temoin_app.temoin.R
import io.github.temoin_app.temoin.TemoinApp
import io.github.temoin_app.temoin.crypto.Recipient
import io.github.temoin_app.temoin.data.ServerConfig
import io.github.temoin_app.temoin.net.Api
import io.github.temoin_app.temoin.net.ApiException
import io.github.temoin_app.temoin.net.EnrollLink
import io.github.temoin_app.temoin.net.RegisterResult
import io.github.temoin_app.temoin.net.ServerInfo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private data class Candidate(val url: String, val invite: String, val info: ServerInfo, val fromQr: Boolean)

@Composable
fun OnboardingScreen(app: TemoinApp, link: EnrollLink?, onDone: () -> Unit) {
    val scope = rememberCoroutineScope()
    var manual by remember { mutableStateOf(false) }
    var url by remember { mutableStateOf("") }
    var invite by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var candidate by remember { mutableStateOf<Candidate?>(null) }

    val errMismatch = stringResource(R.string.err_key_mismatch)
    val errInviteRequired = stringResource(R.string.err_invite_required)
    val regErrors = mapOf(
        "unknown_invite" to stringResource(R.string.err_invite_unknown),
        "invite_expired" to stringResource(R.string.err_invite_expired),
        "invite_revoked" to stringResource(R.string.err_invite_revoked),
        "invite_exhausted" to stringResource(R.string.err_invite_exhausted),
        "banned" to stringResource(R.string.err_banned),
    )
    val errRegister = stringResource(R.string.err_register)
    val errNoAnswer = stringResource(R.string.err_no_answer)
    val errBadQr = stringResource(R.string.err_bad_qr)
    val errNotServer = stringResource(R.string.err_not_server)

    fun check(rawUrl: String, tok: String?, expectedKey: String?) {
        if (tok.isNullOrBlank()) {
            // A discreet server does not even describe itself without an invitation code.
            error = errInviteRequired; manual = true; url = rawUrl
            return
        }
        busy = true
        error = null
        scope.launch {
            try {
                val u = Api.normalizeUrl(rawUrl)
                val info = withContext(Dispatchers.IO) { Api.info(u, invite = tok) }
                when {
                    !Recipient.isValid(info.recipient) -> error = errNotServer.format("bad key")
                    expectedKey != null && info.recipient != expectedKey -> error = errMismatch
                    else -> candidate = Candidate(u, tok, info, expectedKey != null)
                }
            } catch (t: ApiException) {
                error = if (t.status == 404) errNoAnswer else errNotServer.format(t.message ?: "")
            } catch (t: Throwable) {
                error = errNotServer.format(t.message ?: t.javaClass.simpleName)
            } finally {
                busy = false
            }
        }
    }

    LaunchedEffect(link) {
        if (link != null) {
            if (!Recipient.isValid(link.recipient)) error = errBadQr
            else check(link.url, link.invite, link.recipient)
        }
    }

    val scanner = rememberLauncherForActivityResult(ScanContract()) { result ->
        val contents = result.contents ?: return@rememberLauncherForActivityResult
        val l = EnrollLink.parse(contents)
        if (l == null || !Recipient.isValid(l.recipient)) error = errBadQr else check(l.url, l.invite, l.recipient)
    }
    val scanPrompt = stringResource(R.string.onb_scan)

    Box(Modifier.fillMaxSize().background(Color.Black).safeDrawingPadding()) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Spacer(Modifier.height(32.dp))
            Image(painterResource(R.drawable.logo), contentDescription = null, modifier = Modifier.size(64.dp).clip(RoundedCornerShape(14.dp)))
            Text(stringResource(R.string.app_name), style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.Bold)
            Text(stringResource(R.string.tagline), style = MaterialTheme.typography.titleMedium, color = Color(0xFFBDBDC2))
            Text(stringResource(R.string.onb_body), color = Color(0xFF9AA0A6))
            Spacer(Modifier.height(8.dp))

            Button(
                onClick = {
                    scanner.launch(ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE).setPrompt(scanPrompt)
                        .setBeepEnabled(false).setOrientationLocked(true))
                },
                enabled = !busy,
                modifier = Modifier.fillMaxWidth().height(56.dp),
            ) {
                Icon(Icons.Default.QrCodeScanner, null)
                Spacer(Modifier.size(8.dp))
                Text(stringResource(R.string.onb_scan))
            }
            if (!manual) {
                OutlinedButton(onClick = { manual = true }, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.onb_manual), color = Color.White)
                }
            } else {
                OutlinedTextField(
                    value = url, onValueChange = { url = it }, singleLine = true,
                    label = { Text(stringResource(R.string.onb_url)) },
                    placeholder = { Text("https://record.example.org") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next, autoCorrectEnabled = false),
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = invite, onValueChange = { invite = it.uppercase() }, singleLine = true,
                    label = { Text(stringResource(R.string.onb_invite)) },
                    placeholder = { Text("ABCDE FGHJK LMNPQ RSTUV") },
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false, imeAction = ImeAction.Done),
                    modifier = Modifier.fillMaxWidth(),
                )
                Button(onClick = { check(url, invite, link?.recipient) }, enabled = !busy && url.isNotBlank(), modifier = Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.onb_connect))
                }
            }
            if (busy) {
                Row2 { CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp); Text(stringResource(R.string.onb_checking)) }
            }
            error?.let { Text(it, color = Accent) }
        }
    }

    candidate?.let { c ->
        val fp = Recipient.fingerprint(c.info.recipient)
        AlertDialog(
            onDismissRequest = { candidate = null },
            title = { Text(stringResource(R.string.onb_confirm_title)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(c.info.name, fontWeight = FontWeight.SemiBold)
                    Text(c.url, color = Color(0xFF9AA0A6))
                    Text(fp, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.titleMedium)
                    Text(
                        if (c.fromQr) stringResource(R.string.onb_confirm_qr_body)
                        else stringResource(R.string.onb_confirm_body),
                        color = Color(0xFF9AA0A6),
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    candidate = null
                    busy = true
                    error = null
                    scope.launch {
                        val key = app.settings.keyFor(c.url, c.info.recipient)
                        when (val r = withContext(Dispatchers.IO) { Api.register(c.url, key, c.invite) }) {
                            is RegisterResult.Ok -> {
                                app.settings.adoptDeviceKey(key)
                                app.settings.saveServer(ServerConfig(c.url, c.info.recipient, c.info.name, r.name, c.info.limits))
                                app.settings.updateUser { it.copy(videoHeight = c.info.limits.defaultVideoHeight.coerceAtMost(c.info.limits.maxVideoHeight)) }
                                app.uploader.kick()
                                onDone()
                            }
                            is RegisterResult.Failed -> error = regErrors[r.error] ?: errRegister.format(r.message)
                        }
                        busy = false
                    }
                }) { Text(stringResource(R.string.trust)) }
            },
            dismissButton = { TextButton(onClick = { candidate = null }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

@Composable
private fun Row2(content: @Composable () -> Unit) {
    androidx.compose.foundation.layout.Row(
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) { content() }
}
