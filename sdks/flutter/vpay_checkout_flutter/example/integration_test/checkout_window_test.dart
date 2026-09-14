/// Lane E — drives the REAL `VpayCheckoutActivity` and its REAL `WebView`
/// on a real (headless) Android emulator, against the REAL running vpay
/// stack, through `VpayCheckout.start` — the plugin's own public entry
/// point, not a Dart stub of the platform channel.
///
/// Requires an attached device (`flutter test integration_test/
/// checkout_window_test.dart -d emulator-serial`) and a fixture minted by
/// `just test-flutter-emulator` at `$VPAY_E2E_FIXTURE_FILE` — see
/// `support/fixture.dart`. Never skips: `EmulatorFixture.load` fails loudly
/// with neither.
///
/// # What this proves, in one continuous real flow
///
/// 1. `VpayCheckout.start` opens the REAL `VpayCheckoutActivity` through
///    the REAL platform channel (`MethodChannelVpayCheckoutPlatform` ->
///    `VpayCheckoutHostApi.show` -> `VpayCheckoutFlutterPlugin.show` ->
///    `startActivityForResult`) — no `VpayCheckoutPlatform` fake anywhere
///    in this file.
/// 2. The Activity's `WebView` actually rendered vpay's real hosted
///    checkout page: asserted on `document.title`, `document.location.href`
///    and the page's own `data-screen="select_rail"` DOM state, all read
///    back from the REAL WebView's REAL JS engine via
///    `TestJsHarness` (`example/android/app`'s test-only channel — see its
///    own doc comment for why this is not `addJavascriptInterface` and is
///    not part of `vpay_checkout_flutter`'s public API).
/// 3. The full MTN MoMo push is driven THROUGH THE PAGE — clicking the
///    real rail button, typing a real steering MSISDN into the real form,
///    submitting it, and waiting for the page's OWN poll of
///    `/v1/browser/payment_intents/{id}` to reach `succeeded` — then
///    clicking the page's own forward button, which the page navigates on
///    exactly as a real payer's tap would. That navigation is intercepted
///    by the REAL `VpayCheckoutActivity.shouldOverrideUrlLoading`, which
///    finishes the Activity and reports `stopUrlReached` back over the
///    REAL `VpayCheckoutFlutterApi.onWindowEvent` channel — asserted here
///    directly off a second, independent listener on
///    `VpayCheckoutPlatform.instance.windowEvents`, not only off
///    `VpayCheckout.start`'s own return value.
/// 4. `VpayCheckout.start` itself resolves `VpayCheckoutSucceeded` — its
///    own subsequent poll (`CheckoutController.resolveAfterStopUrlReached`)
///    re-confirms what the page already drove to `succeeded`, against the
///    real server, independently of the page's own poll.
///
/// # What this does NOT prove
///
/// The rail is WireMock (`docs/status.md`'s banner) — this is not a real
/// MTN charge. And the MSISDN form is filled via `HTMLInputElement.value`
/// plus a real DOM `.click()` on the submit button (see `_fillAndSubmit`),
/// not a touch event: a headless emulator has no way to inject one, and
/// `docs/sdks/parity.md` says so plainly rather than implying a touch test
/// ran. The click and the value the real page's own `FormData`-reading
/// submit handler receives are both real, and the page's own JS decides
/// everything from there — nothing here fabricates the outcome.
library;

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

import 'support/ensure_platform_registered.dart';
import 'support/fixture.dart';
import 'support/test_js_harness.dart';

