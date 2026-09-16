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
  });

  /// The session's own hosted `url` (D6: carries the session secret in its
  /// fragment). The platform host loads exactly this and nothing else — it
  /// does not construct a URL of its own.
  final String url;

  /// The host matches an **incoming deep link** against these (D2:
  /// scheme+host+port+path, query and fragment ignored). It no longer
  /// matches navigations: since D5 was revised on 2026-09-16 the page runs
  /// in the browser's own process, where no host on any platform can see a
  /// navigation at all. See [CheckoutWindowOutcome.stopUrlReached].
  final List<CheckoutStopUrl?> stopUrls;

  /// D6's named insecure opt-in, forwarded so a platform host does not have
  /// to re-derive "is this the demo stack" from the URL's scheme itself.
  final bool allowInsecureUrl;
}

/// Which of the two signals `checkout_controller.dart` polls will resolve
/// happened. Never a `succeeded`/`canceled`/`failed` member — the design's
/// whole point (D1) is that this interface cannot say that, only Dart's
/// poll of `/v1/browser/payment_intents/{id}` can.
enum CheckoutWindowOutcome {
  /// An **incoming deep link** (Android App Link, iOS/macOS Universal
  /// Link) matched one of `ShowCheckoutRequest.stopUrls`, bringing the app
  /// back to the foreground.
  ///
  /// **Unverified on every platform** as of 2026-09-16, and the status
  /// pages say so: a verified App Link/Universal Link needs an HTTPS
  /// origin serving `assetlinks.json` / `apple-app-site-association` for
  /// the merchant's own `success_url` host, which this repository can
  /// neither deploy nor prove against. Every host below therefore reports
  /// [dismissed] in practice today, and D1/D4 make that
  /// correctness-complete — the poll, not the window, decides. Nothing
  /// here pretends otherwise.
  stopUrlReached,

  /// The payer closed the browser sheet (back press, swipe, "Done") with
  /// no matching deep link having arrived. Since D5's 2026-09-16 revision
  /// this is the **ordinary** end of a successful payment too, not only a
  /// cancellation — which is exactly why D4 polls before answering.
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
  /// Opens the payer's **browser** on `request.url` — a partial (bottom
  /// sheet) Custom Tab on Android, a detented `SFSafariViewController` on
  /// iOS, `NSWorkspace.open` on macOS, `window.open` on web (D5, revised
  /// 2026-09-16). Resolves once the window is showing; the outcome arrives
  /// later, over [VpayCheckoutFlutterApi.onWindowEvent].
  ///
  /// No host renders the page in a `WebView`/`WKWebView` any more. That
  /// was the previous D5 and it put vpay's payment form inside the
  /// merchant app's own process, where `evaluateJavascript`, the cookie
  /// store and a navigation delegate are all reachable — i.e. where a
  /// compromised merchant app could read the payer's PAN and OTP without
  /// the payer being able to tell. The browser's process cannot be
  /// inspected that way, and the payer gets a real URL bar. The cost is
  /// stated rather than hidden: no host can see a navigation any more, so
  /// stop URLs arrive only as deep links — see
  /// [CheckoutWindowOutcome.stopUrlReached].
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
