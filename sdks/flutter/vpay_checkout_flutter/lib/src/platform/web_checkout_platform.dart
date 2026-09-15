// In order to *not* need this ignore, this implementation would have to be
// extracted into a separate package — the convention `flutter create
// --template=plugin` itself uses. Kept inline here because this package is
// deliberately not federated (design doc D7: "there is no third-party
// implementor to accommodate").
// ignore: avoid_web_libraries_in_flutter

/// The web implementation of [VpayCheckoutPlatform] (design doc D5, "Web") —
/// Lane C.
///
/// There is no native window on web at all: [show] opens the session's own
/// `url` in a popup with `window.open`, the same surface
/// `sdks/stripe-js/src/popup.ts` already treats as a first-class peer, and
/// the *only* two signals available to a popup opener are the ones that
/// module documents — a cross-origin popup's `location` cannot be read from
/// the opener at all, which is exactly why that package's protocol exists:
///
/// - a `{type: 'vpay:complete', …}` `postMessage` from the popup's own page
///   (design doc D8's `vpay:complete`, sent by the merchant's own
///   `success_url`/`cancel_url` page once it lands there — never by vpay's
///   checkout page itself, which does not know it is inside a popup), or
/// - the popup's `closed` property going true with no such message having
///   arrived, watched on a short poll.
///
/// [ShowCheckoutRequest.stopUrls] cannot be applied here — there is no
/// navigation to watch — so this implementation does not attempt to; the
/// two signals above are already this platform's own spelling of "reached a
/// stop URL" and "the payer left" (`docs/flows/mobile-checkout.md`, D5). A
/// merchant's own Flutter web app has to be listening for that message on
/// its own origin for the first branch to ever fire — mirroring
/// `OpenCheckoutPopupOptions.completionOrigin`'s default in `popup.ts`,
/// this only ever accepts a message whose `event.origin` is this page's own
/// origin, not the checkout page's — and, mirroring `popup.ts`'s own
/// `event.source !== popup` check, only a message whose `event.source` is
/// the exact popup [show] opened, not merely some other window sharing
/// this page's origin (another tab, another popup, an iframe of the
/// merchant's own).
///
/// **D8's `VpayCheckoutMode` collapses to one behaviour here.** `inApp` and
/// `externalBrowser` both open the same `window.open` popup — there is no
/// in-app WebView on Flutter web to distinguish `inApp` from, and the popup
/// this class already opens for every mode *is* an external-browser
/// context. [show]'s `mode` parameter is accepted, never dropped from the
/// signature (a caller passing `externalBrowser` on web gets a real window,
/// not an `UnimplementedError`), and then intentionally not read.
library;

import 'dart:async';
import 'dart:js_interop';

import 'package:flutter_web_plugins/flutter_web_plugins.dart';
import 'package:web/web.dart' as web;

import '../checkout_controller.dart' show StopUrlSpec;
import 'checkout_platform.dart';
import 'messages.g.dart';

/// The `postMessage` payload's `type`, exactly as `popup.ts`'s
/// `COMPLETE_MESSAGE_TYPE` spells it — the two packages must agree on this
/// string, since one only ever reads what the other's return-page listener
/// (`@vaam-apps/vpay-stripe-js`) writes.
const String _completeMessageType = 'vpay:complete';

/// How often [show] asks whether the payer closed the popup — the same
/// value `popup.ts`'s `CLOSE_POLL_INTERVAL_MS` uses.
const Duration _closePollInterval = Duration(milliseconds: 500);

final class WebVpayCheckoutPlatform extends VpayCheckoutPlatform {
  WebVpayCheckoutPlatform();

  /// Registered by Flutter's web plugin loader (`pubspec.yaml`'s
  /// `flutter: plugin: platforms: web:`), the same `Registrar`-based
  /// convention `flutter create --template=plugin --platforms=web`
  /// generates.
  static void registerWith(Registrar registrar) {
    VpayCheckoutPlatform.instance = WebVpayCheckoutPlatform();
  }

  web.Window? _popup;

