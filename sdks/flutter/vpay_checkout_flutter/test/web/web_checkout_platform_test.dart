/// The web platform, run by a **real Chrome** — the platform this package's
/// own `flutter test` (stack-independent, VM-only) has never exercised.
/// `WebVpayCheckoutPlatform` (`lib/src/platform/web_checkout_platform.dart`)
/// opens a `window.open` popup and listens for one of two real browser
/// signals: a `vpay:complete` `postMessage` pinned to this window's own
/// origin, or the popup's own `closed` property going true. Neither has an
/// analogue in `dart:html`-free jsdom, and neither can be driven without a
/// second, real browsing context — so this file runs under
/// `flutter test --platform chrome`, launched only by `just test-flutter-web`
/// (`justfile`), never by plain `flutter test`.
///
/// # Why this file is not under `test/` proper
///
/// `flutter test` (the stack-independent gate every other suite in this
/// package's `test/` directory runs under, "82 passed, 0 skipped") compiles
/// **every** file it walks for the VM, and this file imports `package:web`,
/// which does not exist there — a bare `flutter test` fails to even compile
/// it. `@TestOn('chrome')` (below) is what keeps it out of that run: measured
/// directly, a `flutter test` invocation that includes a `@TestOn('chrome')`
/// file under `test/` **excludes it from the count entirely** — it is
/// neither a pass nor a skip, so "82 passed, 0 skipped" stays true with this
/// file present. `--platform chrome test/web` is what re-includes it.
///
/// # Why every scenario here uses a REAL second browsing context
///
/// `[fixtures/return.htm]` is a real HTML page this suite's own test server
/// serves over real HTTP (see that file's own doc comment for why it is
/// `.htm`, not `.html`) — opened as a REAL popup, running REAL JavaScript
/// that calls REAL `window.opener.postMessage`. No test here constructs a
/// `MessageEvent` by hand or calls `WebVpayCheckoutPlatform`'s listener
/// directly: every signal below crosses a real cross-window boundary the way
/// a payer's browser actually would.
///
/// # The origin trick
///
/// A "message from a different origin" needs a second popup whose origin
/// genuinely differs from this test page's own — but this suite has no
/// second server to reach for one. `http://127.0.0.1:<port>` and
/// `http://localhost:<port>` are the SAME physical server (the very same
/// `flutter test` dev server this test page itself is served by) and
/// DIFFERENT origins (the browser's origin comparison is host-exact, and
/// `127.0.0.1` is not `localhost` even though both resolve to the loopback
/// interface) — so re-requesting the identical fixture through the
/// `127.0.0.1` alias gives a real popup, real JavaScript, a real
/// `postMessage`, and a genuinely different `event.origin`, with no second
/// process to stand up.
///
/// # The named-window trick
///
/// `WebVpayCheckoutPlatform` keeps its `web.Window` handle private
/// (`_popup`) — by design, `VpayCheckoutPlatform`'s own contract exposes no
/// window handle at all. `show()` always opens the popup under the fixed
/// name `'vpay-checkout'`, and per the same-name `window.open` behaviour
/// `sdks/stripe-js/src/popup.ts`'s own header documents at length,
/// `web.window.open('', 'vpay-checkout')` returns a handle to that SAME
/// browsing context without navigating it. That is how this suite inspects
/// the real popup `show()` opened — `.closed`, `.location.href` (same-origin
/// here, so readable) — without `WebVpayCheckoutPlatform` exposing anything
/// it should not.
///
/// # What this does NOT prove — read before trusting a ✅
///
/// This is a **browser-executed unit test**, not an end-to-end test. It
/// proves `WebVpayCheckoutPlatform`'s own popup/message/close-poll mechanics
/// against a real Chrome DOM; it never opens vpay's actual hosted checkout
/// page. Driving THAT for real was attempted and abandoned for a structural
/// reason worth recording rather than hiding: the merchant's own return page
/// (`examples/shop`'s `/orders/{id}/return`, `PopupReturnNotifier`) posts to
/// `window.opener`'s own origin, i.e. the origin of the page that opened the
/// popup — which in a real integration is the merchant's own app. Here the
/// "opener" is this test file, served by `flutter test`'s own ephemeral
/// localhost port, an origin the demo stack's `examples/shop` (a separate
/// server on a separate port) has no way to address — so a real shop return
/// page can genuinely never complete the loop back to a Flutter web test
/// page unless the two are deployed on the same origin, which this
/// environment cannot arrange. `docs/sdks/parity.md`'s Flutter table row
/// says this plainly rather than implying an end-to-end run happened.
@TestOn('chrome')
library;

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/src/platform/messages.g.dart';
import 'package:vpay_checkout_flutter/src/platform/web_checkout_platform.dart';
import 'package:web/web.dart' as web;

