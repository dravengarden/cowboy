package top.thundersparrow.cowboy

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.webkit.WebView
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import org.json.JSONObject

// Provider authentication browser for the Android shell. It implements the
// same page contract as the iOS shell (web/src/openExternal.ts, bridge v2):
// `__cowboyOpenAuthenticationBrowser(url)`, `__cowboyCloseAuthenticationBrowser()`
// and the opened / open-failed / closed window events.
//
// Without it the remote UI treats Android as a plain browser and navigates the
// shell's only WebView to the Provider. That path has no busy state, so a
// repeated tap during the slow Provider redirect restarts OIDC on every tap and
// exhausts the per-IP pending-transaction limit. With the bridge the page runs
// the PKCE-bound native flow: a Custom Tab shows the Provider while the WebView
// keeps its state and waits on the handoff WebSocket.
//
// The Custom Tab is launched with the raw protocol extras rather than
// androidx.browser, whose dependency the generated Gradle project does not
// carry. It opens in Cowboy's task, above MainActivity (launchMode singleTask),
// so relaunching MainActivity dismisses it.
//
// It is a partial (bottom-sheet) Custom Tab so Cowboy stays visible. A
// full-screen tab leaves Cowboy's process cached, and Android's cached-app
// freezer then stops both the app and its WebView renderer: the page never
// receives the ready handoff event and cannot dismiss the tab. Chrome honours
// the partial height only for tabs started for a result, which also reports
// when the tab actually finishes. Resuming MainActivity alone is not a close:
// Chrome can minimize a tab into picture-in-picture.
internal class AuthenticationBrowser(
  private val activity: ComponentActivity,
  private val origin: String,
) {
  private val handler = Handler(Looper.getMainLooper())
  private var open = false
  private var generation = 0
  private var covered = false
  private var resumed = false
  private var reply: JavaScriptReplyProxy? = null

  // Registered at construction, before the activity is created, as the
  // Activity Result API requires.
  private val launcher = activity.registerForActivityResult(
    ActivityResultContracts.StartActivityForResult(),
  ) { onBrowserFinished() }

  fun install(webView: WebView) {
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER) ||
      !WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)
    ) return
    // Only the Cowboy main frame may open or close the browser.
    val origins = setOf(origin)
    WebViewCompat.addWebMessageListener(webView, BRIDGE, origins) {
        _: WebView, message: WebMessageCompat, sourceOrigin: Uri, isMainFrame: Boolean,
        replyProxy: JavaScriptReplyProxy ->
      if (!isMainFrame || sourceOrigin.toString().trimEnd('/') != origin) {
        return@addWebMessageListener
      }
      val payload = runCatching { JSONObject(message.data ?: "") }.getOrNull()
        ?: return@addWebMessageListener
      activity.runOnUiThread {
        when (payload.optString("action")) {
          "open" -> open(payload.optString("url"), replyProxy)
          "close" -> close()
        }
      }
    }
    WebViewCompat.addDocumentStartJavaScript(webView, SCRIPT, origins)
  }

  fun onPause() {
    resumed = false
    if (open) covered = true
  }

  // The WebView was hidden and throttled while the browser covered it. Wake the
  // page so resume-driven reconciliation (passkeys, update checks) runs now.
  fun onResume() {
    resumed = true
    if (!covered) return
    covered = false
    post(RESUMED_EVENT)
  }

  private fun open(rawUrl: String, replyProxy: JavaScriptReplyProxy) {
    reply = replyProxy
    val uri = Uri.parse(rawUrl)
    if ((uri.scheme != "https" && uri.scheme != "http") || uri.host.isNullOrEmpty()) {
      post(OPEN_FAILED_EVENT)
      return
    }
    val height = (activity.resources.displayMetrics.heightPixels * SHEET_HEIGHT_FRACTION).toInt()
    val intent = Intent(Intent.ACTION_VIEW, uri)
      .addCategory(Intent.CATEGORY_BROWSABLE)
      // A session extra, even a null binder, is what asks the browser for a
      // Custom Tab. Browsers without Custom Tabs open the URL normally.
      .putExtras(Bundle().apply { putBinder(EXTRA_SESSION, null) })
      .putExtra(EXTRA_SHARE_STATE, SHARE_STATE_OFF)
      .putExtra(EXTRA_INITIAL_ACTIVITY_HEIGHT_PX, height)
      .putExtra(EXTRA_ACTIVITY_HEIGHT_RESIZE_BEHAVIOR, ACTIVITY_HEIGHT_ADJUSTABLE)
    try {
      launcher.launch(intent)
    } catch (_: ActivityNotFoundException) {
      post(OPEN_FAILED_EVENT)
      return
    }
    open = true
    covered = false
    generation += 1
    post(OPENED_EVENT)
  }

  // A programmatic close belongs to a completed or abandoned web flow. When the
  // browser still covers Cowboy, bring MainActivity back; singleTask finishes
  // the Custom Tab above it.
  private fun close() {
    if (!open) return
    open = false
    generation += 1
    if (resumed) return
    val intent = Intent(activity, activity.javaClass)
      .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
    runCatching { activity.startActivity(intent) }
  }

  // The tab finished without a programmatic close: the user closed it. Report
  // it after a grace period, because a handoff that completed just before the
  // close may still be delivering its ready event; that path closes the
  // browser itself and cancels this notification.
  private fun onBrowserFinished() {
    if (!open) return
    val finished = generation
    handler.postDelayed({
      if (open && generation == finished) {
        open = false
        post(CLOSED_EVENT)
      }
    }, CLOSE_GRACE_MS)
  }

  private fun post(event: String) {
    runCatching { reply?.postMessage(event) }
  }

  private companion object {
    const val BRIDGE = "cowboyAndroidAuthenticationBrowser"
    const val OPENED_EVENT = "cowboy:native-authentication-browser-opened"
    const val OPEN_FAILED_EVENT = "cowboy:native-authentication-browser-open-failed"
    const val CLOSED_EVENT = "cowboy:native-authentication-browser-closed"
    const val RESUMED_EVENT = "cowboy:native-resume"
    const val CLOSE_GRACE_MS = 1_500L
    const val SHEET_HEIGHT_FRACTION = 0.9
    const val EXTRA_SESSION = "android.support.customtabs.extra.SESSION"
    const val EXTRA_SHARE_STATE = "androidx.browser.customtabs.extra.SHARE_STATE"
    const val SHARE_STATE_OFF = 2
    const val EXTRA_INITIAL_ACTIVITY_HEIGHT_PX =
      "androidx.browser.customtabs.extra.INITIAL_ACTIVITY_HEIGHT_PX"
    const val EXTRA_ACTIVITY_HEIGHT_RESIZE_BEHAVIOR =
      "androidx.browser.customtabs.extra.ACTIVITY_HEIGHT_RESIZE_BEHAVIOR"
    const val ACTIVITY_HEIGHT_ADJUSTABLE = 1
    // Page-world functions and event relay. Native replies are limited to the
    // lifecycle event names below; nothing else is dispatched into the page.
    val SCRIPT = """
      (() => {
        const bridge = globalThis.$BRIDGE;
        if (!bridge) return;
        const events = new Set([
          "$OPENED_EVENT", "$OPEN_FAILED_EVENT", "$CLOSED_EVENT", "$RESUMED_EVENT",
        ]);
        bridge.addEventListener("message", (event) => {
          if (events.has(event.data)) globalThis.dispatchEvent(new Event(event.data));
        });
        globalThis.__cowboyAuthenticationBrowserBridgeVersion = 2;
        globalThis.__cowboyOpenAuthenticationBrowser = (url) => {
          try {
            bridge.postMessage(JSON.stringify({ action: "open", url: String(url) }));
            return true;
          } catch {
            return false;
          }
        };
        globalThis.__cowboyCloseAuthenticationBrowser = () => {
          try {
            bridge.postMessage(JSON.stringify({ action: "close" }));
          } catch {}
        };
      })();
    """.trimIndent()
  }
}
