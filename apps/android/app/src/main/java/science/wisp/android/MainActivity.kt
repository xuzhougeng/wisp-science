package science.wisp.android

import android.os.Bundle
import android.view.KeyEvent
import android.view.Window
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color

class MainActivity : ComponentActivity() {
    private val model: WorkspaceModel by viewModels()
    private var handlingEscape = false
    private var previousWindowCallback: Window.Callback? = null
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val colors = if (isSystemInDarkTheme()) darkColorScheme(primary = Color(0xFF84CFC4), onPrimary = Color(0xFF00382F),
                primaryContainer = Color(0xFF244B46), onPrimaryContainer = Color(0xFFB2E0D9),
                secondary = Color(0xFF84CFC4), secondaryContainer = Color(0xFF244B46), onSecondaryContainer = Color(0xFFD0EEE9),
                onSurface = Color(0xFFE9E9EC), onBackground = Color(0xFFE9E9EC), background = Color(0xFF161619),
                surface = Color(0xFF232326), surfaceVariant = Color(0xFF323238), onSurfaceVariant = Color(0xFFADADB5), outlineVariant = Color(0xFF424247))
                else lightColorScheme(primary = Color(0xFF196D65), onPrimary = Color.White,
                primaryContainer = Color(0xFFE2EFED), onPrimaryContainer = Color(0xFF153F39),
                secondary = Color(0xFF196D65), secondaryContainer = Color(0xFFE2EFED), onSecondaryContainer = Color(0xFF153F39), background = Color(0xFFF2F2F7),
                surface = Color.White, surfaceVariant = Color(0xFFE9E9EC), onSurface = Color(0xFF151516),
                onSurfaceVariant = Color(0xFF85858C), outlineVariant = Color(0xFFE1E1E5))
            MaterialTheme(colorScheme = colors) { WorkspaceScreen(model) }
        }
        // Window-level routing works immediately, before any child obtains focus.
        // Delegate through the public Window API, not AndroidX's restricted dispatcher.
        val previous = window.callback
        previousWindowCallback = previous
        window.callback = object : Window.Callback by previous {
            override fun dispatchKeyEvent(event: KeyEvent): Boolean {
                if (event.keyCode == KeyEvent.KEYCODE_ESCAPE) {
                    if (event.action == KeyEvent.ACTION_DOWN && event.repeatCount == 0) handlingEscape = model.back()
                    if (handlingEscape) {
                        if (event.action == KeyEvent.ACTION_UP) handlingEscape = false
                        return true
                    }
                }
                return previous.dispatchKeyEvent(event)
            }
        }
    }
    override fun onStart() { super.onStart(); model.resume() }
    override fun onStop() { model.pause(); super.onStop() }
    override fun onDestroy() {
        previousWindowCallback?.let { window.callback = it }
        previousWindowCallback = null
        super.onDestroy()
    }
}
