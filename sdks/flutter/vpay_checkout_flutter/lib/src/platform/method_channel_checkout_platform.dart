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
/// plugin loader calls [registerWith] for us; nothing else in this package
/// calls it directly (asserted by `test/method_channel_checkout_platform_test.dart`
/// only exercising the class itself, not the loader).
library;

import 'dart:async';

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
    VpayCheckoutFlutterApi.setUp(this);
  }

  final VpayCheckoutHostApi _hostApi;
  final StreamController<CheckoutWindowEvent> _windowEvents =
      StreamController<CheckoutWindowEvent>.broadcast();

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
    required CheckoutWindowMode mode,
  }) {
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
        mode: mode,
      ),
    );
  }

  @override
  Future<void> dismiss() => _hostApi.dismiss();

  @override
  Stream<CheckoutWindowEvent> get windowEvents => _windowEvents.stream;

  /// [VpayCheckoutFlutterApi]'s one method: the native host fires this
  /// exactly once per `show` (design doc, "The shape"); this class only
  /// republishes it, it never inspects [event.reachedUrl] (D1 — see
  /// `pigeons/checkout.dart`'s doc comment on that field).
  @override
  void onWindowEvent(CheckoutWindowEvent event) => _windowEvents.add(event);
}
