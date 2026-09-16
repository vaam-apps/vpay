/// D8, revised 2026-09-16 — proves the platform host opens a REAL Custom
/// Tab on a REAL (headless) Android emulator, and that a REAL return to the
/// host app's own `Activity` reports the real dismissal over the real
/// platform channel — never a fabricated outcome.
///
/// Formerly this covered `VpayCheckoutMode.externalBrowser` as one of two
/// selectable modes; since the 2026-09-16 revision of D5 there is only one
/// surface — the payer's browser, a Custom Tab on Android — on every
/// platform, so `VpayCheckoutPlatform.show` no longer takes a `mode`
/// argument at all. What this file proves did not change: it is no longer
/// a *distinct* mode alongside an in-app WebView (that surface, and the
/// `checkout_window_test.dart`/JS-evaluation harness that drove it, were
/// retired 2026-09-16 along with the WebView itself — see
/// `docs/status/mobile-flutter-plugin.md`), it is simply what `show` does
/// now, always.
///
/// Run SEPARATELY from `checkout_dismiss_test.dart`, the same way those
/// two are separate from each other: this file needs a real hardware
/// back-key event too (to leave the Custom Tab and
/// return to the host app), and a headless emulator has no touchscreen or
/// keyboard for this suite to drive from inside the Dart test process
/// itself (which runs ON the device, with no `adb` of its own). The host
/// supplies the back press, timed off the `LANE_D8_EXTERNAL_BROWSER_READY`
/// log marker this file prints, the same pattern
/// `checkout_dismiss_test.dart` uses for its own marker.
///
/// D8's own design: Custom Tabs (`androidx.browser`) has no scheme
/// callback and is never started with `startActivityForResult` — a Custom
/// Tab is another app's (Chrome's) window. So the only return signal is
/// `VpayCheckoutFlutterPlugin`'s `Application.ActivityLifecycleCallbacks`
/// noticing the host `Activity` resume, reported as `dismissed`, never
/// `stopUrlReached` — there is no navigation to watch a stop URL against
/// once Chrome, not this plugin, owns the page.
///
/// This still proves something no stub could: [VpayCheckoutPlatform.show]
/// launches a REAL `CustomTabsIntent` that a REAL browser (Chrome,
/// confirmed present on this AVD's system image — see this repository's
/// own report for the `pm`/`cmd package` evidence) actually opens,
/// backgrounding the host app; the event that arrives is READ off the REAL
/// `VpayCheckoutFlutterApi.onWindowEvent` channel after a REAL
/// `Application.ActivityLifecycleCallbacks.onActivityResumed` fired on a
/// REAL `Activity`.
library;

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

import 'support/fixture.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  late final EmulatorFixture fixture;

  setUpAll(() {
    fixture = EmulatorFixture.load();
  });

  testWidgets(
    'show() opens a real Custom Tab (the only Android surface now); a real return to the host Activity reports dismissed',
    (WidgetTester tester) async {
      final VpayCheckoutPlatform platform = VpayCheckoutPlatform.instance;
      final Completer<CheckoutWindowEvent> settled =
          Completer<CheckoutWindowEvent>();
      final StreamSubscription<CheckoutWindowEvent> subscription = platform
          .windowEvents
          .listen((CheckoutWindowEvent event) {
            if (!settled.isCompleted) {
              settled.complete(event);
            }
          });
      addTearDown(subscription.cancel);

      // Reuses the dismiss fixture's session — this suite never drives the
      // page's own confirm UI (it cannot: the page is inside Chrome, not
      // this plugin's WebView), so which session it loads is otherwise
      // unconstrained.
      await platform.show(
        url: fixture.dismissSessionUrl,
        stopUrls: const [],
        allowInsecureUrl: true,
      );

      // Gives Chrome's own Custom Tab activity transition time to land in
      // the foreground before the marker below tells the host it is safe
      // to send the back key — the host ALSO independently polls
      // `dumpsys activity` for a Chrome activity before pressing.
      await Future<void>.delayed(const Duration(milliseconds: 1500));

      // Read by the host's `adb logcat` — see this file's header.
      // ignore: avoid_print
      print('LANE_D8_EXTERNAL_BROWSER_READY');

      final CheckoutWindowEvent event = await settled.future.timeout(
        const Duration(seconds: 45),
        onTimeout: () => fail(
          'no window event arrived within 45s of printing the ready '
          "marker — either the host's back press never arrived, or the "
          'real Application.ActivityLifecycleCallbacks.onActivityResumed '
          'did not fire',
        ),
      );

      expect(
        event.outcome,
        CheckoutWindowOutcome.dismissed,
        reason:
            'D8 tier 0 has no stop-URL interception once Chrome owns the '
            'page — every return is reported as a dismissal, and D1/D4 '
            'make that correctness-complete via the poll',
      );
      expect(
        event.reachedUrl,
        isNull,
        reason: 'dismissed carries no reachedUrl (pigeons/checkout.dart)',
      );
    },
    timeout: const Timeout(Duration(minutes: 1)),
  );
}
