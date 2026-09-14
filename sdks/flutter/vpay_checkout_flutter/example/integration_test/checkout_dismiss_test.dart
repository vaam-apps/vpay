/// Lane E — proves a REAL back-press on the REAL `VpayCheckoutActivity`
/// reports a dismissal over the REAL platform channel, never a fabricated
/// outcome.
///
/// Run SEPARATELY from `checkout_window_test.dart`
/// (`just test-flutter-emulator`'s own two invocations) because this file
/// needs a real hardware-back-key event, and a headless emulator has no
/// touchscreen or keyboard for this suite to drive from inside the Dart
/// test process itself (which runs ON the device, with no `adb` of its
/// own). The recipe supplies the back press from the HOST, timed off a log
/// marker this file prints — see `justfile`'s `test-flutter-emulator` for
/// the `adb shell input keyevent 4` it sends the moment it sees
/// `LANE_E_DISMISS_TEST_READY` in `adb logcat`.
///
/// This still proves something no stub could: [VpayCheckoutPlatform.show]
/// opens a REAL Activity (a fabricated platform would need no back key to
/// "dismiss"), and the event that arrives is READ off the REAL
/// `VpayCheckoutFlutterApi.onWindowEvent` channel after a REAL Android
/// `OnBackPressedCallback` fired inside a REAL `ComponentActivity`.
library;

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:vpay_checkout_flutter/src/platform/messages.g.dart'
    show CheckoutWindowMode;
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

import 'support/ensure_platform_registered.dart';
import 'support/fixture.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  late final EmulatorFixture fixture;

  setUpAll(() {
    ensurePlatformRegistered();
    fixture = EmulatorFixture.load();
  });

  testWidgets(
    'a real back press on the real Activity reports dismissed, not a fabricated outcome',
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

      await platform.show(
        url: fixture.sessionUrl,
        stopUrls: const [],
        allowInsecureUrl: true,
        mode: CheckoutWindowMode.inApp,
      );

      // Gives the Activity transition time to land in the foreground
      // before the marker below tells the host it is safe to send the
      // back key — the host ALSO independently polls `dumpsys activity`
      // for `VpayCheckoutActivity` before pressing (justfile), so this is
      // belt, not the only strap.
      await Future<void>.delayed(const Duration(milliseconds: 800));

      // Read by the host's `adb logcat` — see this file's header.
      // ignore: avoid_print
      print('LANE_E_DISMISS_TEST_READY');

      final CheckoutWindowEvent event = await settled.future.timeout(
        const Duration(seconds: 45),
        onTimeout: () => fail(
          'no window event arrived within 45s of printing the ready '
          "marker — either the host's back press never arrived, or the "
          'real OnBackPressedCallback did not fire',
        ),
      );

      expect(
        event.outcome,
        CheckoutWindowOutcome.dismissed,
        reason:
            'a back press must always be a dismissal (design doc D5), '
            'never a fabricated stopUrlReached',
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
