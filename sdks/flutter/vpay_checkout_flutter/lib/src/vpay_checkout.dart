/// The public entry point: `VpayCheckout.start(sessionUrl)`
/// (design doc, "The shape") — parse, pre-flight, open, watch, resolve,
/// answer.
library;

import 'dart:async';

import 'package:http/http.dart' as http;

import 'browser_client.dart';
import 'checkout_controller.dart';
import 'errors.dart';
import 'platform/checkout_platform.dart';
import 'platform/messages.g.dart'
    show CheckoutWindowEvent, CheckoutWindowMode, CheckoutWindowOutcome;
import 'result.dart';

/// D8: the in-app WebView (default) or the external-browser mode. Both are
/// resolved identically by [CheckoutController] — the mode only changes
/// what the platform host shows, never how the outcome is decided (D1).
///
/// Kept distinct from `pigeons/checkout.dart`'s `CheckoutWindowMode` (the
/// wire type [_toWire] converts to) rather than exporting that one
/// directly: this is the public Dart API and must not change shape just
/// because the pigeon-generated seam does.
enum VpayCheckoutMode {
  /// The in-app `WebView`/`WKWebView`/popup (design doc D5). The default,
  /// and the only mode with no scheme-hijack surface at all.
  inApp,

  /// Custom Tabs (`androidx.browser`) on Android, `SFSafariViewController`
  /// on iOS below 17.4 (design doc D8). Android is wired end to end and
  /// compiles for real; iOS/macOS are wired the same way but compiled by
  /// nobody (no Xcode on this host); web opens the same popup it always
  /// does, because a `window.open` popup already is an external-browser
  /// context — see `WebVpayCheckoutPlatform`'s own doc comment.
  ///
  /// **No custom URL scheme, on any platform, ever** (design doc D8):
  /// `checked_forward_url` accepts only `http(s)`, and schemes are
  /// first-come-first-served on Android, so a scheme callback could be
  /// hijacked by any installed app. Return detection is therefore D8's
  /// "tier 0" — the payer finishes on the merchant's own page and the
  /// plugin polls on resume — which D1 already makes correctness-complete:
  /// the window never decides an outcome, only `/v1/browser` does.
  externalBrowser,
}

extension on VpayCheckoutMode {
  CheckoutWindowMode get _toWire => switch (this) {
    VpayCheckoutMode.inApp => CheckoutWindowMode.inApp,
    VpayCheckoutMode.externalBrowser => CheckoutWindowMode.externalBrowser,
  };
}

/// `{base}/c/{cs_id}?key={pk}#{cs_secret}` (D6), split into the one thing
/// this package needs out of it: the fragment. The query's `key` is not
/// re-parsed here — `VpayCheckout` already holds the publishable key it was
/// constructed with, and re-deriving a second copy from the URL would be two
/// places that could disagree.
final class _SessionUrl {
  const _SessionUrl({
    required this.clientSecret,
    required this.bestEffortSessionId,
  });

  final String clientSecret;

  /// A label for [VpayCheckoutUnresolved] when the pre-flight itself never
  /// ran — best-effort only. `BrowserClient.retrieveCheckoutSession` is
  /// what actually validates the secret's shape; this is not a second
  /// validator, only a display string.
  final String bestEffortSessionId;

  static _SessionUrl? parse(String url) {
    final int hash = url.indexOf('#');
    if (hash == -1 || hash == url.length - 1) {
      return null;
    }
    final String secret = url.substring(hash + 1);
    final int separator = secret.indexOf('_secret_');
    final String label = separator == -1 ? '' : secret.substring(0, separator);
    return _SessionUrl(clientSecret: secret, bestEffortSessionId: label);
  }
}

/// vpay's Flutter checkout entry point. Constructed once per deployment
/// (base URL + publishable key), the way `sdks/stripe-js`'s `loadStripe`
/// answers a `Stripe` bound to one.
final class VpayCheckout {
  VpayCheckout({
    required String baseUrl,
    required String publishableKey,
    http.Client? httpClient,
    bool allowInsecureBaseUrl = false,
    VpayClock clock = const SystemClock(),
  }) : _controller = CheckoutController(
         client: BrowserClient(
           baseUrl: baseUrl,
           publishableKey: publishableKey,
           httpClient: httpClient,
           allowInsecureBaseUrl: allowInsecureBaseUrl,
         ),
         clock: clock,
       );

  final CheckoutController _controller;

