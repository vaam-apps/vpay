/// The error half of `vpay_checkout_flutter` — the Dart port of
/// `sdks/stripe-js/src/errors.ts`'s rules, restated for this package:
///
/// 1. **Nothing throws out of a poll.** `BrowserClient`'s two reads answer a
///    `PaymentIntentResult`/`CheckoutSessionResult`, never a rejected
///    `Future` — the same contract Stripe.js keeps, for the same reason: a
///    payer-facing failure is data, not a control-flow surprise the caller
///    must remember to catch.
/// 2. **No error message ever carries a credential.** Every message below
///    is a fixed string, never interpolated from a thrown value or a URL —
///    the query string of every request this package makes holds `key` and
///    `client_secret`.
///
/// The `type`/`code` vocabulary mirrors vpay's server vocabulary exactly,
/// the way `sdks/stripe-js/src/errors.ts`'s does — `invalid_request_error` +
/// `resource_missing` is what `Category::NotFound` renders
/// (`backends/crates/vpay-api/src/browser/mod.rs`'s uniform 404).
library;

/// The four codes this package originates rather than reads off the wire —
/// mirrors `sdks/stripe-js/src/errors.ts`'s `CLIENT_ERROR_CODES`.
abstract final class VpayClientErrorCodes {
  /// The polling budget in `CheckoutController.resolve` elapsed with the
  /// intent still not terminal. Distinct from `VpayCheckoutPending`: this
  /// code is used for a caller that asked to await one poll directly, not
  /// for the "reached a stop URL or was dismissed" flow, which resolves to
  /// `VpayCheckoutPending` rather than an error (D4 — a still-processing
  /// payment is not a failure).
  static const String pollingTimeout = 'polling_timeout';

  /// A response that is not the documented envelope — a proxy's HTML 502,
  /// say.
  static const String unexpectedResponse = 'unexpected_response';

  /// D2: an `embedded` session was named at the pre-flight. Refused before
  /// any window opens.
  static const String embeddedSessionNotSupported =
      'embedded_session_not_supported';

  /// The platform window never opened, or closed without ever reporting
  /// which of its two signals occurred — a browser that refused the popup,
  /// an `Activity` that would not start, a host that dropped the event.
  /// Distinct from every status this package reads off an intent: it means
  /// the *window* failed, so nothing was observed either way.
  static const String platformWindowFailed = 'platform_window_failed';
}

/// vpay's error envelope (`vpay_api::error_envelope_with_param`), narrowed to
/// the members this package can populate — the Dart analogue of
/// `sdks/stripe-js/src/errors.ts`'s `StripeError`.
final class VpayError {
  const VpayError({required this.type, this.code, this.message, this.param});

  /// e.g. `invalid_request_error`, `api_error`, `api_connection_error`.
  final String type;

  /// Machine-readable code, e.g. `resource_missing`.
  final String? code;

  /// Human-readable detail. Never contains a `client_secret` or a
  /// publishable key — every call site below passes a fixed string.
  final String? message;

  /// The offending request parameter, when one can be named.
  final String? param;

  /// A `fetch`/`http` call that never reached the API: DNS, TLS, connection
  /// refused. Deliberately carries no `code` and a fixed message — mirrors
  /// `sdks/stripe-js/src/errors.ts`'s `connectionError`.
  factory VpayError.connection() => const VpayError(
    type: 'api_connection_error',
    message: 'Could not reach the vpay API.',
  );

  /// A response that is not vpay's documented error envelope. Carries the
  /// status, never the body.
  factory VpayError.unexpectedResponse(int status) => VpayError(
    type: 'api_error',
    code: VpayClientErrorCodes.unexpectedResponse,
    message: 'The vpay API returned an unexpected response (HTTP $status).',
  );

  /// An integration mistake this package catches before making a request —
  /// a malformed `clientSecret`, a non-`https` base URL, and so on.
  factory VpayError.invalidRequest(String message, {String? param}) =>
      VpayError(
        type: 'invalid_request_error',
        code: 'invalid_request',
        message: message,
        param: param,
      );

  /// D2: an `embedded` checkout session was named at the pre-flight. A typed
  /// error, refused before any window opens — never a WebView that renders
  /// and then refuses itself.
  factory VpayError.embeddedSessionNotSupported(String sessionId) => VpayError(
    type: 'invalid_request_error',
    code: VpayClientErrorCodes.embeddedSessionNotSupported,
    message:
        'Checkout session $sessionId is ui_mode "embedded"; '
        'vpay_checkout_flutter only opens a hosted session in a native '
        'window.',
  );

  /// The platform window could not be opened, or ended without reporting an
  /// outcome. A **fixed** message on purpose: the thrown value this replaces
  /// is a `PlatformException`/`StateError` whose own message can quote the
  /// URL it failed on, and that URL carries the session's `client_secret` in
  /// its fragment (D6). Nothing from it is interpolated here.
  factory VpayError.platformWindow() => const VpayError(
    type: 'api_error',
    code: VpayClientErrorCodes.platformWindowFailed,
    message:
        'The native checkout window could not be opened, or closed without '
        'reporting an outcome.',
  );

  /// The polling budget elapsed before the intent reached a terminal state,
  /// for a caller that awaits one poll outcome directly rather than going
  /// through `CheckoutController.resolve`, which maps this case to
  /// `VpayCheckoutPending` instead (D4).
  factory VpayError.pollingTimeout() => const VpayError(
    type: 'api_error',
    code: VpayClientErrorCodes.pollingTimeout,
    message: 'Timed out waiting for the payment intent to reach a final state.',
  );

  /// Reads `{ "error": { "type", "code", "message", "param" } }` —
  /// `vpay_api::error_envelope_with_param` — out of a decoded response body.
  ///
  /// `type` is the only member this insists on; `code` is always written
  /// server-side too, but a version skew that dropped it should surface as
  /// a typed error rather than as [VpayError.unexpectedResponse]. Returns
  /// `null` for anything else, and the caller falls back to
  /// [VpayError.unexpectedResponse].
  ///
  /// This is also **the uniform 404**, mapped as one error exactly as
  /// `browser::authenticate` renders it: this function does not special-case
  /// any of the four causes the server itself refuses to distinguish, so the
  /// confidentiality property survives all the way to the Dart client.
  static VpayError? fromEnvelope(Object? body) {
    if (body is! Map) {
      return null;
    }
    final Object? rawError = body['error'];
    if (rawError is! Map) {
      return null;
    }
    final Object? type = rawError['type'];
    if (type is! String) {
      return null;
    }
    String? stringOrNull(Object? key) {
      final Object? value = rawError[key];
      return value is String ? value : null;
    }

    return VpayError(
      type: type,
      code: stringOrNull('code'),
      message: stringOrNull('message'),
      param: stringOrNull('param'),
    );
  }

  /// No override needed to satisfy D6: every member above is either a fixed
  /// string this package wrote or a value that already came from the
  /// server's *error* response, never a session URL or a secret — a
  /// `client_secret` never appears in the query string of a failed request's
  /// error body. `toString` is still written explicitly, rather than left to
  /// Dart's default (which would print the class name only), so a caller
  /// logging a `VpayError` gets something legible without engineering it
  /// into a hazard: nothing here interpolates a value this package did not
  /// itself construct as a fixed string.
  @override
  String toString() =>
      'VpayError(type: $type, code: $code, message: $message, param: $param)';
}
