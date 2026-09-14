// D8's Android external-browser window — Custom Tabs (`androidx.browser`),
// wired 2026-09-14. Unlike `VpayCheckoutActivity` (D5, the in-app WebView),
// this class owns no Activity of its own and cannot: a Custom Tab is a
// separate app's window (the device's default/chosen browser), started with
// a plain `startActivity`, not `startActivityForResult` — Chrome Custom
// Tabs has never supported returning an activity result, and there is no
// scheme callback to wait on either (design doc D8: `checked_forward_url`
// accepts only `http(s)`, and a custom scheme is refused by design —
// schemes are first-come-first-served on Android and any installed app
// could claim one).
//
// So return detection here is D8's own "tier 0": the payer finishes on the
// merchant's own page inside the Custom Tab, backs out or closes it, and
// this class reports the moment the HOST Activity (the Flutter embedding
// activity that launched the tab) itself resumes — the one signal available
// with no merchant deployment work at all. That resume is always reported
// as [CheckoutWindowOutcome.DISMISSED], never `STOP_URL_REACHED` (there is
// no navigation here to watch a stop URL against) — which is
// correctness-neutral by construction: `checkout_controller.dart`'s D4
// polls the real payment intent before answering a dismissal, so a payer
// who actually completed the payment before backing out of the tab still
// resolves to `succeeded`, not `canceled`. D1 stays true: this class never
// decides the outcome, only that the payer is back.
//
// `Application.ActivityLifecycleCallbacks` rather than overriding the host
// Activity's own `onResume` — this plugin does not own that Activity's
// class (it is Flutter's own embedding Activity, or the host app's), so it
// cannot subclass it. Registering on the `Application` is the standard
// workaround for exactly this shape of problem.
package dev.vpay.checkout_flutter

import android.app.Activity
import android.app.Application
import android.content.ActivityNotFoundException
import android.net.Uri
import android.os.Bundle
import androidx.browser.customtabs.CustomTabsIntent

/**
 * One external-browser "window" — really a launch-and-watch session, since
 * this class never holds a reference to the Custom Tab's own Activity (it
 * lives in another app's process). [onEvent] fires at most once, mirroring
 * `VpayCheckoutFlutterApi.onWindowEvent`'s own "exactly once per show"
 * contract.
 */
internal class VpayCheckoutExternalBrowserSession(
  private val application: Application,
  private val hostActivity: Activity,
  private val onEvent: (CheckoutWindowEvent) -> Unit,
) : Application.ActivityLifecycleCallbacks {

  private var pending = false

  /**
   * Launches the Custom Tab loading [url] and starts watching for
   * [hostActivity]'s own resume. Throws [ActivityNotFoundException] if no
   * app on the device can handle it — an emulator image with no browser
   * installed hits this; the caller maps it to a typed [FlutterError]
   * rather than letting a raw platform exception reach Dart with the URL
   * (D6) in its message.
   */
  fun launch(url: String) {
    pending = true
    application.registerActivityLifecycleCallbacks(this)
    try {
      val customTabsIntent = CustomTabsIntent.Builder().build()
      customTabsIntent.launchUrl(hostActivity, Uri.parse(url))
    } catch (e: ActivityNotFoundException) {
      pending = false
      application.unregisterActivityLifecycleCallbacks(this)
      throw e
    }
  }

  /**
   * Best-effort only, and usually a no-op in practice: a Custom Tab is
   * another app's window and there is no API to close it from here. This
   * exists only so [VpayCheckoutFlutterPlugin.dismiss] has something to
   * call; it does not stop watching for the host Activity's resume, which
   * remains the only real return signal.
   */
  fun dismiss() {
    // Deliberately empty — see the doc comment above.
  }

  private fun report(outcome: CheckoutWindowOutcome) {
    if (!pending) return
    pending = false
    application.unregisterActivityLifecycleCallbacks(this)
    onEvent(CheckoutWindowEvent(outcome, null))
  }

  override fun onActivityResumed(activity: Activity) {
    if (activity !== hostActivity) return
    report(CheckoutWindowOutcome.DISMISSED)
  }

  override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {}

  override fun onActivityStarted(activity: Activity) {}

  override fun onActivityPaused(activity: Activity) {}

  override fun onActivityStopped(activity: Activity) {}

  override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}

  override fun onActivityDestroyed(activity: Activity) {}
}
