/// What `just test-flutter-emulator` mints on the HOST before running the
/// dismiss and external-browser suites ON THE EMULATOR — the same shape
/// Lane D's `test_e2e/real_stack_e2e_test.dart` fixture is (no pre-expired
/// session: both suites open a real, still-open session, they never call
/// `/confirm` themselves).
///
/// [publishableKey] and [mtnSucceedsMsisdn] were added for
/// `checkout_window_test.dart`, the suite that drove a full MTN push
/// through the in-app `WebView`'s own confirm UI. That suite (and the
/// `WebView` it drove) was retired 2026-09-16 —
/// `docs/status/mobile-flutter-plugin.md` — and neither field is read by
/// any suite left in this directory; they stay required here only because
/// trimming them would also mean reworking `justfile`'s
/// `test-flutter-emulator` fixture-minting step, which is out of scope for
/// that retirement.
///
/// Carried in as `--dart-define=VPAY_E2E_FIXTURE_B64=...`, NOT
/// `Platform.environment` (`test_e2e/real_stack_e2e_test.dart`'s own
/// mechanism): that suite runs as a plain Dart VM process on the HOST,
/// which inherits the host shell's environment; `integration_test` runs
/// the compiled app ON THE DEVICE, a separate Android process the host
/// shell's environment never reaches. `--dart-define` is the one channel
/// that DOES cross that boundary — it is compiled into the app as a
/// `String.fromEnvironment` constant. Base64-JSON rather than one
/// `--dart-define` per field: a minted session `url` carries its own `?`,
/// `&`, `=` and `#` characters, and round-tripping those through a
/// `KEY=VALUE` shell argument correctly is exactly the kind of thing
/// worth not hand-rolling twice.
///
/// Loading this is a loud [fail] in `setUpAll`, never a skip — a skipped
/// test is not a passing test (CLAUDE.md).
library;

import 'dart:convert';

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
  /// thing to the payer's browser (a Custom Tab on Android) and to this
  /// suite's own `BrowserClient` calls, where rewriting only the
  /// top-level URL's host would not. Unused by any suite left in this
  /// directory — see this file's header.
  final String baseUrl;

  /// The merchant's publishable key the minted [sessionUrl] carries.
  /// Unused by any suite left in this directory — see this file's header.
  final String publishableKey;

  /// A FRESH, still-`open`, UNCONFIRMED hosted session's `url` —
  /// `{checkoutOrigin}/c/{cs_id}?key=...#{secret}` — minted through
  /// `examples/shop`'s real server exactly as
  /// `test_e2e/real_stack_e2e_test.dart`'s fixture is, with its host
  /// already `localhost:3080` (`adb reverse tcp:3080 tcp:3080` makes that
  /// reachable from the emulator too). Deliberately NOT pre-confirmed —
  /// read by `checkout_dismiss_test.dart`, which opens it and drives a
  /// real back press, never a confirm; no suite in this directory drives a
  /// page's own confirm UI any more (`checkout_window_test.dart`, the one
  /// that did, was retired 2026-09-16).
  final String sessionUrl;

  /// A SECOND, independently-minted hosted session's `url`, read by
  /// `checkout_external_browser_test.dart` — a separate session so that
  /// file never depends on `checkout_dismiss_test.dart` having run first
  /// (or at all): each `flutter test integration_test/<file>.dart`
  /// invocation is its own process, and sharing one session across both
  /// would make one suite's pass/fail depend on run order.
  final String dismissSessionUrl;

  /// `frontends/tests/e2e/cypress/support/shop.ts`'s `MTN.succeeds` —
  /// `PENDING` on the first status query, `SUCCESSFUL` on the next
  /// (WireMock scenario `mtn-e2e-poll`). Digits-only: vpay's page
  /// validates Cameroon E.164 and refuses the hex-suffixed steering
  /// numbers other suites use. Unused by any suite left in this
  /// directory — see this file's header.
  final String mtnSucceedsMsisdn;

  static const String _fixtureB64 = String.fromEnvironment(
    'VPAY_E2E_FIXTURE_B64',
  );

  static EmulatorFixture load() {
    if (_fixtureB64.isEmpty) {
      fail(
        'VPAY_E2E_FIXTURE_B64 is not set (as a --dart-define). This is a '
        'REAL emulator suite and refuses to run against nothing — it '
        "never skips. Run it through 'just test-flutter-emulator', which "
        'mints this fixture against a running vpay stack, arms adb '
        'reverse, and passes it in this way — see the justfile.',
      );
    }
    late final Map<String, Object?> json;
    try {
      final String decoded = utf8.decode(base64.decode(_fixtureB64));
      final Object? parsed = jsonDecode(decoded);
      if (parsed is! Map) {
        fail('VPAY_E2E_FIXTURE_B64 did not decode to a JSON object.');
      }
      json = parsed.cast();
    } on FormatException catch (e) {
      fail('VPAY_E2E_FIXTURE_B64 is not valid base64/JSON: $e');
    }
    String field(String name) {
      final Object? value = json[name];
      if (value is! String || value.isEmpty) {
        fail('VPAY_E2E_FIXTURE_B64 is missing a non-empty "$name" field.');
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
