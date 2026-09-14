// The Android native window itself (design doc D5). A plain `Activity` with
// a full-bleed `WebView` — nothing else. This class decides nothing about
// the payer's payment outcome (D1): it only watches for a navigation that
// matches one of `ShowCheckoutRequest.stopUrls`, or a dismissal, and
// reports which. `checkout_controller.dart`'s poll of
// `GET /v1/browser/payment_intents/{id}` is the only thing that ever says
// `succeeded`.
//
// Non-negotiables (docs/plans/2026-09-13-flutter-plugin-brief.md, and the
// design doc's D5):
// - `android:exported="false"` in AndroidManifest.xml — this Activity is
//   never started by anything outside this app.
// - `onReceivedSslError` is **not overridden** below: the default
//   `WebViewClient` behaviour is to cancel the load on any TLS error, and
//   that default is exactly what a payment SDK must do. Overriding it to
//   proceed through a certificate error is the single worst thing this
//   plugin could ship — so it is easier to audit this file for the absence
//   of that override than to trust a good implementation of one.
// - No `addJavascriptInterface` anywhere in this file (design doc D3): the
//   page loaded here is third-party rail content, not vpay's own, and nothing
//   here exposes a native bridge to it.
// - `allowFileAccess`, `allowFileAccessFromFileURLs` and
//   `allowUniversalAccessFromFileURLs` are all `false`.
// - The WebView's data store is Android's ordinary default (D-M5) — Android's
//   `WebView` has no separate "ephemeral" mode the way `WKWebView` does; not
//   doing anything special to wipe or sandbox it is what keeps it persistent.
// - Back press is always a dismissal, **never** a WebView history pop — a
//   history pop would land the payer back on a stale form (design doc D5).
package dev.vpay.checkout_flutter

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.view.ViewGroup
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback

/** The Intent extra carrying `ShowCheckoutRequest.url` — cleared on finish. */
private const val EXTRA_URL = "vpay_checkout_url"
private const val EXTRA_ALLOW_INSECURE = "vpay_checkout_allow_insecure"
private const val EXTRA_STOP_URLS = "vpay_checkout_stop_urls"
private const val EXTRA_RESULT_OUTCOME = "vpay_checkout_result_outcome"
private const val EXTRA_RESULT_REACHED_URL = "vpay_checkout_result_reached_url"

private const val BUNDLE_SCHEME = "scheme"
private const val BUNDLE_HOST = "host"
private const val BUNDLE_PORT = "port"
private const val BUNDLE_PATH = "path"

class VpayCheckoutActivity : ComponentActivity() {

  companion object {
    const val REQUEST_CODE = 0x76706179 // 'vpay' in hex, arbitrarily.

    /** The single open instance, if any — cleared in [onDestroy]. */
    private var current: VpayCheckoutActivity? = null

    fun isOpen(): Boolean = current != null

    fun dismissIfOpen() {
      current?.finishAsDismissed()
    }

    fun buildIntent(context: Context, request: ShowCheckoutRequest): Intent {
      val stopUrls = ArrayList<Bundle>()
      for (stopUrl in request.stopUrls) {
        if (stopUrl == null) continue
        stopUrls.add(
          Bundle().apply {
            putString(BUNDLE_SCHEME, stopUrl.scheme)
            putString(BUNDLE_HOST, stopUrl.host)
            putLong(BUNDLE_PORT, stopUrl.port)
            putString(BUNDLE_PATH, stopUrl.path)
          }
        )
      }
      return Intent(context, VpayCheckoutActivity::class.java).apply {
        putExtra(EXTRA_URL, request.url)
        putExtra(EXTRA_ALLOW_INSECURE, request.allowInsecureUrl)
        putParcelableArrayListExtra(EXTRA_STOP_URLS, stopUrls)
      }
    }

    /** Reads the result [VpayCheckoutFlutterPlugin] hands back to Dart. */
    fun eventFromResult(data: Intent?): CheckoutWindowEvent {
      val rawOutcome = data?.getIntExtra(EXTRA_RESULT_OUTCOME, CheckoutWindowOutcome.DISMISSED.raw)
        ?: CheckoutWindowOutcome.DISMISSED.raw
      val outcome = CheckoutWindowOutcome.ofRaw(rawOutcome) ?: CheckoutWindowOutcome.DISMISSED
      val reachedUrl = data?.getStringExtra(EXTRA_RESULT_REACHED_URL)
      return CheckoutWindowEvent(outcome, reachedUrl)
    }

    /**
     * TEST-ONLY (Lane E, `docs/plans/2026-09-13-flutter-plugin.md` D1/D5's
     * emulator proof). Not part of `pigeons/checkout.dart`'s frozen
     * contract, and not called from anywhere in this package's own
     * production code -- only from `example/android/app`'s own test-harness
     * `MethodChannel`, itself only present in the example app, never in
     * this plugin.
     *
     * Runs [script] in the currently-open window's `WebView` and hands its
     * string result to [callback], exactly the one-directional native ->
     * JS command `WebView.evaluateJavascript` already is (Android's own
     * public API since API 19). This is **not** `addJavascriptInterface`:
     * nothing here exposes a native object into the page's own JS world in
     * either direction, and no production behaviour changes -- a real payer
     * window never has anything calling this. It exists because a headless
     * emulator has no way to inject a touch event into this Activity's
     * `WebView`, so Lane E's integration test drives the REAL hosted
     * checkout page's own rail/submit/forward buttons and reads its own
     * `data-screen`/`data-outcome` attributes this way instead of by touch.
     *
     * Returns `false` (and never calls [callback]) when no window is open.
     */
    fun evaluateJavascriptForTests(script: String, callback: (String?) -> Unit): Boolean {
      val view = current?.webView ?: return false
      view.evaluateJavascript(script, callback)
      return true
    }
  }

