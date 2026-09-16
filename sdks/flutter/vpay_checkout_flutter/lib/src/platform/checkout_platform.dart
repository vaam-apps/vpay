/// The platform seam (D7, D5). Deliberately **not** the pigeon-generated
/// `VpayCheckoutHostApi` directly: that class is a concrete caller bound to
/// a `BinaryMessenger` the moment it is constructed, which is exactly right
/// for Lane C's real implementations and exactly wrong for a fake this
/// package's own tests substitute — so this abstract class is the thing
/// `VpayCheckout` actually depends on, and a platform implementation wraps
/// the generated Host/Flutter API internally.
///
/// **Android, iOS and macOS are implemented**
/// (`method_channel_checkout_platform.dart`, Lane C) and web is too
/// (`web_checkout_platform.dart`). [VpayCheckoutPlatform.instance] defaults
/// to [UnimplementedVpayCheckoutPlatform] only on a platform none of those
/// cover (Linux/Windows desktop) — every member of that stub throws
/// `UnimplementedError`, never a hard-coded plausible answer. This is the
/// parity table's ⛔ "no platform host exists" row, made true by the code
/// rather than only claimed by the doc.
///
/// **Registering the real implementation does not depend on
/// `pubspec.yaml`'s `dartPluginClass` mechanism running.** That mechanism
/// (a generated `_PluginRegistrant.register()` the Flutter *engine* calls
/// before the app's own `main()` runs) turned out to be racy on a real
/// installed app: sometimes the engine calls it before
/// `WidgetsFlutterBinding.ensureInitialized()`, which made the eager
/// `VpayCheckoutFlutterApi.setUp` call inside
/// `MethodChannelVpayCheckoutPlatform`'s constructor throw — a throw the
/// engine's own generated wrapper only prints, leaving
/// [VpayCheckoutPlatform.instance] stuck at [UnimplementedVpayCheckoutPlatform]
/// for the whole app lifetime
/// (`docs/status/verification/2026-09-16-flutter-real-app-registration.md`).
/// [instance]'s getter below is therefore platform-aware on its own: the
/// first read on Android, iOS or macOS that finds no host registered yet
/// resolves [MethodChannelVpayCheckoutPlatform] itself, lazily, at a point
/// guaranteed to be after the app's own `main()` has run (a merchant's
/// widget tree cannot call [VpayCheckoutPlatform.instance] before that).
/// `dartPluginClass` registration, when it *does* win the race, is now
/// purely an optimisation — the eager path this same file's `instance`
/// getter would otherwise take lazily.
library;

import '../checkout_controller.dart' show StopUrlSpec;
import 'messages.g.dart' show CheckoutWindowEvent, CheckoutWindowMode;
import 'method_channel_checkout_platform.dart'
    show MethodChannelVpayCheckoutPlatform;
import 'native_mobile_host_stub.dart'
    if (dart.library.io) 'native_mobile_host_io.dart'
    show isNativeMobileHost;

/// What a platform host must do: show a URL, watch for one of a list of
/// stop URLs or a dismissal, and say which — nothing else (design doc, "The
/// shape").
abstract class VpayCheckoutPlatform {
  const VpayCheckoutPlatform();

  static VpayCheckoutPlatform _instance =
      const UnimplementedVpayCheckoutPlatform();

  /// The active implementation. A platform package
  /// (`sdks/flutter/vpay_checkout_flutter/{android,ios,macos}` or the web
  /// implementation — all Lane C) can set this directly (`registerWith`,
  /// or a test substituting a fake); until one does **and** this getter is
  /// read on a platform none of those cover, this lazily becomes
  /// [MethodChannelVpayCheckoutPlatform] on Android/iOS/macOS (see this
  /// file's own doc comment) or stays [UnimplementedVpayCheckoutPlatform]
  /// everywhere else (web included — `WebVpayCheckoutPlatform` registers
  /// itself through Flutter's separate web plugin-loading mechanism, not
  /// this fallback).
  static VpayCheckoutPlatform get instance {
    final VpayCheckoutPlatform current = _instance;
    if (current is UnimplementedVpayCheckoutPlatform && isNativeMobileHost) {
      final VpayCheckoutPlatform resolved = MethodChannelVpayCheckoutPlatform();
      _instance = resolved;
      return resolved;
    }
    return current;
  }

  static set instance(VpayCheckoutPlatform value) => _instance = value;

  /// Opens the native window loading `url`. [stopUrls] is already
  /// normalised (D2) — scheme, host, port and path only — so a platform
  /// host never has to apply `{CHECKOUT_SESSION_ID}` substitution or decide
  /// what "matches" means itself. [allowInsecureUrl] mirrors
  /// `BrowserClient.allowInsecureBaseUrl` (D6).
  ///
  /// [mode] is D8's `inApp`/`externalBrowser` choice. A host that has not
  /// implemented `externalBrowser` must throw [UnimplementedError] rather
  /// than silently opening the in-app window instead — see
  /// `vpay_checkout.dart`'s doc comment on `VpayCheckoutMode.externalBrowser`
  /// for why that fallback is the worse failure.
  Future<void> show({
    required String url,
    required List<StopUrlSpec> stopUrls,
    required bool allowInsecureUrl,
    required CheckoutWindowMode mode,
  });

  /// Closes the window if one is open.
  Future<void> dismiss();

  /// Fires exactly once per [show] with the one outcome that occurred —
  /// the Dart side of `pigeons/checkout.dart`'s `VpayCheckoutFlutterApi`.
  Stream<CheckoutWindowEvent> get windowEvents;
}

/// The stub every call throws from until Lane C registers a real
/// implementation.
final class UnimplementedVpayCheckoutPlatform extends VpayCheckoutPlatform {
  const UnimplementedVpayCheckoutPlatform();

  static Never _unimplemented(String member) {
    throw UnimplementedError(
      'vpay_checkout_flutter: VpayCheckoutPlatform.$member has no platform '
      'host yet. Lane C of docs/plans/2026-09-13-flutter-plugin-brief.md '
      'implements Android, iOS, macOS and web; until one registers itself '
      'as VpayCheckoutPlatform.instance, this package cannot open a window.',
    );
  }

  @override
  Future<void> show({
    required String url,
    required List<StopUrlSpec> stopUrls,
    required bool allowInsecureUrl,
    required CheckoutWindowMode mode,
  }) => _unimplemented('show');

  @override
  Future<void> dismiss() => _unimplemented('dismiss');

  @override
  Stream<CheckoutWindowEvent> get windowEvents =>
      _unimplemented('windowEvents');
}
