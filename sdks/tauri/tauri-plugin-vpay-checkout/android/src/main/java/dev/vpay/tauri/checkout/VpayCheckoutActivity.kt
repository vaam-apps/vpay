// The Android native window itself (design doc D5, revised 2026-09-16 —
// "browser, not WebView"), ported from
// `sdks/flutter/vpay_checkout_flutter/android/src/main/kotlin/dev/vpay/checkout_flutter/VpayCheckoutActivity.kt`
// onto Tauri v2's Android plugin API. The host framework changed; not one
// of the security decisions below did, and the reasoning is carried over
// with them rather than left behind in the Flutter package.
//
// This class renders nothing of its own: it launches the payer's browser as
// a partial (bottom sheet) Custom Tab and watches for either the tab
// closing with no matching deep link, or an App Link return forwarded to it
// by VpayCheckoutAppLinkActivity (this package's one exported entry point —
// see that file's own header for the full security tradeoff). It decides
// nothing about the payer's payment outcome (D1): it only reports which of
// those two things happened. The guest-JS state machine's poll of
// `GET /v1/browser/payment_intents/{id}` is the only thing that ever says
// `succeeded`.
//
// Why a browser and not a WebView, restated because the reason is the whole
// design: a `WebView` puts the payment form inside the merchant app's own
// process, where `evaluateJavascript`, the cookie store and a navigation
// delegate are all reachable — i.e. where a compromised merchant app could
// read the payer's PAN and OTP without the payer being able to tell. The
// browser's process cannot be inspected that way, and the payer gets a real
// URL bar. For a Tauri app this also rules out the obvious-looking shortcut
// of a second `WebviewWindow`: that is the merchant's own webview process
// again, with the same reachability, so it is banned here for exactly the
// same reason a `WebView` is (D5 rev. 2026-09-16). There is no `WebView`,
// no `WKWebView` and no Tauri webview anywhere in this module.
//
// The cost is stated rather than hidden: no host can see a navigation
// inside the browser, so a "stop URL reached" can only ever arrive as an
// **incoming** deep link (App Link), never an intercepted one — see
// [VpayCheckoutActivity.interceptIfStopUrl] and VpayCheckoutAppLinkActivity.
//
// Non-negotiables (docs/plans/2026-09-22-tauri-plugin-brief.md, which
// inherits ADR-0021/D1-D9 unchanged):
// - `android:exported="false"` in AndroidManifest.xml — this Activity is
//   never started by anything outside this app. It is launched only by this
//   same app's own Tauri plugin, `VpayCheckoutPlugin.show`. The App Link
//   forwarder (`VpayCheckoutAppLinkActivity`) is a deliberate, narrow,
//   separately exported component for exactly the one thing an
//   `exported="false"` Activity cannot do (receive an external Intent at
//   all) — see that file's header for the tradeoff this buys and costs.
// - No custom URL scheme anywhere in this plugin (ADR-0021/D8): a scheme is
//   first-come-first-served on Android and any installed app could claim
//   one. `stopUrls` are matched against `https`/`http` App Links only.
// - Exactly **one** [CheckoutWindowEvent] per `show` — the `reported` guard
//   below, checked by both finish paths, is half of that guarantee;
//   `VpayCheckoutPlugin`'s own single-flight `Channel` field is the other.
// - No `Log`, no `println`, no `toString` of the URL anywhere in this file
//   (D6). The URL extra carries the checkout session's secret in its
//   fragment.
// - This file lives in `src/main/java`, so it is compiled into EVERY build
//   variant, including a merchant's release build.
package dev.vpay.tauri.checkout

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.browser.customtabs.CustomTabsIntent

/**
 * One stop URL, already normalised by the guest-JS state machine the way
 * `checkout_controller.dart`'s `StopUrlSpec` is: scheme, host, port and
 * path only. Query and fragment are ignored (D2), so they never cross the
 * wire and there is nothing here a host could compare against them by
 * mistake.
 *
 * `port` is a `Long` rather than an `Int` because that is the width the
 * cross-language contract fixes (`CheckoutStopUrl.port` is a pigeon `int`
 * → Kotlin `Long` in the Flutter host, and a JSON number on Tauri's wire);
 * keeping the same width keeps the comparison in [interceptIfStopUrl]
 * literal.
 */