  /// One long-lived broadcast controller for the life of this object, not
  /// one per [show].
  ///
  /// Until 2026-09-14 this was created inside `show`, and [windowEvents]
  /// threw a `StateError` when read before it. That forced every caller to
  /// subscribe *after* `show` returned — and `show` returns with the popup
  /// already open, so a payer who closed it in that gap had their event
  /// added to a broadcast stream with no listener, which drops it silently
  /// and hangs the caller forever. A stream that exists before the window
  /// does is what lets `VpayCheckout.start` subscribe first.
  final StreamController<CheckoutWindowEvent> _events =
      StreamController<CheckoutWindowEvent>.broadcast();
  Timer? _closePoll;
  JSFunction? _messageListener;
  bool _settled = false;

  @override
  Future<void> show({
    required String url,
    // Not applicable on web — see this file's doc comment: a cross-origin
    // popup's location cannot be read from the opener at all, so there is
    // no navigation here for a stop URL to match against.
    required List<StopUrlSpec> stopUrls,
    // Not applicable on web either: `allowInsecureUrl` exists so a native
    // WebView host can be told the demo stack's `http://` URL is expected
    // rather than a mistake (D6). A browser's own address bar already shows
    // the payer whatever scheme `window.open` navigated to; there is no
    // separate host-side check to relax.
    required bool allowInsecureUrl,
    // D8's `inApp`/`externalBrowser` choice is not applicable on web
    // either, and not for the same reason as the two params above: it is
    // not that this platform ignores the distinction, it is that the
    // distinction does not exist here. There is no in-app WebView option
    // on Flutter web at all (design doc, "The shape": web's row is
    // `window.open` popup, "no plugin window") — a `window.open` popup
    // already **is** an external-browser context, the same surface
    // `sdks/stripe-js/src/popup.ts` treats as a first-class peer for every
    // integration on the web. So `inApp` and `externalBrowser` open the
    // identical popup here; this parameter is accepted (never silently
    // dropped from the signature — a caller passing `externalBrowser` gets
    // a real window, not an `UnimplementedError`) and then intentionally
    // not read.
    required CheckoutWindowMode mode,
  }) async {
    _teardown();
    _settled = false;

    final web.Window? popup = web.window.open(
      url,
      'vpay-checkout',
      'popup=yes,location=yes,resizable=yes,scrollbars=yes',
    );
    if (popup == null) {
      throw StateError(
        'vpay_checkout_flutter (web): the browser refused to open the '
        'checkout popup. Call VpayCheckout.start from a click handler, or '
        'use a full-page redirect instead.',
      );
    }
    _popup = popup;

    void onMessage(web.Event event) {
      final web.MessageEvent messageEvent = event as web.MessageEvent;
      if (messageEvent.origin != web.window.location.origin) {
        return;
      }
      // …and it must be *this* window, not merely something else on the
      // same origin — the same second check `popup.ts`'s own `onMessage`
      // makes ("it must be *this* window, not merely something else on the
      // same origin"): another tab, another popup, an iframe of the
      // merchant's own can all share this page's origin and still hold a
      // `postMessage` handle to it. `_popup` is the exact `web.Window` this
      // call to [show] opened; `messageEvent.source` not being that
      // reference — including when `_popup` is already `null` because
      // [_teardown] already ran — rejects the message rather than trusting
      // it.
      if (messageEvent.source != _popup) {
        return;
      }
      final Object? data = messageEvent.data?.dartify();
      if (data is Map && data['type'] == _completeMessageType) {
        _reportOnce(CheckoutWindowOutcome.stopUrlReached);
      }
    }

    final JSFunction listener = onMessage.toJS;
    _messageListener = listener;
    web.window.addEventListener('message', listener);

    _closePoll = Timer.periodic(_closePollInterval, (_) {
      if (_popup?.closed ?? true) {
        _reportOnce(CheckoutWindowOutcome.dismissed);
      }
    });
  }

  void _reportOnce(CheckoutWindowOutcome outcome) {
    if (_settled || _events.isClosed) {
      return;
    }
    _settled = true;
    _events.add(CheckoutWindowEvent(outcome: outcome));
    _popup?.close();
    _teardown();
  }

  void _teardown() {
    _closePoll?.cancel();
    _closePoll = null;
    final JSFunction? listener = _messageListener;
    if (listener != null) {
      web.window.removeEventListener('message', listener);
    }
    _messageListener = null;
    _popup = null;
  }

  @override
  Future<void> dismiss() async {
    _popup?.close();
  }

  /// Readable at any time, including before [show] — see [_events].
  @override
  Stream<CheckoutWindowEvent> get windowEvents => _events.stream;
}
