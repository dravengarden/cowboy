package top.thundersparrow.cowboy

import android.content.res.Configuration
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.graphics.ColorUtils
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature

// Cowboy's Android shell activity. Tauri generates the surrounding Gradle
// project; the builder overlays this owned source onto every generated project.
class MainActivity : TauriActivity() {
  private lateinit var content: View

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    content = findViewById(android.R.id.content)
    applyContentInsets(content)
    applySystemBarColor(defaultSurface())
  }

  override fun onWebViewCreate(webView: WebView) {
    followThemeColor(webView)
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

  // The padded system-bar strips show the container background. The Web UI
  // keeps its theme-color meta equal to the navbar surface for the selected
  // appearance, so mirror that colour and pick matching bar glyphs. Before the
  // remote page reports a colour, use the Web defaults for the system theme.
  private fun followThemeColor(webView: WebView) {
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER) ||
      !WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)
    ) return
    // Only the Cowboy origin may report a colour; the message carries nothing
    // but a colour string and grants no other native effect.
    val origins = setOf(COWBOY_ORIGIN)
    WebViewCompat.addWebMessageListener(webView, THEME_BRIDGE, origins) {
        _: WebView, message: WebMessageCompat, sourceOrigin: Uri, isMainFrame: Boolean,
        _: JavaScriptReplyProxy ->
      if (!isMainFrame || sourceOrigin.toString().trimEnd('/') != COWBOY_ORIGIN) {
        return@addWebMessageListener
      }
      val color = parseCssColor(message.data) ?: return@addWebMessageListener
      runOnUiThread { applySystemBarColor(color) }
    }
    WebViewCompat.addDocumentStartJavaScript(webView, THEME_OBSERVER, origins)
  }

  private fun applySystemBarColor(color: Int) {
    content.setBackgroundColor(color)
    val light = ColorUtils.calculateLuminance(color) > 0.5
    WindowCompat.getInsetsController(window, window.decorView).apply {
      isAppearanceLightStatusBars = light
      isAppearanceLightNavigationBars = light
    }
  }

  private fun defaultSurface(): Int {
    val night = resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK
    return if (night == Configuration.UI_MODE_NIGHT_YES) DARK_SURFACE else LIGHT_SURFACE
  }

  private fun parseCssColor(value: String?): Int? {
    val text = value?.trim() ?: return null
    HEX.matchEntire(text)?.let { return Color.parseColor(text) }
    RGB.matchEntire(text)?.let { match ->
      val (r, g, b) = match.destructured
      return Color.rgb(r.toInt(), g.toInt(), b.toInt())
    }
    return null
  }

  private companion object {
    const val COWBOY_ORIGIN = "https://cowboy.stormbird.xyz"
    const val THEME_BRIDGE = "cowboyAndroidThemeColor"
    // Matches web/index.html's initial theme-color metas.
    val LIGHT_SURFACE = Color.parseColor("#f7fdff")
    val DARK_SURFACE = Color.parseColor("#101014")
    val HEX = Regex("#[0-9a-fA-F]{6}")
    val RGB = Regex("rgba?\\((\\d{1,3}),\\s*(\\d{1,3}),\\s*(\\d{1,3})(?:,\\s*[\\d.]+)?\\)")
    // Reports the theme-color that currently applies: the Web UI replaces the
    // meta on appearance changes and the static document ships media-scoped
    // light/dark metas.
    val THEME_OBSERVER = """
      (() => {
        const bridge = globalThis.$THEME_BRIDGE;
        if (!bridge) return;
        let last = "";
        const report = () => {
          const metas = document.querySelectorAll('meta[name="theme-color"]');
          for (const meta of metas) {
            if (meta.media && !matchMedia(meta.media).matches) continue;
            const color = meta.content.trim();
            if (color && color !== last) {
              last = color;
              bridge.postMessage(color);
            }
            return;
          }
        };
        const observe = () => {
          report();
          new MutationObserver(report).observe(document.head, {
            childList: true, subtree: true, attributes: true,
            attributeFilter: ["content", "media"],
          });
          matchMedia("(prefers-color-scheme: dark)").addEventListener("change", report);
        };
        if (document.head) observe();
        else document.addEventListener("DOMContentLoaded", observe, { once: true });
      })();
    """.trimIndent()
  }
}