internal data class CheckoutStopUrl(
  val scheme: String,
  val host: String,
  val port: Long,
  val path: String,
)

/**
 * Which of the two signals the guest-JS poll will resolve actually
 * happened. These are the exact strings the wire contract fixes
 * (docs/plans/2026-09-22-tauri-plugin-brief.md, "The one event"), spelled
 * once here so the Intent extra, the `Channel` payload and any future
 * reader cannot drift apart.
 *
 * Never a `succeeded`/`canceled`/`failed` member — the design's whole point
 * (D1) is that this interface *cannot* say that; only the poll of
 * `/v1/browser/payment_intents/{id}` can.
 */
internal object CheckoutWindowOutcome {
  /**
   * An **incoming deep link** (Android App Link) matched one of
   * `stopUrls`, bringing the app back to the foreground.
   *
   * **Unverified**, and this repository's status pages say so: a verified
   * App Link needs an HTTPS origin serving `assetlinks.json` for the
   * merchant's own `success_url` host, which this repository can neither
   * deploy nor prove against. Every checkout therefore reports [DISMISSED]
   * in practice today, and D1/D4 make that correctness-complete — the
   * poll, not the window, decides. Nothing here pretends otherwise.
   */
  const val STOP_URL_REACHED = "stopUrlReached"

  /**
   * The payer closed the browser sheet (back press, swipe, "Done") with no
   * matching deep link having arrived. Since D5's 2026-09-16 revision this
   * is the **ordinary** end of a successful payment too, not only a
   * cancellation — which is exactly why D4 polls before answering.
   */
  const val DISMISSED = "dismissed"
}

/**
 * The one event this Activity reports back, read out of the result Intent
 * by [VpayCheckoutActivity.eventFromResult] and sent on the invoke's
 * `Channel` by `VpayCheckoutPlugin`.
 *
 * [reachedUrl] is present only for
 * [CheckoutWindowOutcome.STOP_URL_REACHED], and is carried for diagnostics
 * only — guest-JS reads nothing off it (D1). It is a merchant's own public
 * success/cancel URL, never the session URL, so carrying it does not widen
 * D6's surface.
 */
internal data class CheckoutWindowEvent(
  val outcome: String,
  val reachedUrl: String?,
)

