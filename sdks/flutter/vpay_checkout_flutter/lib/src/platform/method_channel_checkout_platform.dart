/// The Android, iOS and macOS implementation of [VpayCheckoutPlatform] —
/// Lane C (`docs/plans/2026-09-13-flutter-plugin-brief.md`).
///
/// One Dart class for all three platforms: the channel it speaks
/// (`pigeons/checkout.dart`'s `VpayCheckoutHostApi`/`VpayCheckoutFlutterApi`)
/// is identical on each, and what differs — `VpayCheckoutActivity` on
/// Android, `VpayCheckoutViewController` on iOS, `VpayCheckoutViewController`
/// on macOS (design doc D5) — lives entirely on the native side of the
/// channel. This class itself decides nothing about the outcome (D1): it
/// only forwards [VpayCheckoutPlatform.show]'s already-normalised
/// [StopUrlSpec]s across the channel as [CheckoutStopUrl]s, and republishes
/// whatever [CheckoutWindowEvent] the native host reports.
///
/// Registered as the Dart-side default for `android`, `ios` and `macos` in
/// `pubspec.yaml`'s `flutter: plugin: platforms:` block, via
/// `dartPluginClass: MethodChannelVpayCheckoutPlatform` — Flutter's own
/// plugin loader calls [registerWith] for us. That is now an optimisation,
/// not the only path here: `checkout_platform.dart`'s `VpayCheckoutPlatform
/// .instance` getter also constructs this class lazily on Android/iOS/macOS
/// the first time nothing has registered yet — see that file's doc comment
/// for why (`dartPluginClass`'s own timing turned out to be racy on a real
/// installed app).
///
/// **The constructor never lets a missing `ServicesBinding` escape as an
/// exception.** [registerWith] can run before
/// `WidgetsFlutterBinding.ensureInitialized()` has (the exact real-app
/// defect `test/method_channel_checkout_platform_registration_test.dart`
/// guards), so [_trySetUpFlutterApi] catches the [FlutterError] that
/// `VpayCheckoutFlutterApi.setUp` throws in that state and retries from
/// every call this class makes into the channel ([show], [dismiss],
/// [windowEvents]) — each of which only ever runs after a merchant's own
/// `main()` has, by which point the binding always exists.
library;

import 'dart:async';

import 'package:flutter/foundation.dart' show FlutterError;

import '../checkout_controller.dart' show StopUrlSpec;
import 'checkout_platform.dart';
import 'messages.g.dart';

/// Wraps the pigeon-generated [VpayCheckoutHostApi] (Dart calls into the
/// native host) and answers [VpayCheckoutFlutterApi] (the native host calls
/// back) by republishing onto [windowEvents] — the same shape
/// [VpayCheckoutPlatform] promises regardless of which native host answers
/// it.
final class MethodChannelVpayCheckoutPlatform extends VpayCheckoutPlatform
    implements VpayCheckoutFlutterApi {
  /// [hostApi] is injectable so a test can substitute a fake without a real
  /// `BinaryMessenger` — the same reason `checkout_platform.dart`'s doc
  /// comment gives for this whole seam existing as an abstract class rather
  /// than pigeon's generated class used directly.
  MethodChannelVpayCheckoutPlatform({VpayCheckoutHostApi? hostApi})
    : _hostApi = hostApi ?? VpayCheckoutHostApi() {
    _trySetUpFlutterApi();
  }

  final VpayCheckoutHostApi _hostApi;
  final StreamController<CheckoutWindowEvent> _windowEvents =
      StreamController<CheckoutWindowEvent>.broadcast();
  bool _flutterApiRegistered = false;

  /// Wires this instance up to answer [VpayCheckoutFlutterApi] calls from
  /// the native host — a no-op once it has already succeeded. Swallows
  /// exactly the [FlutterError] `ServicesBinding.instance` throws before
  /// `WidgetsFlutterBinding.ensureInitialized()` has run (see this file's
  /// doc comment); any other exception is a real bug and is left to
  /// propagate.
  void _trySetUpFlutterApi() {
    if (_flutterApiRegistered) {
      return;
    }
    try {
      VpayCheckoutFlutterApi.setUp(this);
      _flutterApiRegistered = true;
    } on FlutterError {
      // No Flutter binding yet — retried by [show], [dismiss] and
      // [windowEvents], each of which only runs once the app's own
      // main() has, by which point one always exists.
    }
  }

  /// Sets [VpayCheckoutPlatform.instance] to a fresh instance of this class.
  /// Called by Flutter's plugin loader on Android, iOS and macOS — see this
  /// file's own doc comment.
  static void registerWith() {
    VpayCheckoutPlatform.instance = MethodChannelVpayCheckoutPlatform();
  }

  @override
  Future<void> show({
    required String url,
    required List<StopUrlSpec> stopUrls,
    required bool allowInsecureUrl,
  }) {
    _trySetUpFlutterApi();
    return _hostApi.show(
      ShowCheckoutRequest(
        url: url,
        stopUrls: [
          for (final StopUrlSpec stopUrl in stopUrls)
            CheckoutStopUrl(
              scheme: stopUrl.scheme,
              host: stopUrl.host,
              port: stopUrl.port,
              path: stopUrl.path,
            ),
        ],
        allowInsecureUrl: allowInsecureUrl,
      ),
    );
  }

  @override
  Future<void> dismiss() {
    _trySetUpFlutterApi();
    return _hostApi.dismiss();
  }

  @override
  Stream<CheckoutWindowEvent> get windowEvents {
    _trySetUpFlutterApi();
    return _windowEvents.stream;
  }

  /// [VpayCheckoutFlutterApi]'s one method: the native host fires this
  /// exactly once per `show` (design doc, "The shape"); this class only
  /// republishes it, it never inspects [event.reachedUrl] (D1 — see
  /// `pigeons/checkout.dart`'s doc comment on that field).
  @override
  void onWindowEvent(CheckoutWindowEvent event) => _windowEvents.add(event);
}
