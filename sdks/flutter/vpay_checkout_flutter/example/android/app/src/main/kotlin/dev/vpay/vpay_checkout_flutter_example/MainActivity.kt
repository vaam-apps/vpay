package dev.vpay.vpay_checkout_flutter_example

import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * The generic, always-compiled (every build type, including release) half
 * of Lane E test-only channel:
 * `dev.vpay.checkout_flutter.example/test_harness`
 * (`docs/plans/2026-09-13-flutter-plugin.md` D1/D5's emulator proof).
 *
 * [evaluateJavascript] starts `null` and answers every call with the
 * `no_window` error, in every build type. Only
 * `example/android/app/src/debug/kotlin/.../TestHarnessInitProvider.kt` —
 * present in a debug build only — ever sets it, to
 * `VpayCheckoutActivityTestHarness.evaluateJavascriptForTests`
 * (`dev.vpay.checkout_flutter`'s own `debug` source set, absent from a
 * release build of the plugin entirely). This file deliberately never
 * imports `VpayCheckoutActivity` or anything plugin-internal, so a release
 * build of this EXAMPLE app compiles cleanly even though the plugin's own
 * release build carries none of the test harness.
 */
object TestHarnessBridge {
  var evaluateJavascript: ((String, (String?) -> Unit) -> Boolean)? = null
}

/**
 * This is the EXAMPLE app, not the plugin -- `pigeons/checkout.dart`'s
 * frozen contract carries no such channel and never will. It exists
 * because a headless emulator has no way to inject a touch event into
 * `VpayCheckoutActivity`'s `WebView`, so `integration_test/` drives the
 * real hosted checkout page's own buttons by running a snippet of
 * JavaScript in it and reading the result back, standing in for touch --
 * see [TestHarnessBridge]'s own doc comment for why the actual JS-eval
 * capability lives only in a debug build, both of the plugin and of this
 * example app.
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
              TestHarnessBridge.evaluateJavascript?.invoke(script) { value ->
                result.success(value)
              } ?: false
            if (!opened) {
              result.error("no_window", "no VpayCheckoutActivity is open", null)
            }
          }
          else -> result.notImplemented()
        }
      }
  }
}
