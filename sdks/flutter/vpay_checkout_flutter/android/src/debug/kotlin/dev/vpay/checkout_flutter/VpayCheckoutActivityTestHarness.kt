// DEBUG-ONLY (this file lives in `android/src/debug/kotlin`, Gradle's own
// `debug` source set — see `android/build.gradle.kts`'s `sourceSets` block).
// It is compiled into debug builds only and is absent from a release
// build's DEX entirely: `docs/sdks/parity.md`'s Flutter row records the
// `grep -c evaluateJavascriptForTests` count that proves it, both zero in
// a release APK and non-zero in a debug one.
//
// This used to be a public method, `evaluateJavascriptForTests`, on
// `VpayCheckoutActivity`'s own companion object in `src/main/kotlin` —
// compiled into every build variant, including a merchant's release build,
// which made a native -> JS injection point into the payer's live checkout
// `WebView` reachable from release code (see `VpayCheckoutActivity.kt`'s
// own file header). Moving the function here removes it from release
// entirely, but a straight cut-and-paste is not enough on its own: the
// function needs the currently-open `VpayCheckoutActivity`'s `WebView`,
// and `VpayCheckoutActivity.current`/`webView` are (and stay) `private` —
// widening either to `internal` so this file could read them would still
// compile a reachable accessor for that same `WebView` reference into
// every release build of `VpayCheckoutActivity.kt`, reproducing the exact
// defect this file exists to remove, just renamed.
//
// Instead, this file tracks the open window itself, using nothing but
// stock, always-public Android APIs that add no capability to a release
// build that was not already there regardless of this package:
// `Application.ActivityLifecycleCallbacks` (identifying the open window by
// its already-public `VpayCheckoutActivity` type — the very type this
// package's own `buildIntent` already hands a `Context` to start) and
// `Activity.findViewById(android.R.id.content)` (reading back whatever
// `VpayCheckoutActivity.onCreate` passed to `setContentView`, exactly the
// same `WebView` reference, without touching a single private field of
// `VpayCheckoutActivity` itself).
package dev.vpay.checkout_flutter

import android.app.Activity
import android.app.Application
import android.os.Bundle
import android.view.ViewGroup
import android.webkit.WebView

/**
 * Lane E's test-only bridge into the REAL, currently-open
 * [VpayCheckoutActivity]'s `WebView`
 * (`docs/plans/2026-09-13-flutter-plugin.md` D1/D5's emulator proof). Not
 * part of `pigeons/checkout.dart`'s frozen contract, not called from
 * anywhere in this package's own production code, and — since this file
 * lives in Gradle's `debug` source set — not present at all in a release
 * build.
 *
 * `example/android/app/src/debug/kotlin/.../TestHarnessInitProvider.kt`
 * calls [install] once, at debug-build process start, and wires
 * [evaluateJavascriptForTests] to the example app's own test-only
 * `MethodChannel`. A headless emulator has no way to inject a touch event
 * into a separate `Activity`'s `WebView`, so Lane E's integration test
 * drives the REAL hosted checkout page's own rail/submit/forward buttons
 * and reads its own `data-screen`/`data-outcome` attributes this way
 * instead of by touch.
 */
object VpayCheckoutActivityTestHarness {
  @Volatile private var openWindow: Activity? = null

  /** Registers the [Application.ActivityLifecycleCallbacks] that track [openWindow]. */
  fun install(application: Application) {
    application.registerActivityLifecycleCallbacks(
      object : Application.ActivityLifecycleCallbacks {
        override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {
          if (activity is VpayCheckoutActivity) openWindow = activity
        }

        override fun onActivityDestroyed(activity: Activity) {
          if (openWindow === activity) openWindow = null
        }

        override fun onActivityStarted(activity: Activity) {}

        override fun onActivityResumed(activity: Activity) {}

        override fun onActivityPaused(activity: Activity) {}

        override fun onActivityStopped(activity: Activity) {}

        override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
      }
    )
  }

  /**
   * Runs [script] in the currently-open window's `WebView` and hands its
   * string result to [callback], exactly the one-directional native -> JS
   * command `WebView.evaluateJavascript` already is (Android's own public
   * API since API 19). This is **not** `addJavascriptInterface`: nothing
   * here exposes a native object into the page's own JS world in either
   * direction, and no production behaviour changes — a real payer window
   * never has anything calling this, and this file does not even exist in
   * a release build.
   *
   * Returns `false` (and never calls [callback]) when no window is open.
   */
  fun evaluateJavascriptForTests(script: String, callback: (String?) -> Unit): Boolean {
    val view = currentWebView() ?: return false
    view.evaluateJavascript(script, callback)
    return true
  }

  /** The `WebView` `VpayCheckoutActivity.onCreate` passed to `setContentView`, if any. */
  private fun currentWebView(): WebView? {
    val content = openWindow?.findViewById<ViewGroup>(android.R.id.content) ?: return null
    return findWebView(content)
  }

  private fun findWebView(view: android.view.View): WebView? {
    if (view is WebView) return view
    if (view is ViewGroup) {
      for (i in 0 until view.childCount) {
        findWebView(view.getChildAt(i))?.let {
          return it
        }
      }
    }
    return null
  }
}
