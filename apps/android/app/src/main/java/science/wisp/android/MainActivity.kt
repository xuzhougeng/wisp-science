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
            val colors = if (isSystemInDarkTheme()) darkColorScheme(primary = Color(0xFFAACFB5))
                else lightColorScheme(primary = Color(0xFF42634E), surface = Color(0xFFF9FAF7))
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
