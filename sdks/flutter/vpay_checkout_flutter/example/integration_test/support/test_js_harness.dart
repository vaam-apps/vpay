/// Lane E's test-only bridge into the REAL, currently-open
/// `VpayCheckoutActivity`'s `WebView` — the Dart half of
/// `dev.vpay.checkout_flutter.example/test_harness`
/// (`example/android/app/src/main/kotlin/.../MainActivity.kt`).
///
/// **Not part of `vpay_checkout_flutter`'s public API and never imported by
/// it.** This channel exists only in the example app, only for this test
/// suite: a headless emulator has no way to inject a touch event into a
/// separate Android `Activity`'s `WebView`, so this drives the REAL hosted
/// checkout page's own rail/submit/forward buttons with a snippet of
/// JavaScript instead — a real DOM `.click()`/`.value =`, executed by the
/// REAL WebView's REAL JS engine on the REAL page vpay served, and reads a
/// real result back. See
/// `VpayCheckoutActivityTestHarness.evaluateJavascriptForTests`'s own doc
/// comment (`android/src/debug/kotlin/dev/vpay/checkout_flutter/
/// VpayCheckoutActivityTestHarness.kt` — Gradle's own `debug` source set,
/// absent from a release build) for why this is not `addJavascriptInterface`
/// and changes no production behaviour.
library;

import 'package:flutter/services.dart';

/// Thrown when [TestJsHarness.eval] is called with no
/// `VpayCheckoutActivity` window open — a harness bug, not a page state.
class NoCheckoutWindowOpen implements Exception {
  const NoCheckoutWindowOpen();

  @override
  String toString() =>
      'NoCheckoutWindowOpen: no VpayCheckoutActivity is open to run '
      'JavaScript in.';
}

class TestJsHarness {
  TestJsHarness()
    : _channel = const MethodChannel(
        'dev.vpay.checkout_flutter.example/test_harness',
      );

  final MethodChannel _channel;

  /// Runs [script] in the currently-open checkout window's `WebView` and
  /// returns whatever `WebView.evaluateJavascript`'s own callback receives
  /// — a JSON-encoded string for most expressions (Android's own
  /// behaviour: a JS string result arrives quoted, `"select_rail"` rather
  /// than `select_rail`; callers that want the bare text use [evalString]).
  Future<String?> eval(String script) async {
    try {
      return await _channel.invokeMethod<String>('evaluateJavascript', {
        'script': script,
      });
    } on PlatformException catch (e) {
      if (e.code == 'no_window') {
        throw const NoCheckoutWindowOpen();
      }
      rethrow;
    }
  }

  /// [eval], with the JSON string-literal quoting `evaluateJavascript`
  /// hands back for a JS string expression stripped, and `"null"`/`null`
  /// folded to Dart `null` — what every caller in this suite actually
  /// wants when reading a `data-*` attribute or `document.title`.
  Future<String?> evalString(String script) async {
    final String? raw = await eval(script);
    if (raw == null || raw == 'null') {
      return null;
    }
    if (raw.length >= 2 && raw.startsWith('"') && raw.endsWith('"')) {
      return jsonUnquote(raw);
    }
    return raw;
  }

  /// Minimal JSON string-literal decoder for exactly the shape
  /// `evaluateJavascript` produces (Android's own JSON encoder) — avoids a
  /// dependency on `dart:convert`'s full `jsonDecode` for what is always a
  /// single already-known-to-be-a-string-literal value.
  static String jsonUnquote(String literal) {
    final String inner = literal.substring(1, literal.length - 1);
    final StringBuffer out = StringBuffer();
    for (int i = 0; i < inner.length; i++) {
      final String c = inner[i];
      if (c == r'\' && i + 1 < inner.length) {
        i++;
        final String next = inner[i];
        switch (next) {
          case 'n':
            out.write('\n');
          case 't':
            out.write('\t');
          case 'r':
            out.write('\r');
          case '"':
            out.write('"');
          case r'\':
            out.write(r'\');
          default:
            out.write(next);
        }
      } else {
        out.write(c);
      }
    }
    return out.toString();
  }
}
