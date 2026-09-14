// Pigeon spec for `vpay_checkout_flutter`'s platform seam — **definitions
// only**. Design D7
// (`docs/plans/2026-09-13-flutter-plugin.md`): "Pigeon rather than a raw
// `MethodChannel` because the channel carries a credential and an untyped
// `Map<String, dynamic>` across three languages is where that gets
// mistyped."
//
// Lane A (docs/plans/2026-09-13-flutter-plugin-brief.md) owns this file.
// **Lane C implements against it and must not change it** — the shape of
// `ShowCheckoutRequest`/`CheckoutWindowEvent` and the two API surfaces below
// is the frozen contract between the Dart core and every platform host.
//
// Regenerate the Dart side with (run from this package's root):
//
//   dart run pigeon --input pigeons/checkout.dart
//
// which reads the `@ConfigurePigeon` output paths below. Lane C adds native
// output **without editing this file**, by passing `--kotlin_out=…` /
// `--swift_out=…` on the command line — pigeon's CLI flags override
// `PigeonOptions` per invocation, so the native codegen targets never need a
// change here.
//
// # What the platform host is asked to do, and nothing more
//
// Every decision that matters — whether a stop URL was reached, what the
// payer's outcome actually is — lives in Dart (design doc, "The shape").
// This interface is deliberately "stupid": show a URL, watch for one of a
// list of stop URLs (matched scheme+host+port+path, D2) or a dismissal, and
// say which. It does not parse the outcome, does not format money and holds
// no credential beyond the URL it was handed for exactly as long as the
// window is open.
import 'package:pigeon/pigeon.dart';

@ConfigurePigeon(
  PigeonOptions(
    dartOut: 'lib/src/platform/messages.g.dart',
    dartOptions: DartOptions(),
  ),
)
/// One stop URL, already normalised the way `checkout_controller.dart`'s
/// `StopUrlSpec` is: scheme, host, port and path only. Query and fragment
/// are ignored (D2), so they are not carried across the channel at all —
/// there is nothing here for a platform host to compare against them by
/// mistake.
class CheckoutStopUrl {
  CheckoutStopUrl({
    required this.scheme,
    required this.host,
    required this.port,
    required this.path,
  });

  final String scheme;
  final String host;
  final int port;
  final String path;
}

/// D8: which window the platform host shows. Mirrors
/// `vpay_checkout.dart`'s `VpayCheckoutMode` — the two enums are kept
/// distinct on purpose (one is the public Dart API, one is a wire type) so
/// the pigeon-generated side can change shape without touching the public
/// one, but every member here must have a same-named counterpart there.
enum CheckoutWindowMode {
  /// The in-app `WebView`/`WKWebView`/popup (design doc D5).
  inApp,

  /// Custom Tabs on Android, `SFSafariViewController` on iOS below 17.4
  /// (design doc D8) — no custom URL scheme, ever (D8: schemes are
  /// first-come-first-served on Android and any installed app could claim
  /// one).
  externalBrowser,
}

/// What `show` hands the platform host.
///
/// `stopUrls` is empty exactly when the session carries no `success_url` or
/// `cancel_url` at all — an embedded session never reaches here (D2 refuses
/// it at the pre-flight), but a hosted session that only ever forwards one
/// way is real and the platform host must not require both.
class ShowCheckoutRequest {
  ShowCheckoutRequest({
    required this.url,
    required this.stopUrls,
    required this.allowInsecureUrl,
    required this.mode,
  });

  /// The session's own hosted `url` (D6: carries the session secret in its
  /// fragment). The platform host loads exactly this and nothing else — it
  /// does not construct a URL of its own.
  final String url;

  final List<CheckoutStopUrl?> stopUrls;

  /// D6's named insecure opt-in, forwarded so a platform host does not have
  /// to re-derive "is this the demo stack" from the URL's scheme itself.
  final bool allowInsecureUrl;

  /// D8: `inApp` (the default) or `externalBrowser`. A platform host that
  /// has not implemented `externalBrowser` refuses rather than silently
  /// falling back to `inApp` — see `vpay_checkout.dart`'s doc comment on
  /// `VpayCheckoutMode.externalBrowser` for why that fallback is the worse
  /// failure.
  final CheckoutWindowMode mode;
}

/// Which of the two signals `checkout_controller.dart` polls will resolve
/// happened. Never a `succeeded`/`canceled`/`failed` member — the design's
/// whole point (D1) is that this interface cannot say that, only Dart's
/// poll of `/v1/browser/payment_intents/{id}` can.
enum CheckoutWindowOutcome {
  /// A navigation matched one of `ShowCheckoutRequest.stopUrls`.
  stopUrlReached,

  /// The payer dismissed the window (back press, swipe, close) without a
  /// navigation ever matching a stop URL.
  dismissed,
}

/// One event the platform host reports back through
/// [VpayCheckoutFlutterApi.onWindowEvent].
class CheckoutWindowEvent {
  CheckoutWindowEvent({required this.outcome, this.reachedUrl});

  final CheckoutWindowOutcome outcome;

  /// The full navigated-to URL, present only for
  /// [CheckoutWindowOutcome.stopUrlReached] — carried for diagnostics only;
  /// `checkout_controller.dart` does not read anything off it (D1).
  final String? reachedUrl;
}

/// Dart calls into the platform host. One `show`, one `dismiss` — the
/// window's entire vocabulary (design doc, "The shape").
@HostApi()
abstract class VpayCheckoutHostApi {
  /// Opens the native window (Android `Activity`+`WebView`, iOS/macOS
  /// `UIViewController`/`NSViewController`+`WKWebView`, or `window.open` on
  /// web — D5) loading `request.url`. Resolves once the window is showing;
  /// the outcome arrives later, over [VpayCheckoutFlutterApi.onWindowEvent].
  @async
  void show(ShowCheckoutRequest request);

  /// Closes the window if one is open. A no-op if none is.
  @async
  void dismiss();
}

/// The platform host calls back into Dart.
@FlutterApi()
abstract class VpayCheckoutFlutterApi {
  /// Fired exactly once per `show`, with the one outcome that occurred.
  void onWindowEvent(CheckoutWindowEvent event);
}
