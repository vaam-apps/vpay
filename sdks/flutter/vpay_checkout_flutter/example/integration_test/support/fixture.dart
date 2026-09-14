/// What `just test-flutter-emulator` mints on the HOST and writes to
/// `$VPAY_E2E_FIXTURE_FILE` before running this suite ON THE EMULATOR —
/// the same shape Lane D's `test_e2e/real_stack_e2e_test.dart` reads,
/// trimmed to what a WebView-driving suite needs (no pre-expired session:
/// this suite drives the REAL page's own confirm, it never calls
/// `/confirm` itself).
///
/// Loading this is a loud [fail] in `setUpAll`, never a skip — a skipped
/// test is not a passing test (CLAUDE.md).
library;

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

class EmulatorFixture {
  const EmulatorFixture({
    required this.baseUrl,
    required this.publishableKey,
    required this.sessionUrl,
    required this.dismissSessionUrl,
    required this.mtnSucceedsMsisdn,
  });

  /// vpay's own API origin AS REACHABLE FROM INSIDE THE EMULATOR —
  /// `http://localhost:8080`, made to resolve to the host's real
  /// `localhost:8080` by `adb reverse tcp:8080 tcp:8080`
  /// (`justfile`'s `test-flutter-emulator`). The emulator's own
  /// `10.0.2.2` alias is not used here on purpose: the checkout page's
  /// own `NEXT_PUBLIC_VPAY_API_URL` is baked as `http://localhost:8080`
  /// (`compose.demo.yml`), and the page's client-side JS has to resolve
  /// that same string — `adb reverse` makes "localhost" mean the same
  /// thing to the WebView's JS and to this suite's own `BrowserClient`
  /// calls, where rewriting only the top-level URL's host would not.
  final String baseUrl;

  /// The merchant's publishable key the minted [sessionUrl] carries.
  final String publishableKey;

  /// A FRESH, still-`open`, UNCONFIRMED hosted session's `url` —
  /// `{checkoutOrigin}/c/{cs_id}?key=...#{secret}` — minted through
  /// `examples/shop`'s real server exactly as
  /// `test_e2e/real_stack_e2e_test.dart`'s fixture is, with its host
  /// already `localhost:3080` (`adb reverse tcp:3080 tcp:3080` makes that
  /// reachable from the emulator too). Deliberately NOT pre-confirmed:
  /// this suite drives the REAL page's own rail-select/msisdn/submit UI to
  /// confirm it, which is the thing under test.
  final String sessionUrl;

  /// A SECOND, independently-minted hosted session's `url`, used only by
  /// `checkout_dismiss_test.dart` — a separate session so that file never
  /// depends on `checkout_window_test.dart` having run first (or at all):
  /// each `flutter test integration_test/<file>.dart` invocation is its
  /// own process, and sharing one session across both would make the
  /// dismiss suite's pass/fail depend on run order.
  final String dismissSessionUrl;

  /// `frontends/tests/e2e/cypress/support/shop.ts`'s `MTN.succeeds` —
  /// `PENDING` on the first status query, `SUCCESSFUL` on the next
  /// (WireMock scenario `mtn-e2e-poll`). Digits-only: vpay's page
  /// validates Cameroon E.164 and refuses the hex-suffixed steering
  /// numbers other suites use.
  final String mtnSucceedsMsisdn;

  static EmulatorFixture load() {
    final String? path = Platform.environment['VPAY_E2E_FIXTURE_FILE'];
    if (path == null || path.trim().isEmpty) {
      fail(
        'VPAY_E2E_FIXTURE_FILE is not set. This is a REAL emulator suite '
        "and refuses to run against nothing — it never skips. Run it "
        "through 'just test-flutter-emulator', which mints this fixture "
        'against a running vpay stack, arms adb reverse, and sets this '
        'variable — see the justfile.',
      );
    }
    final File file = File(path);
    if (!file.existsSync()) {
      fail('VPAY_E2E_FIXTURE_FILE names $path, and that file does not exist.');
    }
    final Object? decoded = jsonDecode(file.readAsStringSync());
    if (decoded is! Map) {
      fail('$path did not decode to a JSON object.');
    }
    final Map<String, Object?> json = decoded.cast();
    String field(String name) {
      final Object? value = json[name];
      if (value is! String || value.isEmpty) {
        fail('$path is missing a non-empty "$name" field.');
      }
      return value;
    }

    return EmulatorFixture(
      baseUrl: field('baseUrl'),
      publishableKey: field('publishableKey'),
      sessionUrl: field('sessionUrl'),
      dismissSessionUrl: field('dismissSessionUrl'),
      mtnSucceedsMsisdn: field('mtnSucceedsMsisdn'),
    );
  }
}