  private var webView: WebView? = null
  private var stopUrls: List<CheckoutStopUrl> = emptyList()

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    current = this

    val url = intent.getStringExtra(EXTRA_URL)
    if (url == null) {
      finishAsDismissed()
      return
    }
    stopUrls =
      (intent.getParcelableArrayListExtra<Bundle>(EXTRA_STOP_URLS) ?: arrayListOf()).map {
        CheckoutStopUrl(
          scheme = it.getString(BUNDLE_SCHEME, ""),
          host = it.getString(BUNDLE_HOST, ""),
          port = it.getLong(BUNDLE_PORT, 0L),
          path = it.getString(BUNDLE_PATH, ""),
        )
      }
    // D6: the extra is not needed again after this point, and must not
    // outlive the window — it carries the session's own credential in its
    // fragment.
    intent.removeExtra(EXTRA_URL)

    val view = WebView(this)
    webView = view
    setContentView(
      view,
      ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT),
    )

    view.settings.apply {
      javaScriptEnabled = true
      domStorageEnabled = true
      allowFileAccess = false
      @Suppress("DEPRECATION") allowFileAccessFromFileURLs = false
      @Suppress("DEPRECATION") allowUniversalAccessFromFileURLs = false
    }
    // No `addJavascriptInterface` call anywhere in this class (design D3).

    view.webViewClient =
      object : WebViewClient() {
        // `onReceivedSslError` is deliberately not overridden here — see
        // this file's header.

        override fun shouldOverrideUrlLoading(
          view: WebView,
          request: WebResourceRequest,
        ): Boolean {
          return interceptIfStopUrl(request.url)
        }

        @Suppress("DEPRECATION")
        override fun shouldOverrideUrlLoading(view: WebView, url: String): Boolean {
          // Pre-N devices (design's minSdk 21) never call the
          // `WebResourceRequest` overload above.
          if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) return false
          return interceptIfStopUrl(Uri.parse(url))
        }
      }

    view.loadUrl(url)

    onBackPressedDispatcher.addCallback(
      this,
      object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() {
          // Always a dismissal, never `webView.goBack()` — see this file's
          // header.
          finishAsDismissed()
        }
      },
    )
  }

  /** `true` (and finishes) when [uri] matches one of [stopUrls]. */
  private fun interceptIfStopUrl(uri: Uri): Boolean {
    val scheme = uri.scheme ?: return false
    val host = uri.host ?: return false
    val port = if (uri.port != -1) uri.port.toLong() else defaultPortFor(scheme)
    val path = uri.path ?: ""
    val matched =
      stopUrls.any {
        it.scheme == scheme && it.host == host && it.port == port && it.path == path
      }
    if (matched) {
      finishAsStopUrlReached(uri.toString())
    }
    return matched
  }

  private fun defaultPortFor(scheme: String): Long =
    when (scheme) {
      "https" -> 443L
      "http" -> 80L
      else -> 0L
    }

  private fun finishAsStopUrlReached(reachedUrl: String) {
    val result =
      Intent().apply {
        putExtra(EXTRA_RESULT_OUTCOME, CheckoutWindowOutcome.STOP_URL_REACHED.raw)
        putExtra(EXTRA_RESULT_REACHED_URL, reachedUrl)
      }
    setResult(RESULT_OK, result)
    finish()
  }

  private fun finishAsDismissed() {
    val result =
      Intent().apply { putExtra(EXTRA_RESULT_OUTCOME, CheckoutWindowOutcome.DISMISSED.raw) }
    setResult(RESULT_OK, result)
    finish()
  }

  override fun onDestroy() {
    // D6: the window's own credential-carrying state does not outlive it.
    webView?.let { view ->
      view.stopLoading()
      view.loadUrl("about:blank")
      view.clearHistory()
      (view.parent as? ViewGroup)?.removeView(view)
      view.destroy()
    }
    webView = null
    if (current === this) {
      current = null
    }
    super.onDestroy()
  }
}