/// Builds a URL for `fixtures/return.htm`, relative to this suite's own
/// `web.window.location.href` — the only way to address it, since the dev
/// server's port is chosen at random per run. [host], when given, replaces
/// the URL's host (`127.0.0.1` for the different-origin cases) while
/// leaving the port untouched — the same physical server, a different
/// origin.
Uri _fixtureUri(Map<String, String> query, {String? host}) {
  Uri base = Uri.parse(web.window.location.href);
  if (host != null) {
    base = base.replace(host: host);
  }
  return base
      .resolve('fixtures/return.htm')
      .replace(queryParameters: query.isEmpty ? null : query);
}

/// Polls [condition] until it is true, failing loudly — never silently —
/// once [timeout] elapses. Every wait in this file goes through this
/// rather than a fixed `Future.delayed`: the real browser's navigation and
/// message-dispatch timing is not something this suite controls.
Future<void> _waitUntil(
  String description,
  bool Function() condition, {
  Duration timeout = const Duration(seconds: 10),
}) async {
  final DateTime deadline = DateTime.now().add(timeout);
  while (DateTime.now().isBefore(deadline)) {
    if (condition()) {
      return;
    }
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
  fail('timed out waiting for $description');
}

void main() {
  group('WebVpayCheckoutPlatform, driven by a real Chrome', () {
    test('show() opens a real popup window at the session URL', () async {
      final WebVpayCheckoutPlatform platform = WebVpayCheckoutPlatform();
      addTearDown(platform.dismiss);

      // A realistic session URL shape (query + fragment, exactly like a
      // real hosted `session.url`) built off our own fixture, so this
      // test can also read the popup's `location.href` back afterwards —
      // possible only because the fixture is same-origin with this page.
      final Uri sessionUrl = _fixtureUri({'key': 'pk_test_web_e2e'})
          .replace(fragment: 'cs_web_e2e_secret_deadbeef');

      await platform.show(
        url: sessionUrl.toString(),
        stopUrls: const <Never>[],
        allowInsecureUrl: true,
        mode: CheckoutWindowMode.inApp,
      );

      // The named-window trick — see this file's own header.
      final web.Window? handle = web.window.open('', 'vpay-checkout');
      expect(
        handle,
        isNotNull,
        reason:
            'show() must have opened a REAL popup under the name '
            "'vpay-checkout' for this same-name open to find one at all",
      );
      expect(
        handle!.closed,
        isFalse,
        reason: 'a freshly opened popup is not closed',
      );

      await _waitUntil(
        "the real popup's own location to reach the session URL",
        () => handle.location.href == sessionUrl.toString(),
      );
    }, timeout: const Timeout(Duration(seconds: 30)));

    test("a real vpay:complete message from this window's own origin resolves the checkout, and one from a different origin is ignored (the security property)", () async {
      final WebVpayCheckoutPlatform platform = WebVpayCheckoutPlatform();
      addTearDown(platform.dismiss);

      final List<CheckoutWindowEvent> events = <CheckoutWindowEvent>[];
      final StreamSubscription<CheckoutWindowEvent> subscription = platform
          .windowEvents
          .listen(events.add);
      addTearDown(subscription.cancel);

      // The popup show() actually tracks — no message fired yet (no
      // `session` query parameter, see fixtures/return.htm's own doc
      // comment).
      await platform.show(
        url: _fixtureUri(const <String, String>{}).toString(),
        stopUrls: const <Never>[],
        allowInsecureUrl: true,
        mode: CheckoutWindowMode.inApp,
      );

      // A SECOND, independent real popup on a genuinely DIFFERENT origin
      // (see this file's header) posting a real, well-formed vpay:complete
      // — the actual attack this origin check exists to stop: a hostile
      // page with a `window.open` handle reporting a checkout complete.
      final Uri hostileUrl = _fixtureUri(const <String, String>{
        'session': 'cs_hostile_origin',
        'status': 'succeeded',
      }, host: '127.0.0.1');
      final web.Window? hostile = web.window.open(
        hostileUrl.toString(),
        'hostile-popup',
        'popup=yes',
      );
      addTearDown(() => hostile?.close());

      // Long enough for a real cross-origin popup to load and post; short
      // enough to keep the suite fast. If the origin check were broken,
      // `events` would already be non-empty by the time this returns.
      await Future<void>.delayed(const Duration(seconds: 2));
      expect(
        events,
        isEmpty,
        reason:
            'a vpay:complete from a different origin must NOT resolve the '
            'checkout — accepting it would let any other page on the '
            "payer's machine report a payment as finished",
      );

      // NOW the real popup — this window's own origin — posts the real
      // completion message, by navigating the SAME browsing context
      // show() opened (the named-window trick again).
      final web.Window? legit = web.window.open('', 'vpay-checkout');
      legit!.location.href = _fixtureUri(const <String, String>{
        'session': 'cs_same_origin',
        'status': 'succeeded',
      }).toString();

      await _waitUntil(
        'the real same-origin message to resolve the checkout',
        () => events.isNotEmpty,
      );
      expect(
        events,
        hasLength(1),
        reason:
            'exactly one real event: the ignored hostile message must not '
            'have queued anything that fires later',
      );
      expect(events.single.outcome, CheckoutWindowOutcome.stopUrlReached);
    }, timeout: const Timeout(Duration(seconds: 30)));

    test(
      'ignores a vpay:complete from a different window on the right origin',
      () async {
        final WebVpayCheckoutPlatform platform = WebVpayCheckoutPlatform();
        addTearDown(platform.dismiss);

        final List<CheckoutWindowEvent> events = <CheckoutWindowEvent>[];
        final StreamSubscription<CheckoutWindowEvent> subscription = platform
            .windowEvents
            .listen(events.add);
        addTearDown(subscription.cancel);

        // The popup show() actually tracks — no message fired yet (no
        // `session` query parameter).
        await platform.show(
          url: _fixtureUri(const <String, String>{}).toString(),
          stopUrls: const <Never>[],
          allowInsecureUrl: true,
          mode: CheckoutWindowMode.inApp,
        );

        // A SECOND, independent real popup — deliberately NOT the
        // 'vpay-checkout' named window show() opened — on this SAME origin
        // (no `host:` override, unlike the different-origin test above)
        // posting a real, well-formed vpay:complete. Its `event.origin` is
        // indistinguishable from the legitimate popup's; only `event.source`
        // (this window's `postMessage` sender identity, which the browser
        // sets and no page can spoof) tells them apart. This is exactly the
        // attack the window check exists to stop: another tab, another
        // popup, or an iframe of the merchant's own, sharing this page's
        // origin, reporting a payment as finished.
        final Uri sameOriginOtherWindowUrl = _fixtureUri(const <String, String>{
          'session': 'cs_same_origin_wrong_window',
          'status': 'succeeded',
        });
        final web.Window? otherWindow = web.window.open(
          sameOriginOtherWindowUrl.toString(),
          'same-origin-but-not-the-checkout-popup',
          'popup=yes',
        );
        addTearDown(() => otherWindow?.close());

        // Long enough for the real, same-origin, wrong-window popup to load
        // and post; short enough to keep the suite fast. If the window check
        // did not exist (or were satisfied by the origin check alone), this
        // message would already have resolved the checkout by the time this
        // returns — see this file's header for how `just test-flutter-web`
        // proves that failure mode for real, by removing the check.
        await Future<void>.delayed(const Duration(seconds: 2));
        expect(
          events,
          isEmpty,
          reason:
              'a vpay:complete from a different window on the SAME origin '
              'must NOT resolve the checkout — the origin check alone is not '
              'enough, because another window on this origin can share it',
        );

        // NOW the real popup show() itself opened — the named-window trick
        // — posts the real completion message, proving the platform still
        // accepts the one window it is actually supposed to.
        final web.Window? legit = web.window.open('', 'vpay-checkout');
        legit!.location.href = _fixtureUri(const <String, String>{
          'session': 'cs_same_origin_right_window',
          'status': 'succeeded',
        }).toString();

        await _waitUntil(
          'the real message from the actual checkout popup to resolve the '
          'checkout',
          () => events.isNotEmpty,
        );
        expect(
          events,
          hasLength(1),
          reason:
              'exactly one real event: the ignored wrong-window message must '
              'not have queued anything that fires later',
        );
        expect(events.single.outcome, CheckoutWindowOutcome.stopUrlReached);
      },
      timeout: const Timeout(Duration(seconds: 30)),
    );

    test("the popup's own closed state reports a real dismissal, never a fabricated outcome (the web analogue of design D4)", () async {
      final WebVpayCheckoutPlatform platform = WebVpayCheckoutPlatform();
      addTearDown(platform.dismiss);

      final List<CheckoutWindowEvent> events = <CheckoutWindowEvent>[];
      final StreamSubscription<CheckoutWindowEvent> subscription = platform
          .windowEvents
          .listen(events.add);
      addTearDown(subscription.cancel);

      await platform.show(
        url: _fixtureUri(const <String, String>{}).toString(),
        stopUrls: const <Never>[],
        allowInsecureUrl: true,
        mode: CheckoutWindowMode.inApp,
      );

      final web.Window? handle = web.window.open('', 'vpay-checkout');
      expect(handle, isNotNull);
      expect(handle!.closed, isFalse);

      // A real close — no message ever sent — is the only thing that
      // should produce `dismissed`; nothing here tells the platform what
      // outcome to report.
      handle.close();

      await _waitUntil(
        "the real closed poll to report the payer's dismissal",
        () => events.isNotEmpty,
        // WebVpayCheckoutPlatform polls every 500ms; give it several
        // cycles of real slack in a real browser.
        timeout: const Duration(seconds: 5),
      );
      expect(events, hasLength(1));
      expect(events.single.outcome, CheckoutWindowOutcome.dismissed);
    }, timeout: const Timeout(Duration(seconds: 30)));

    test('the outcome is never read off the message payload — a real vpay:complete always resolves stopUrlReached, whatever status or session it carries (D1: only the API decides)', () async {
      // `CheckoutWindowOutcome`'s own doc comment (messages.g.dart) makes
      // this the point of the enum: it has no succeeded/canceled/failed
      // member at all, on any platform. This test proves the WEB platform
      // host actually honours that — a message whose payload claims
      // "failed" (or carries nothing meaningful) still resolves the same
      // stopUrlReached a genuine success would, because nothing here ever
      // reads status/session to decide.
      for (final String status in <String>['failed', '', 'nonsense']) {
        final WebVpayCheckoutPlatform platform = WebVpayCheckoutPlatform();
        addTearDown(platform.dismiss);

        final List<CheckoutWindowEvent> events = <CheckoutWindowEvent>[];
        final StreamSubscription<CheckoutWindowEvent> subscription = platform
            .windowEvents
            .listen(events.add);
        addTearDown(subscription.cancel);

        await platform.show(
          url: _fixtureUri(<String, String>{
            'session': 'cs_status_$status',
            'status': status,
          }).toString(),
          stopUrls: const <Never>[],
          allowInsecureUrl: true,
          mode: CheckoutWindowMode.inApp,
        );

        await _waitUntil(
          'a real message carrying status="$status" to still resolve '
          'stopUrlReached',
          () => events.isNotEmpty,
        );
        expect(
          events.single.outcome,
          CheckoutWindowOutcome.stopUrlReached,
          reason: 'status="$status" must not change what outcome fires',
        );
        expect(
          events.single.reachedUrl,
          isNull,
          reason:
              'a message-driven completion never fabricates a navigated-to '
              'URL — reachedUrl is stopUrlReached-by-native-navigation '
              'only (design doc, "The shape")',
        );
      }
    }, timeout: const Timeout(Duration(seconds: 30)));
  });
}
