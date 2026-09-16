/// Regression test for the real-app defect fixed in
/// `lib/src/platform/method_channel_checkout_platform.dart`: on a real
/// installed APK, `pubspec.yaml`'s `dartPluginClass` mechanism generates a
/// `_PluginRegistrant.register()` that the Flutter **engine** calls when the
/// root isolate starts — measurably *before* the app's own `main()` body
/// runs, in particular before `WidgetsFlutterBinding.ensureInitialized()` /
/// `runApp()`. [MethodChannelVpayCheckoutPlatform.registerWith] used to call
/// `VpayCheckoutFlutterApi.setUp(this)` eagerly during construction, which
/// reaches into `ServicesBinding.instance` immediately; with no binding yet,
/// that threw `Binding has not yet been initialized`, and the engine's own
/// generated wrapper silently swallowed it (only printing it) — leaving
/// [VpayCheckoutPlatform.instance] stuck at [UnimplementedVpayCheckoutPlatform]
/// for the whole app lifetime. A real merchant tapping "Start checkout" then
/// saw `windowEvents has no platform host yet` and no window ever opened.
///
/// Deliberately the only test file in this package whose `main()` does
/// **not** call `TestWidgetsFlutterBinding.ensureInitialized()`:
/// `test/method_channel_checkout_platform_test.dart` (and every other
/// existing test that touches this class) does, which is exactly why none
/// of them caught this — `flutter test` runs each file in its own isolate,
/// so this file alone, left unmodified, reproduces the pre-`main()` state a
/// real app's root isolate is in when the engine calls the plugin
/// registrant. Revert the fix (restore the eager `VpayCheckoutFlutterApi
/// .setUp(this)` call in the constructor, or the constructor's registration
/// happening before a binding can exist) and this test fails with exactly
/// that `Binding has not yet been initialized` error.
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/src/platform/checkout_platform.dart';
import 'package:vpay_checkout_flutter/src/platform/method_channel_checkout_platform.dart';

void main() {
  test('registerWith succeeds and sets a real platform instance with no '
      'Flutter binding initialized yet — the exact state a real installed '
      "app's root isolate is in when the engine calls the generated plugin "
      "registrant, before that app's own main() has run "
      'WidgetsFlutterBinding.ensureInitialized()/runApp()', () {
    expect(MethodChannelVpayCheckoutPlatform.registerWith, returnsNormally);
    expect(
      VpayCheckoutPlatform.instance,
      isA<MethodChannelVpayCheckoutPlatform>(),
      reason:
          'a real app tapping "Start checkout" would otherwise still see '
          'UnimplementedVpayCheckoutPlatform and never open a window',
    );
  });
}
