// The Android host (design doc D5, D8) — Lane C
// (docs/plans/2026-09-13-flutter-plugin-brief.md). This class only wires
// Dart's `VpayCheckoutHostApi` calls to launching/closing either
// `VpayCheckoutActivity` (mode `IN_APP`) or a
// `VpayCheckoutExternalBrowserSession` (mode `EXTERNAL_BROWSER`, D8 —
// Custom Tabs, wired 2026-09-14), and forwards whichever one's result back
// over `VpayCheckoutFlutterApi.onWindowEvent`. It decides nothing about the
// outcome itself (D1) — see those two classes' own headers for where the
// actual window lives in each mode.
package dev.vpay.checkout_flutter

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import io.flutter.plugin.common.PluginRegistry.ActivityResultListener
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/** vpay_checkout_flutter's Android plugin entry point. */
class VpayCheckoutFlutterPlugin :
  FlutterPlugin,
  ActivityAware,
  VpayCheckoutHostApi,
  ActivityResultListener {

  private var flutterApi: VpayCheckoutFlutterApi? = null
  private var activity: Activity? = null
  private var activityBinding: ActivityPluginBinding? = null
  private val scope = CoroutineScope(Dispatchers.Main)

  /** Non-null exactly while a D8 external-browser session is pending a
   * return — see `VpayCheckoutExternalBrowserSession`'s own header. */
  private var externalBrowserSession: VpayCheckoutExternalBrowserSession? = null

  override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
    VpayCheckoutHostApi.setUp(binding.binaryMessenger, this)
    flutterApi = VpayCheckoutFlutterApi(binding.binaryMessenger)
  }

  override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
    VpayCheckoutHostApi.setUp(binding.binaryMessenger, null)
    flutterApi = null
  }

  override fun onAttachedToActivity(binding: ActivityPluginBinding) {
    activity = binding.activity
    activityBinding = binding
    binding.addActivityResultListener(this)
  }

  override fun onReattachedToActivityForConfigChanges(binding: ActivityPluginBinding) {
    onAttachedToActivity(binding)
  }

  override fun onDetachedFromActivityForConfigChanges() {
    onDetachedFromActivity()
  }

  override fun onDetachedFromActivity() {
    activityBinding?.removeActivityResultListener(this)
    activityBinding = null
    activity = null
  }

  // VpayCheckoutHostApi — Dart calling into this host.

  override suspend fun show(request: ShowCheckoutRequest) {
    when (request.mode) {
      CheckoutWindowMode.IN_APP -> showInApp(request)
      CheckoutWindowMode.EXTERNAL_BROWSER -> showExternalBrowser(request)
    }
  }

  private fun showInApp(request: ShowCheckoutRequest) {
    val hostActivity = requireActivity()
    if (VpayCheckoutActivity.isOpen()) {
      throw FlutterError(
        "already_open",
        "vpay_checkout_flutter: a checkout window is already open.",
        null,
      )
    }
    hostActivity.startActivityForResult(
      VpayCheckoutActivity.buildIntent(hostActivity, request),
      VpayCheckoutActivity.REQUEST_CODE,
    )
  }

  /**
   * D8: Custom Tabs. `VpayCheckoutExternalBrowserSession`'s own header
   * explains why this never uses `startActivityForResult` and why an
   * `onResume` on [hostActivity] is the return signal.
   */
  private fun showExternalBrowser(request: ShowCheckoutRequest) {
    val hostActivity = requireActivity()
    if (VpayCheckoutActivity.isOpen() || externalBrowserSession != null) {
      throw FlutterError(
        "already_open",
        "vpay_checkout_flutter: a checkout window is already open.",
        null,
      )
    }
    val session =
      VpayCheckoutExternalBrowserSession(
        application = hostActivity.application,
        hostActivity = hostActivity,
        onEvent = ::reportExternalBrowserEvent,
      )
    externalBrowserSession = session
    try {
      session.launch(request.url)
    } catch (e: ActivityNotFoundException) {
      externalBrowserSession = null
      throw FlutterError(
        "no_browser",
        "vpay_checkout_flutter: no app on this device can open " +
          "VpayCheckoutMode.externalBrowser's Custom Tabs intent.",
        null,
      )
    }
  }

  private fun reportExternalBrowserEvent(event: CheckoutWindowEvent) {
    externalBrowserSession = null
    val api = flutterApi
    if (api != null) {
      scope.launch { api.onWindowEvent(event) }
    }
  }

  private fun requireActivity(): Activity =
    activity
      ?: throw FlutterError(
        "no_activity",
        "vpay_checkout_flutter: show() called with no Activity attached.",
        null,
      )

  override suspend fun dismiss() {
    VpayCheckoutActivity.dismissIfOpen()
    externalBrowserSession?.dismiss()
  }

  // ActivityResultListener — VpayCheckoutActivity reporting back.

  override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?): Boolean {
    if (requestCode != VpayCheckoutActivity.REQUEST_CODE) {
      return false
    }
    val event = VpayCheckoutActivity.eventFromResult(data)
    val api = flutterApi
    if (api != null) {
      scope.launch { api.onWindowEvent(event) }
    }
    return true
  }
}
