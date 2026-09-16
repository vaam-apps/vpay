package dev.vpay.vpay_checkout_flutter_example

import io.flutter.embedding.android.FlutterActivity

/**
 * This is the EXAMPLE app, not the plugin. It used to also host Lane E's
 * test-only `dev.vpay.checkout_flutter.example/test_harness` channel (a
 * `TestHarnessBridge` object plus a `MethodChannel` wired to it here),
 * which let `integration_test/checkout_window_test.dart` run a snippet of
 * JavaScript in `VpayCheckoutActivity`'s `WebView` and read the result
 * back, standing in for a touch event a headless emulator has no way to
 * inject.
 *
 * Retired 2026-09-16 along with that `WebView`: the plugin now launches
 * the payer's browser (a Custom Tab on Android) instead of rendering the
 * checkout page in-process, so there is no native `WebView` left for a
 * JS-evaluation channel to reach into. See
 * `VpayCheckoutActivity.kt`'s own file header and
 * `docs/status/mobile-flutter-plugin.md`.
 */
class MainActivity : FlutterActivity()
