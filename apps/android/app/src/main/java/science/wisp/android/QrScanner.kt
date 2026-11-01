package science.wisp.android

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.ImageAnalysis
import androidx.camera.mlkit.vision.MlKitAnalyzer
import androidx.camera.view.LifecycleCameraController
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import science.wisp.remote.RemoteConfiguration

/**
 * Live scan of the QR code the desktop shows under its connection code. The
 * code is the remote link itself, so the first frame that parses as one is
 * handed to the connection form; every other QR code is ignored.
 */
@Composable
fun QrScannerDialog(onLink: (String) -> Unit, onDismiss: () -> Unit) {
    val context = LocalContext.current
    var granted by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    var asked by remember { mutableStateOf(granted) }
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it; asked = true }
    LaunchedEffect(Unit) { if (!granted) request.launch(Manifest.permission.CAMERA) }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize().testTag("qr-scanner")) {
            Column(Modifier.fillMaxSize()) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    TextButton(onClick = onDismiss) { Text("取消") }
                    Text("扫描桌面二维码", style = MaterialTheme.typography.titleMedium)
                }
                if (granted) {
                    CameraScan(Modifier.weight(1f), onLink)
                } else if (asked) {
                    Text("没有相机权限。可在系统设置中允许 Wisp Science 使用相机，或粘贴桌面端复制的链接。", Modifier.padding(24.dp))
                }
            }
        }
    }
}

@Composable
private fun CameraScan(modifier: Modifier, onLink: (String) -> Unit) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val controller = remember { LifecycleCameraController(context) }
    val scanner = remember {
        BarcodeScanning.getClient(BarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build())
    }
    DisposableEffect(controller) {
        var done = false
        val executor = ContextCompat.getMainExecutor(context)
        controller.setImageAnalysisAnalyzer(executor, MlKitAnalyzer(listOf(scanner), ImageAnalysis.COORDINATE_SYSTEM_ORIGINAL, executor) { result ->
            if (done) return@MlKitAnalyzer
            val link = result.getValue(scanner)?.firstNotNullOfOrNull { it.rawValue?.takeIf(::isWispLink) } ?: return@MlKitAnalyzer
            done = true
            onLink(link)
        })
        controller.bindToLifecycle(lifecycleOwner)
        onDispose {
            controller.unbind()
            scanner.close()
        }
    }
    AndroidView(factory = { PreviewView(it).apply { this.controller = controller } }, modifier = modifier)
}

/** Accepts exactly what the connection form would; camera input is untrusted, so any failure means "not ours". */
internal fun isWispLink(payload: String): Boolean =
    try {
        RemoteConfiguration.parse(payload)
        true
    } catch (_: Exception) {
        false
    }
