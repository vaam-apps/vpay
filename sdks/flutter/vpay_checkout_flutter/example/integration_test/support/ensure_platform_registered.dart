/// Explicitly registers the REAL Android platform host as
/// `VpayCheckoutPlatform.instance`.
///
/// `pubspec.yaml`'s `dartPluginClass: MethodChannelVpayCheckoutPlatform`
/// mechanism registers this automatically for a normal `flutter run` (and
/// for `example/lib/main.dart`'s own entrypoint) via a build-time-generated
/// wrapper around `main()`. Measured on this host: that wrapper is not
/// applied to `integration_test/*.dart`'s own `main()` the way it is to
/// `lib/main.dart`'s — `VpayCheckoutPlatform.instance` stayed
/// `UnimplementedVpayCheckoutPlatform` under `flutter test
/// integration_test/checkout_window_test.dart -d device` until this file
/// registered it explicitly. This calls the package's own real
/// registration function directly — not a fake, not a different
/// implementation — the same class `pubspec.yaml` already names.
library;

import 'package:vpay_checkout_flutter/src/platform/checkout_platform.dart';
import 'package:vpay_checkout_flutter/src/platform/method_channel_checkout_platform.dart';

void ensurePlatformRegistered() {
  if (VpayCheckoutPlatform.instance is UnimplementedVpayCheckoutPlatform) {
    MethodChannelVpayCheckoutPlatform.registerWith();
  }
}
