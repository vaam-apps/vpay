// The Android native window itself (design doc D5, revised again
// 2026-09-16 — "browser, not WebView", same day as the "modal checkout
// sheet" revision this file's own history already carried). This class no
// longer renders anything of its own: it launches the payer's browser as a
// partial (bottom sheet) Custom Tab and watches for either the tab closing
// with no matching deep link, or an App Link return forwarded to it by
// VpayCheckoutAppLinkActivity (this package's one exported entry point —
// see that file's own header for the full security tradeoff). This class
// still decides nothing about the payer's payment outcome (D1): it only
// reports which of those two things happened.
// `checkout_controller.dart`'s poll of
// `GET /v1/browser/payment_intents/{id}` is the only thing that ever says
// `succeeded`.
//
// Previous revisions of this file built a `WebView` laid out as a Material
// modal bottom sheet (`CoordinatorLayout`/`BottomSheetBehavior`) — that
// machinery, and the `VpayCheckoutExternalBrowserSession` class that used
// to hold the separate "external browser" mode's Custom Tab launch, are
// both gone as of this revision: `ShowCheckoutRequest` no longer carries a
// `mode` at all (`CheckoutWindowMode` does not exist any more), because
// there is exactly one window shape now — the payer's own browser process,
// never this app's `WebView` — and this Activity is that shape's only
// remaining Android surface. See `Messages.g.kt`'s own doc comments
// (`VpayCheckoutHostApi.show`, `CheckoutWindowOutcome`) for why: a
// `WebView` puts the payment form inside the merchant app's own process,
// where `evaluateJavascript`, the cookie store and a navigation delegate
// are all reachable — i.e. where a compromised merchant app could read the
// payer's PAN and OTP without the payer being able to tell. The browser's
// process cannot be inspected that way.
//
// The cost is stated rather than hidden: no host can see a navigation
// inside the browser any more, so a "stop URL reached" can only ever
// arrive as an **incoming** deep link (App Link), never an intercepted
// one — see [interceptIfStopUrl] and `VpayCheckoutAppLinkActivity`.
//
// Non-negotiables that survive this revision
// (docs/plans/2026-09-13-flutter-plugin-brief.md, ADR-0021/D8):
// - `android:exported="false"` in AndroidManifest.xml — this Activity is
//   never started by anything outside this app. The App Link forwarder
//   (`VpayCheckoutAppLinkActivity`) is a deliberate, narrow, separately
//   exported component for exactly the one thing an `exported="false"`
//   Activity cannot do (receive an external Intent at all) — see that
//   file's header for the tradeoff this buys and costs.
// - No custom URL scheme anywhere in this plugin (ADR-0021/D8): a scheme
//   is first-come-first-served on Android and any installed app could
//   claim one. `stopUrls` are matched against `https`/`http` App Links
//   only.
// - `dismiss()`/`show()` report **exactly one** [CheckoutWindowEvent] per
//   show — the `reported` guard below, checked by both finish paths.
// - This file lives in `src/main/kotlin`, so it is compiled into EVERY
//   build variant, including a merchant's release build.
package dev.vpay.checkout_flutter

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.browser.customtabs.CustomTabsIntent

/** The Intent extra carrying `ShowCheckoutRequest.url` — cleared on finish. */
private const val EXTRA_URL = "vpay_checkout_url"
private const val EXTRA_STOP_URLS = "vpay_checkout_stop_urls"
private const val EXTRA_RESULT_OUTCOME = "vpay_checkout_result_outcome"
private const val EXTRA_RESULT_REACHED_URL = "vpay_checkout_result_reached_url"

/** Persisted across a recreation (config change, or process death) so a
 *  Custom Tab already in front never gets launched a second time. */
private const val STATE_CUSTOM_TAB_LAUNCHED = "vpay_checkout_custom_tab_launched"

private const val BUNDLE_SCHEME = "scheme"
private const val BUNDLE_HOST = "host"
private const val BUNDLE_PORT = "port"
private const val BUNDLE_PATH = "path"

/**
 * The large detent the maintainer asked for explicitly (D5-rev1,
 * 2026-09-16, "a modal bottom sheet, not a full-screen window"): the
 * hosted page is a form, and a shorter detent would push its own "Pay"
 * button under the fold. Fed to
 * `CustomTabsIntent.Builder.setInitialActivityHeightPx` as a fraction of
 * the real display height (never a hardcoded pixel value — screen size and
 * density vary across the `minSdk` 21 floor's actual device range, D-M4),
 * with `ACTIVITY_HEIGHT_ADJUSTABLE` so the sheet stays draggable up to
 * full height exactly like the old `BottomSheetBehavior` detent was.
 */
