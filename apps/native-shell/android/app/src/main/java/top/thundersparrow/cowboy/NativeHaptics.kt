package top.thundersparrow.cowboy

import android.net.Uri
import android.os.Build
import android.view.HapticFeedbackConstants
import android.webkit.WebView
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature

// System haptics for the remote Cowboy UI on Android.
//
// tauri-plugin-haptics plays every impact as a 40-60 ms raw vibrator waveform
// (and notifications as multi-pulse patterns of about a quarter second) behind
// an asynchronous IPC hop, which feels like a buzzing motor rather than a tap.
// The page instead calls `__cowboyNativeHaptic(kind)`
// (components/app-shell/haptics.ts), which this bridge maps to
// View.performHapticFeedback: the short, tuned click and tick effects the system
// keyboard and launcher use, honouring the user's touch-feedback setting and
// intensity.
internal class NativeHaptics(private val origin: String) {
  fun install(webView: WebView) {
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER) ||
      !WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)
    ) return
    val origins = setOf(origin)
    WebViewCompat.addWebMessageListener(webView, BRIDGE, origins) {
        view: WebView, message: WebMessageCompat, sourceOrigin: Uri, isMainFrame: Boolean,
        _: JavaScriptReplyProxy ->
      if (!isMainFrame || sourceOrigin.toString().trimEnd('/') != origin) {
        return@addWebMessageListener
      }
      val feedback = feedbackFor(message.data) ?: return@addWebMessageListener
      view.post { view.performHapticFeedback(feedback) }
    }
    WebViewCompat.addDocumentStartJavaScript(webView, SCRIPT, origins)
  }

  // Strength follows the page's intent: selection is the faintest texture tick,
  // ordinary taps a light tick, firmer commits a click, and only heavy impacts
  // and errors reach the heavy effects.
  private fun feedbackFor(kind: String?): Int? = when (kind) {
    "selection" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      HapticFeedbackConstants.SEGMENT_FREQUENT_TICK
    } else {
      HapticFeedbackConstants.CLOCK_TICK
    }
    "impact:light", "impact:soft" -> HapticFeedbackConstants.CONTEXT_CLICK
    "impact:medium", "impact:rigid" -> HapticFeedbackConstants.VIRTUAL_KEY
    "impact:heavy" -> HapticFeedbackConstants.LONG_PRESS
    "notification:success", "notification:warning" ->
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        HapticFeedbackConstants.CONFIRM
      } else {
        HapticFeedbackConstants.VIRTUAL_KEY
      }
    "notification:error" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      HapticFeedbackConstants.REJECT
    } else {
      HapticFeedbackConstants.LONG_PRESS
    }
    else -> null
  }

  private companion object {
    const val BRIDGE = "cowboyAndroidHaptics"
    val SCRIPT = """
      (() => {
        const bridge = globalThis.$BRIDGE;
        if (!bridge) return;
        globalThis.__cowboyNativeHaptic = (kind) => {
          try {
            bridge.postMessage(String(kind));
            return true;
          } catch {
            return false;
          }
        };
      })();
    """.trimIndent()
  }
}