/** The Intent extra carrying `show`'s `url` — cleared the moment it is read. */
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
 * 2026-09-16, "a modal bottom sheet, not a full-screen window"): the hosted
 * page is a form, and a shorter detent would push its own "Pay" button
 * under the fold. Fed to
 * `CustomTabsIntent.Builder.setInitialActivityHeightPx` as a fraction of
 * the real display height (never a hardcoded pixel value — screen size and
 * density vary across the `minSdk` 21 floor's actual device range, D-M4),
 * with `ACTIVITY_HEIGHT_ADJUSTABLE` so the sheet stays draggable up to full
 * height.
 */
private const val SHEET_INITIAL_HEIGHT_RATIO = 0.9f

/** Rounded top corners on the Custom Tab sheet. */
private const val SHEET_CORNER_RADIUS_DP = 16

class VpayCheckoutActivity : ComponentActivity() {

  companion object {
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
     * [CheckoutWindowOutcome.STOP_URL_REACHED], exactly the one
     * dismissal-vs-stop-url branch [interceptIfStopUrl] already implements.
     */
    fun reportAppLinkIfOpen(uri: Uri): Boolean = current?.interceptIfStopUrl(uri) ?: false

    internal fun buildIntent(
      context: Context,
      url: String,
      stopUrls: List<CheckoutStopUrl>,
    ): Intent {
      val bundles = ArrayList<Bundle>(stopUrls.size)
      for (stopUrl in stopUrls) {
        bundles.add(
          Bundle().apply {
            putString(BUNDLE_SCHEME, stopUrl.scheme)
            putString(BUNDLE_HOST, stopUrl.host)
            putLong(BUNDLE_PORT, stopUrl.port)
            putString(BUNDLE_PATH, stopUrl.path)
          },
        )
      }
      return Intent(context, VpayCheckoutActivity::class.java).apply {
        putExtra(EXTRA_URL, url)
        putParcelableArrayListExtra(EXTRA_STOP_URLS, bundles)
      }
    }

    /**
     * Reads the result `VpayCheckoutPlugin` turns into the one `Channel`
     * message. Defaults to [CheckoutWindowOutcome.DISMISSED] for every
     * shape of missing or unrecognised data, including a `null` Intent (a
     * `RESULT_CANCELED` the system delivers when the Activity is destroyed
     * without calling `setResult`): "the window closed and told us nothing"
     * is a dismissal, and a dismissal is the one outcome that costs
     * nothing to be wrong about, because D4 polls either way.
     */
    internal fun eventFromResult(data: Intent?): CheckoutWindowEvent {
      val outcome = when (data?.getStringExtra(EXTRA_RESULT_OUTCOME)) {
        CheckoutWindowOutcome.STOP_URL_REACHED -> CheckoutWindowOutcome.STOP_URL_REACHED
        else -> CheckoutWindowOutcome.DISMISSED
      }
      val reachedUrl =
        if (outcome == CheckoutWindowOutcome.STOP_URL_REACHED) {
          data?.getStringExtra(EXTRA_RESULT_REACHED_URL)
        } else {
          null
        }
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
      // Recreated — a config change, or a fresh process after this one died
      // in the background — while the Custom Tab was already showing. Do
      // not launch a second one; just resume watching via
      // onResume/reportAppLinkIfOpen. Best-effort only: a process death
      // recreation's first onResume is (again) treated as "just launched,
      // not yet a return" below, which is correct for the config-change
      // case and unverified for the process-death one — this repository has
      // no device to prove it against either way.
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
      // practice; every shipping device ships one).
      //
      // `show` cannot be rejected from here. Tauri's
      // `Plugin.startActivityForResult` hands the Intent straight to the
      // app's `ActivityResultLauncher`
      // (tauri-apps/tauri@tauri-v2.11.6
      // `crates/tauri/mobile/android/src/main/java/app/tauri/plugin/PluginManager.kt:131-134`),
      // which has already returned by the time this Activity's `onCreate`
      // runs on a later main-thread message-loop pass — and
      // `VpayCheckoutPlugin.show` has already resolved, because the
      // contract says it resolves once the window is showing.
      // [CheckoutWindowOutcome] has no error member either, so the closest
      // honest signal left is an ordinary dismissal rather than a silently
      // invented success. D4's poll then answers from the real payment
      // state, which for a checkout the payer never saw is `pending` or
      // whatever the intent actually says — not a fabricated outcome.
      // Stated here because it is a genuine, deliberate limitation, not an
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
    // A second resume since the Custom Tab was launched: it is no longer in
    // front. If a matching App Link had arrived first,
    // [reportAppLinkIfOpen] would already have finished this Activity as
    // [CheckoutWindowOutcome.STOP_URL_REACHED] and set [reported] — the
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
   * `setInitialActivityHeightPx` (`androidx.browser` 1.6+, present in the
   * 1.8.0 this module depends on) is what turns an ordinary full-screen
   * Custom Tab into the bottom sheet shape D5-rev1 asked for — a
   * ~90%-of-the-screen, draggable-to-full-height detent computed from the
   * real display metrics rather than a hardcoded pixel value.
   *
   * Returns `false` (having launched nothing) if no app on the device can
   * open even a plain https URL — see the `onCreate` caller's own comment
   * for what that means for the caller.
   *
   * [url] is a parameter and never a field, is never logged, and is never
   * interpolated into any message (D6).
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
      // Swallowed deliberately and not logged: the exception's message
      // embeds the Intent, and the Intent's data is the session URL (D6).
      false
    }
  }

  /**
   * `true` (and finishes this Activity as
   * [CheckoutWindowOutcome.STOP_URL_REACHED]) when [uri] matches one of
   * [stopUrls] — scheme+host+port+path only, query and fragment ignored
   * (D2). The only caller is [reportAppLinkIfOpen], forwarding from
   * `VpayCheckoutAppLinkActivity`: there is no in-process navigation to
   * check against (this file's own header), only an incoming deep link.
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
        putExtra(EXTRA_RESULT_OUTCOME, CheckoutWindowOutcome.STOP_URL_REACHED)
        putExtra(EXTRA_RESULT_REACHED_URL, reachedUrl)
      }
    setResult(RESULT_OK, result)
    finish()
  }

  private fun finishAsDismissed() {
    if (reported) return
    reported = true
    val result =
      Intent().apply { putExtra(EXTRA_RESULT_OUTCOME, CheckoutWindowOutcome.DISMISSED) }
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