private const val SHEET_INITIAL_HEIGHT_RATIO = 0.9f

/** Rounded top corners on the Custom Tab sheet. */
private const val SHEET_CORNER_RADIUS_DP = 16

class VpayCheckoutActivity : ComponentActivity() {

  companion object {
    const val REQUEST_CODE = 0x76706179 // 'vpay' in hex, arbitrarily.

    /** The single open instance, if any — cleared in [onDestroy]. */
    private var current: VpayCheckoutActivity? = null

    fun isOpen(): Boolean = current != null

    fun dismissIfOpen() {
      current?.finishAsDismissed()
    }

    /**
     * Called by [VpayCheckoutAppLinkActivity] — the plugin's one exported
     * entry point — with the URI of an incoming App Link. A no-op (return
     * `false`) if no window is open, or if [uri] does not match any of the
     * open window's `stopUrls`; otherwise finishes that window as
     * [CheckoutWindowOutcome.stopUrlReached], exactly the one dismissal-vs-
     * stop-url branch [interceptIfStopUrl] already implements.
     */
    fun reportAppLinkIfOpen(uri: Uri): Boolean = current?.interceptIfStopUrl(uri) ?: false

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
  }

  private var stopUrls: List<CheckoutStopUrl> = emptyList()

  /** Guards "exactly one [CheckoutWindowEvent] per show" — both finish
   *  paths below check it, since [finishAsStopUrlReached] (driven by
   *  [reportAppLinkIfOpen]) and [finishAsDismissed] (driven by [onResume])
   *  can otherwise race against each other. */
  private var reported = false

  /** Persisted via [onSaveInstanceState] — see that method's own comment. */
  private var customTabLaunched = false

  /** NOT persisted: true once this specific instance has seen its first
   *  [onResume] since launching the Custom Tab. Only a *second* [onResume]
   *  means the tab actually closed — the first one is `onCreate`'s own
   *  `onStart`/`onResume` pass, moments before the Custom Tab covers this
   *  Activity. */
  private var resumedSinceLaunch = false

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    current = this

    stopUrls = parseStopUrls(intent)
    customTabLaunched = savedInstanceState?.getBoolean(STATE_CUSTOM_TAB_LAUNCHED) ?: false

