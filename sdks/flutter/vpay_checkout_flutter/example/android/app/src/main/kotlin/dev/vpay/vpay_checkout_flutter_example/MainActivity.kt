package dev.vpay.vpay_checkout_flutter_example

import dev.vpay.checkout_flutter.VpayCheckoutActivity
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * Lane E test-only channel: `dev.vpay.checkout_flutter.example/test_harness`
 * (`docs/plans/2026-09-13-flutter-plugin.md` D1/D5's emulator proof).
 *
 * This is the EXAMPLE app, not the plugin -- `pigeons/checkout.dart`'s
 * frozen contract carries no such channel and never will. It exists
 * because a headless emulator has no way to inject a touch event into
 * `VpayCheckoutActivity`'s `WebView`, so `integration_test/` drives the
 * real hosted checkout page's own buttons by running a snippet of
 * JavaScript in it and reading the result back, standing in for touch.
 * `VpayCheckoutActivity.evaluateJavascriptForTests`'s own doc comment says
 * why this is not `addJavascriptInterface` and does not change any
 * production behaviour.
 */
class MainActivity : FlutterActivity() {
  override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
    super.configureFlutterEngine(flutterEngine)
    MethodChannel(
        flutterEngine.dartExecutor.binaryMessenger,
        "dev.vpay.checkout_flutter.example/test_harness",
      )
      .setMethodCallHandler { call, result ->
        when (call.method) {
          "evaluateJavascript" -> {
            val script = call.argument<String>("script")
            if (script == null) {
              result.error("bad_args", "script is required", null)
              return@setMethodCallHandler
            }
            val opened =
              VpayCheckoutActivity.evaluateJavascriptForTests(script) { value ->
                result.success(value)
              }
            if (!opened) {
              result.error("no_window", "no VpayCheckoutActivity is open", null)
            }
          }
          else -> result.notImplemented()
        }
      }
  }
}
