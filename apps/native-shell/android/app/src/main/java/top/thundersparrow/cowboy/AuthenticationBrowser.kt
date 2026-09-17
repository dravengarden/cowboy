package top.thundersparrow.cowboy

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.webkit.WebView
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import org.json.JSONObject

// Provider authentication browser for the Android shell. It implements the
// same page contract as the iOS shell (web/src/openExternal.ts, bridge v2):
// `__cowboyOpenAuthenticationBrowser(url)`, `__cowboyCloseAuthenticationBrowser()`
// and the opened / open-failed window events.
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
// Returning to Cowboy is not reported as a close. Chrome lets the user minimize
// a Custom Tab into picture-in-picture, which resumes MainActivity exactly like
// closing the tab, and a sessionless Custom Tab gets no callback to tell them
// apart. Cancelling on resume would discard a sign-in the user is still
// completing, so the page keeps its explicit Cancel and a resume only emits
// `cowboy:native-resume`.
internal class AuthenticationBrowser(
  private val activity: Activity,
  private val origin: String,
) {
  private var open = false
  private var covered = false
  private var resumed = false
  private var reply: JavaScriptReplyProxy? = null

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
    val intent = Intent(Intent.ACTION_VIEW, uri)
      .addCategory(Intent.CATEGORY_BROWSABLE)
      // A session extra, even a null binder, is what asks the browser for a
      // Custom Tab. Browsers without Custom Tabs open the URL normally.
      .putExtras(Bundle().apply { putBinder(EXTRA_SESSION, null) })
      .putExtra(EXTRA_SHARE_STATE, SHARE_STATE_OFF)
    try {
      activity.startActivity(intent)
    } catch (_: ActivityNotFoundException) {
      post(OPEN_FAILED_EVENT)
      return
    }
    open = true
    covered = false
    post(OPENED_EVENT)
  }

  // A programmatic close belongs to a completed or abandoned web flow. When the
  // browser still covers Cowboy, bring MainActivity back; singleTask finishes
  // the Custom Tab above it.
  private fun close() {
    if (!open) return
    open = false
    if (resumed) return
    val intent = Intent(activity, activity.javaClass)
      .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
    runCatching { activity.startActivity(intent) }
  }

  private fun post(event: String) {
    runCatching { reply?.postMessage(event) }
  }

  private companion object {
    const val BRIDGE = "cowboyAndroidAuthenticationBrowser"
    const val OPENED_EVENT = "cowboy:native-authentication-browser-opened"
    const val OPEN_FAILED_EVENT = "cowboy:native-authentication-browser-open-failed"
    const val RESUMED_EVENT = "cowboy:native-resume"
    const val EXTRA_SESSION = "android.support.customtabs.extra.SESSION"
    const val EXTRA_SHARE_STATE = "androidx.browser.customtabs.extra.SHARE_STATE"
    const val SHARE_STATE_OFF = 2
    // Page-world functions and event relay. Native replies are limited to the
    // three lifecycle event names; nothing else is dispatched into the page.
    val SCRIPT = """
      (() => {
        const bridge = globalThis.$BRIDGE;
        if (!bridge) return;
        const events = new Set([
          "$OPENED_EVENT", "$OPEN_FAILED_EVENT", "$RESUMED_EVENT",
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