  /// The whole flow (design doc, "The shape"). Never reads the outcome off
  /// `sessionUrl` or off anything the platform window navigates to (D1) —
  /// only [BrowserClient.retrievePaymentIntent], through [_controller],
  /// decides.
  Future<VpayCheckoutResult> start(
    String sessionUrl, {
    VpayCheckoutMode mode = VpayCheckoutMode.inApp,
  }) async {
    final _SessionUrl? parsed = _SessionUrl.parse(sessionUrl);
    if (parsed == null) {
      return VpayCheckoutUnresolved(
        sessionId: '',
        paymentIntentId: '',
        error: VpayError.invalidRequest(
          'sessionUrl must carry a client secret in its fragment '
          '(vpay_api renders "{base}/c/{id}?key=…#{client_secret}").',
        ),
      );
    }

    final CheckoutPreflight preflight = await _controller.preflight(
      parsed.clientSecret,
    );
    switch (preflight) {
      case CheckoutPreflightFailure(:final error):
        return VpayCheckoutUnresolved(
          sessionId: parsed.bestEffortSessionId,
          paymentIntentId: '',
          error: error,
        );
      case CheckoutPreflightSuccess(:final ready):
        return _showAndResolve(sessionUrl, ready, mode);
    }
  }

  /// D5/D7: the platform host shows the URL and reports one of two signals.
  ///
  /// The subscription is taken **before** [VpayCheckoutPlatform.show] is
  /// called, not after. Both platform implementations publish onto a
  /// broadcast stream, which drops anything added while nobody is listening;
  /// subscribing after `show` resolved left a window — small on Android,
  /// where the event cannot arrive before `startActivityForResult` returns,
  /// and real on web, where `show` completes with a popup already open and a
  /// payer who closes it instantly — in which the one event this flow waits
  /// for could be lost and `start` would never return.
  Future<VpayCheckoutResult> _showAndResolve(
    String sessionUrl,
    CheckoutPreflightReady ready,
    VpayCheckoutMode mode,
  ) async {
    // `VpayCheckoutPlatform.instance` is `UnimplementedVpayCheckoutPlatform`
    // unless a host registered itself; reading `windowEvents` off it throws
    // `UnimplementedError`, deliberately — see docs/sdks/parity.md.
    final VpayCheckoutPlatform platform = VpayCheckoutPlatform.instance;
    final Completer<CheckoutWindowEvent> settled =
        Completer<CheckoutWindowEvent>();
    final StreamSubscription<CheckoutWindowEvent> subscription = platform
        .windowEvents
        .listen(
          (CheckoutWindowEvent event) {
            if (!settled.isCompleted) {
              settled.complete(event);
            }
          },
          onError: (Object _) {
            // Deliberately not interpolated: a platform error can carry the
            // URL it failed on, and that URL carries the session secret
            // (D6).
            if (!settled.isCompleted) {
              settled.completeError(const _WindowNeverReported());
            }
          },
          onDone: () {
            if (!settled.isCompleted) {
              settled.completeError(const _WindowNeverReported());
            }
          },
        );

    try {
      try {
        await platform.show(
          url: sessionUrl,
          stopUrls: ready.stopUrls,
          allowInsecureUrl: _controller.client.allowInsecureBaseUrl,
          mode: mode._toWire,
        );
      } on UnimplementedError {
        // The honest "no host here" gap, not a runtime failure — it must
        // reach the caller as itself.
        rethrow;
      } on Object {
        // A popup the browser refused, an Android `Activity` that would not
        // start: the window never opened, so no money can have moved. A
        // typed result, and never the thrown value's own message, which on
        // Android or web can quote the URL (D6).
        return VpayCheckoutUnresolved(
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
          error: VpayError.platformWindow(),
        );
      }

      final CheckoutWindowEvent event;
      try {
        event = await settled.future;
      } on _WindowNeverReported {
        return VpayCheckoutUnresolved(
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
          error: VpayError.platformWindow(),
        );
      }

      // Awaited inside the `try` on purpose: the `finally` below cancels
      // the subscription, and cancelling it while the poll is still running
      // would be a live window with nothing listening to it.
      return await (event.outcome == CheckoutWindowOutcome.stopUrlReached
          ? _controller.resolveAfterStopUrlReached(ready)
          : _controller.resolveAfterDismissal(ready));
    } finally {
      await subscription.cancel();
    }
  }
}

/// The window's event stream ended, or errored, without ever reporting an
/// outcome — a platform-host bug. Private: it never leaves this file, it is
/// mapped to [VpayError.platformWindow] at the one place it is caught.
final class _WindowNeverReported implements Exception {
  const _WindowNeverReported();
}