/// Polls [read] every [interval] until it returns [expected], failing with
/// [description] if [timeout] elapses first. Every wait in this file goes
/// through this rather than a fixed `Future.delayed`: the page's own poll
/// interval and the worker's own status-poll ladder are real, unstubbed
/// timing this suite does not control.
Future<void> _waitUntil(
  String description,
  Future<String?> Function() read,
  bool Function(String? value) satisfied, {
  Duration timeout = const Duration(seconds: 30),
  Duration interval = const Duration(milliseconds: 500),
}) async {
  final DateTime deadline = DateTime.now().add(timeout);
  String? last;
  while (DateTime.now().isBefore(deadline)) {
    try {
      last = await read();
    } on NoCheckoutWindowOpen {
      // The Activity has not finished `startActivityForResult` yet — not
      // a failure, just not-yet, exactly like any other not-yet-satisfied
      // value below.
      last = null;
    }
    if (satisfied(last)) {
      return;
    }
    await Future<void>.delayed(interval);
  }
  fail('timed out waiting for $description; last observed value: $last');
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  late final EmulatorFixture fixture;
  final TestJsHarness js = TestJsHarness();

  setUpAll(() {
    ensurePlatformRegistered();
    fixture = EmulatorFixture.load();
  });

  testWidgets(
    'VpayCheckout.start opens the real Activity, the real WebView renders '
    "vpay's real page, a full MTN push is driven through it, and the real "
    'stop-URL interception reports stopUrlReached, resolving succeeded',
    (WidgetTester tester) async {
      final VpayCheckout checkout = VpayCheckout(
        baseUrl: fixture.baseUrl,
        publishableKey: fixture.publishableKey,
        // Reached over adb reverse — see EmulatorFixture.baseUrl's own doc
        // comment for why this is plain http on purpose here.
        allowInsecureBaseUrl: true,
      );

      // An independent listener on the raw platform event stream —
      // requirement 3 is asserted off THIS, not only off checkout.start's
      // eventual return value, so a bug that produced the right final
      // result via the wrong event could not hide here.
      final List<CheckoutWindowEvent> rawEvents = <CheckoutWindowEvent>[];
      final StreamSubscription<CheckoutWindowEvent> rawSubscription =
          VpayCheckoutPlatform.instance.windowEvents.listen(rawEvents.add);
      addTearDown(rawSubscription.cancel);

      final Stopwatch clock = Stopwatch()..start();
      final Future<VpayCheckoutResult> resultFuture = checkout.start(
        fixture.sessionUrl,
      );

      // --- Requirement 1 + 2: the real Activity opened and the real
      // WebView rendered vpay's real page. `evaluateJavascript` answers
      // 'null' (the JSON literal TestJsHarness.eval sees as the raw
      // string) until a page is actually loaded and a script can run in
      // it, so this wait is itself evidence the window opened before any
      // DOM assertion below is attempted.
      await _waitUntil(
        'the real checkout page to render its select_rail screen',
        () => js.evalString(
          "document.querySelector('[data-screen]') ? "
          "document.querySelector('[data-screen]').getAttribute('data-screen') "
          ': null',
        ),
        (value) => value == 'select_rail',
        timeout: const Duration(seconds: 45),
      );
      // ignore: avoid_print
      print('[laneE] t=${clock.elapsed} select_rail rendered');

      final String? title = await js.evalString('document.title');
      final String? href = await js.evalString(
        'document.location.href.toString()',
      );
      expect(
        title,
        isNotNull,
        reason: 'a real WebView on a real page always has a document.title',
      );
      expect(title, isNotEmpty);
      expect(
        href,
        contains('/c/'),
        reason:
            "the WebView's own document.location — a stub could not "
            'produce this: it is the real page reporting where it actually '
            'navigated to',
      );
      final Uri sessionUri = Uri.parse(fixture.sessionUrl.split('#').first);
      expect(
        href,
        startsWith(sessionUri.origin),
        reason:
            'the real WebView actually navigated to the checkout origin '
            "this suite's own fixture minted, not something this test "
            'invented',
      );
      // ignore: avoid_print
      print('[laneE] t=${clock.elapsed} title="$title" href=$href');

      // --- Requirement 3, driven for real through the page.
      await js.eval(
        "document.querySelector('button[data-rail=\"mtn_momo\"]').click();",
      );
      await _waitUntil(
        'the collect_msisdn screen after choosing MTN MoMo',
        () => js.evalString(
          "document.querySelector('[data-screen]') ? "
          "document.querySelector('[data-screen]').getAttribute('data-screen') "
          ': null',
        ),
        (value) => value == 'collect_msisdn',
      );
      // ignore: avoid_print
      print('[laneE] t=${clock.elapsed} collect_msisdn rendered');

      await js.eval(
        "(function(){"
        "var input = document.getElementById('vpay-msisdn');"
        "input.value = '${fixture.mtnSucceedsMsisdn}';"
        "document.querySelector('button[type=\"submit\"]').click();"
        '})();',
      );

      await _waitUntil(
        'the waiting screen after submitting the MSISDN',
        () => js.evalString(
          "document.querySelector('[data-screen]') ? "
          "document.querySelector('[data-screen]').getAttribute('data-screen') "
          ': null',
        ),
        (value) => value == 'waiting',
        timeout: const Duration(seconds: 30),
      );
      // ignore: avoid_print
      print('[laneE] t=${clock.elapsed} waiting screen rendered');

      // The rail is WireMock's `mtn-e2e-poll` scenario: PENDING on the
      // first status query, SUCCESSFUL on the next — `vpay-worker`'s own
      // poll ladder drives that, and the PAGE's own poll of
      // `/v1/browser/payment_intents/{id}` is what surfaces it here. 120s
      // matches `shop-hosted.cy.ts`'s own budget for the identical ladder.
      await _waitUntil(
        "the page's own outcome screen to reach succeeded",
        () => js.evalString(
          "document.querySelector('[data-outcome]') ? "
          "document.querySelector('[data-outcome]').getAttribute('data-outcome') "
          ': null',
        ),
        (value) => value == 'succeeded',
        timeout: const Duration(seconds: 120),
        interval: const Duration(seconds: 2),
      );
      // ignore: avoid_print
      print(
        '[laneE] t=${clock.elapsed} outcome=succeeded rendered by the page',
      );

      // The forward button — real navigation, real interception. Nothing
      // Dart-side has told the native Activity anything yet; this click is
      // the ENTIRE trigger for `shouldOverrideUrlLoading` to fire.
      await js.eval(
        "document.querySelector('[data-outcome=\"succeeded\"] button').click();",
      );

      final VpayCheckoutResult result = await resultFuture.timeout(
        const Duration(seconds: 30),
      );
      // ignore: avoid_print
      print('[laneE] t=${clock.elapsed} checkout.start resolved: $result');

      expect(
        rawEvents,
        hasLength(1),
        reason:
            'VpayCheckoutFlutterApi.onWindowEvent fires exactly once per '
            'show (design doc, "The shape")',
      );
      expect(
        rawEvents.single.outcome,
        CheckoutWindowOutcome.stopUrlReached,
        reason:
            'the real forward-button navigation must have matched one of '
            "ShowCheckoutRequest.stopUrls for the real "
            'shouldOverrideUrlLoading to report this rather than a '
            'dismissal',
      );
      expect(
        rawEvents.single.reachedUrl,
        isNotNull,
        reason:
            'stopUrlReached always carries the URL the WebView actually '
            'navigated to (pigeons/checkout.dart)',
      );

      expect(
        result,
        isA<VpayCheckoutSucceeded>(),
        reason:
            "checkout.start's own re-poll of the real server did not "
            'confirm succeeded; got: $result',
      );
    },
    timeout: const Timeout(Duration(minutes: 3)),
  );
}