    onBackPressedDispatcher.addCallback(
      this,
      object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() {
          // This Activity paints nothing of its own, so in practice there
          // is only a narrow window where it is ever visible enough to
          // receive a back press at all — but when it does, it is the same
          // dismissal path everything else here converges on.
          finishAsDismissed()
        }
      },
    )

    if (customTabLaunched) {
      // Recreated — a config change, or a fresh process after this one
      // died in the background — while the Custom Tab was already
      // showing. Do not launch a second one; just resume watching via
      // onResume/reportAppLinkIfOpen. Best-effort only: a process death
      // recreation's first onResume is (again) treated as "just launched,
      // not yet a return" below, which is correct for the config-change
      // case and unverified for the process-death one — this repository
      // has no device to prove it against either way.
      return
    }

    val url = intent.getStringExtra(EXTRA_URL)
    // D6: this extra carries the checkout session's own credential (in its
    // fragment) and must not outlive this point, win or lose.
    intent.removeExtra(EXTRA_URL)
    if (url == null) {
      // Nothing was ever shown — nothing to wait for.
      finishAsDismissed()
      return
    }

    if (!launchCustomTab(url)) {
      // No app on this device can open even a plain https URL —
      // `CustomTabsIntent.launchUrl` throws `ActivityNotFoundException` in
      // exactly that case (an AOSP-emulator-with-no-browser scenario in
      // practice; every shipping device ships one). The old
      // EXTERNAL_BROWSER-only path could reject `show()` synchronously
      // with a typed "no_browser" FlutterError, because it launched the
      // Custom Tab inline inside the plugin's own `show()` call. That is
      // no longer possible now that launching is this Activity's own job:
      // `startActivityForResult` — what `show()` actually waits on — has
      // already returned successfully by the time this
      // Activity's `onCreate` runs, on a later main-thread message-loop
      // pass. `CheckoutWindowOutcome` has no error member either, so the
      // closest honest signal left is an ordinary dismissal rather than a
      // silently invented success. Stated here because it is a genuine,
      // deliberate behavior change from the old `no_browser` error, not an
      // oversight.
      finishAsDismissed()
      return
    }
    customTabLaunched = true
  }

  override fun onResume() {
    super.onResume()
    if (!customTabLaunched) {
      // Either still inside onCreate's own first pass (finishing already,
      // one way or another), or recreated without ever having launched —
      // nothing to detect a return from yet.
      return
    }
    if (!resumedSinceLaunch) {
      resumedSinceLaunch = true
      return
    }
    // A second resume since the Custom Tab was launched: it is no longer
    // in front. If a matching App Link had arrived first,
    // [reportAppLinkIfOpen] would already have finished this Activity as
    // [CheckoutWindowOutcome.stopUrlReached] and set [reported] — the
    // guard below makes whichever of the two happens first win, race-free.
    finishAsDismissed()
  }

  override fun onSaveInstanceState(outState: Bundle) {
    super.onSaveInstanceState(outState)
    outState.putBoolean(STATE_CUSTOM_TAB_LAUNCHED, customTabLaunched)
  }

  private fun parseStopUrls(intent: Intent): List<CheckoutStopUrl> =
    (intent.getParcelableArrayListExtra<Bundle>(EXTRA_STOP_URLS) ?: arrayListOf()).map {
      CheckoutStopUrl(
        scheme = it.getString(BUNDLE_SCHEME, ""),
        host = it.getString(BUNDLE_HOST, ""),
        port = it.getLong(BUNDLE_PORT, 0L),
        path = it.getString(BUNDLE_PATH, ""),
      )
    }

  /**
   * Launches [url] as a partial (bottom sheet) Custom Tab.
   * `setInitialActivityHeightPx` (`androidx.browser` 1.6+, confirmed
   * present in the 1.8.0 this module already depends on) is what turns an
   * ordinary full-screen Custom Tab into the bottom sheet shape D5-rev1
   * asked for — the same ~90%-of-the-screen, draggable-to-full-height
   * detent the old `WebView`/`BottomSheetBehavior` sheet used, computed
   * from the real display metrics rather than a hardcoded pixel value.
   *
   * Returns `false` (having launched nothing) if no app on the device can
   * open even a plain https URL — see the `onCreate` caller's own comment
   * for what that means for the caller.
   */
  private fun launchCustomTab(url: String): Boolean {
    val heightPx = (resources.displayMetrics.heightPixels * SHEET_INITIAL_HEIGHT_RATIO).toInt()
    val customTabsIntent =
      CustomTabsIntent.Builder()
        .setInitialActivityHeightPx(heightPx, CustomTabsIntent.ACTIVITY_HEIGHT_ADJUSTABLE)
        .setToolbarCornerRadiusDp(SHEET_CORNER_RADIUS_DP)
        // Google's own partial-Custom-Tabs guidance recommends the close
        // button at the START of the toolbar in bottom-sheet mode — the
        // default (END) position sits where the sheet's own drag
        // handle/maximize affordance is.
        .setCloseButtonPosition(CustomTabsIntent.CLOSE_BUTTON_POSITION_START)
        .build()
    return try {
      customTabsIntent.launchUrl(this, Uri.parse(url))
      true
    } catch (e: ActivityNotFoundException) {
      false
    }
  }

  /**
   * `true` (and finishes this Activity as
   * [CheckoutWindowOutcome.stopUrlReached]) when [uri] matches one of
   * [stopUrls] — scheme+host+port+path only, query and fragment ignored
   * (D2). The only caller is [reportAppLinkIfOpen], forwarding from
   * `VpayCheckoutAppLinkActivity`: there is no in-process navigation to
   * check against any more (this file's own header), only an incoming
   * deep link.
   */
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
    if (reported) return
    reported = true
    val result =
      Intent().apply {
        putExtra(EXTRA_RESULT_OUTCOME, CheckoutWindowOutcome.STOP_URL_REACHED.raw)
        putExtra(EXTRA_RESULT_REACHED_URL, reachedUrl)
      }
    setResult(RESULT_OK, result)
    finish()
  }

  private fun finishAsDismissed() {
    if (reported) return
    reported = true
    val result =
      Intent().apply { putExtra(EXTRA_RESULT_OUTCOME, CheckoutWindowOutcome.DISMISSED.raw) }
    setResult(RESULT_OK, result)
    finish()
  }

  override fun onDestroy() {
    if (current === this) {
      current = null
    }
    super.onDestroy()
  }
}
