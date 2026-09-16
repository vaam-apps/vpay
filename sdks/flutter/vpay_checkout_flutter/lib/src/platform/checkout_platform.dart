/// The platform seam (D7, D5). Deliberately **not** the pigeon-generated
/// `VpayCheckoutHostApi` directly: that class is a concrete caller bound to
/// a `BinaryMessenger` the moment it is constructed, which is exactly right
/// for Lane C's real implementations and exactly wrong for a fake this
/// package's own tests substitute — so this abstract class is the thing
/// `VpayCheckout` actually depends on, and a platform implementation wraps
/// the generated Host/Flutter API internally.
///
/// **No android/, ios/, macos/ implementation exists in this repository yet
/// (Lane C).** [VpayCheckoutPlatform.instance] therefore defaults to
/// [UnimplementedVpayCheckoutPlatform], whose every member throws
/// `UnimplementedError` — never a hard-coded plausible answer. This is the
/// parity table's ⛔ "no platform host exists" row, made true by the code
/// rather than only claimed by the doc.
library;

import '../checkout_controller.dart' show StopUrlSpec;
import 'messages.g.dart' show CheckoutWindowEvent, CheckoutWindowMode;

/// What a platform host must do: show a URL, watch for one of a list of
/// stop URLs or a dismissal, and say which — nothing else (design doc, "The
/// shape").
abstract class VpayCheckoutPlatform {
  const VpayCheckoutPlatform();

  /// The active implementation. A platform package
  /// (`sdks/flutter/vpay_checkout_flutter/{android,ios,macos}` or the web
  /// implementation — all Lane C) registers itself here; until one does,
  /// this is [UnimplementedVpayCheckoutPlatform].
  static VpayCheckoutPlatform instance =
      const UnimplementedVpayCheckoutPlatform();

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
