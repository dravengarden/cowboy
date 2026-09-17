package top.thundersparrow.cowboy

import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

// Cowboy's Android shell activity. Tauri generates the surrounding Gradle
// project; the builder overlays this owned source onto every generated project.
class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    applyContentInsets(findViewById(android.R.id.content))
  }

  // Android 15+ always draws apps edge-to-edge, so the WebView would sit under
  // the status bar, navigation bar, display cutout and IME. The remote Cowboy
  // UI lays out against its own viewport (`interactive-widget=resizes-content`)
  // and has no Android safe-area contract, so shrink the WebView's container to
  // the visible area instead. The IME inset makes the viewport resize when the
  // keyboard opens, keeping the Composer above it; consuming the insets keeps
  // the WebView from applying the same insets a second time.
  private fun applyContentInsets(content: View) {
    val visibleArea = WindowInsetsCompat.Type.systemBars() or
      WindowInsetsCompat.Type.displayCutout() or
      WindowInsetsCompat.Type.ime()
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val area = insets.getInsets(visibleArea)
      view.setPadding(area.left, area.top, area.right, area.bottom)
      WindowInsetsCompat.CONSUMED
    }
    ViewCompat.requestApplyInsets(content)
  }
}
