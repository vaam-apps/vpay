/// The read half of `sdks/stripe-js/src/client.ts`, ported to Dart.
///
/// Speaks vpay's `/v1/browser` surface
/// (`backends/crates/vpay-api/src/browser/mod.rs`):
///
/// - `GET /v1/browser/payment_intents/{id}?key=…&client_secret=…`
/// - `GET /v1/browser/checkout/sessions/{id}?key=…&client_secret=…`
///
/// There is no `confirm` here — that stays in the platform window, on the
/// hosted page itself (D3: no JavaScript bridge, nothing for this package to
/// call). This client exists to answer the one question the page's own URL
/// cannot (D1): what did the payment intent actually do.
library;

import 'dart:convert';

import 'package:http/http.dart' as http;

import 'errors.dart';
import 'models.dart';

/// Runs a `fromJson` and answers `null` rather than letting it throw.
///
/// This file's whole contract (`errors.dart`, rule 1) is that **nothing
/// throws out of a poll**: the two reads answer a result object, never a
/// rejected `Future`. `isPaymentIntentJson`/`isCheckoutSessionJson` only
/// check `object` and `id`, so a 200 carrying the right `object` but a
/// missing or wrongly-typed `amount`, `created` or `client_secret` — a
/// version skew, a proxy that rewrote a body — used to escape both reads as
/// a bare `TypeError` and unwind `CheckoutController._resolve` and
/// `VpayCheckout.start` with it. The thrown value is deliberately not
/// inspected or carried: a cast error's message quotes the offending value,
/// and one of these fields *is* a `client_secret` (D6).
T? _decode<T>(T Function() parse) {
  try {
    return parse();
  } on Object {
    return null;
  }
}

/// `{prefix}abc_secret_xyz` split into the object's id — mirrors
/// `sdks/stripe-js/src/client.ts`'s `parseClientSecret`. Deliberately not a
/// regex over the suffix alphabet, for the same reason that file gives: the
/// suffix shape is the server's business, and a client-side pattern that
/// disagreed with a future server change would refuse a valid secret.
class _ParsedSecret {
  const _ParsedSecret(this.id);

  final String id;
}

const String _secretSeparator = '_secret_';

_ParsedSecret? _tryParseClientSecret(String clientSecret, String prefix) {
  final int separator = clientSecret.indexOf(_secretSeparator);
  if (separator == -1) {
    return null;
  }
  final String id = clientSecret.substring(0, separator);
  final String suffix = clientSecret.substring(
    separator + _secretSeparator.length,
  );
  if (!id.startsWith(prefix) || id.length <= prefix.length || suffix.isEmpty) {
    return null;
  }
  return _ParsedSecret(id);
}

/// {@template parse_client_secret}
/// Parses `clientSecret`, or answers a [VpayError] that never echoes the
/// offending value — it is a credential the caller just handed this
/// package.
/// {@endtemplate}
_ParsedSecret? _parseSecretOrNull(String clientSecret, String prefix) {
  if (clientSecret.isEmpty) {
    return null;
  }
  return _tryParseClientSecret(clientSecret, prefix);
}

/// `{ paymentIntent }` or `{ error }` — never both, never a thrown
/// [Future]. Mirrors `sdks/stripe-js/src/types.ts`'s `PaymentIntentResult`.
final class PaymentIntentResult {
  const PaymentIntentResult.ok(this.paymentIntent) : error = null;

  const PaymentIntentResult.err(VpayError this.error) : paymentIntent = null;

  final PaymentIntent? paymentIntent;
  final VpayError? error;

  bool get isError => error != null;
}

/// `{ checkoutSession }` or `{ error }` — the session-read analogue of
/// [PaymentIntentResult].
final class CheckoutSessionResult {
  const CheckoutSessionResult.ok(this.checkoutSession) : error = null;

  const CheckoutSessionResult.err(VpayError this.error)
    : checkoutSession = null;

  final CheckoutSession? checkoutSession;
  final VpayError? error;

  bool get isError => error != null;
}

/// vpay's Dart browser client. Holds a publishable key and a base URL and
/// nothing else — never a `client_secret`, which is why [toString] can be
/// exhaustive rather than a filter (D6).
final class BrowserClient {
  /// [baseUrl] must be an absolute `https://` origin unless
  /// [allowInsecureBaseUrl] is `true` — a named, explicit opt-in for
  /// `compose.demo.yml`, never inferred from a debug build (D6). Trailing
  /// slashes are stripped so `$baseUrl/v1/browser/...` cannot produce a `//`
  /// some ingresses redirect and others 404.
  BrowserClient({
    required String baseUrl,
    required this.publishableKey,
    http.Client? httpClient,
    this.allowInsecureBaseUrl = false,
  }) : baseUrl = _stripTrailingSlashes(baseUrl.trim()),
       _httpClient = httpClient ?? http.Client() {
    final Uri? parsed = Uri.tryParse(this.baseUrl);
    if (this.baseUrl.isEmpty || parsed == null || !parsed.hasScheme) {
      throw ArgumentError.value(baseUrl, 'baseUrl', 'must be an absolute URL');
    }
    if (parsed.scheme != 'https' && !allowInsecureBaseUrl) {
      throw ArgumentError.value(
        baseUrl,
        'baseUrl',
        'must be an absolute https:// URL; pass allowInsecureBaseUrl: true '
            'to opt into a non-https base for a local/demo stack '
            '(compose.demo.yml) — never inferred from a debug build',
      );
    }
  }

  final String baseUrl;
  final String publishableKey;

  /// D6: stated here rather than only in the doc comment above, because it
  /// is a fact a reviewer of this class needs beside the constructor that
  /// enforces it.
  final bool allowInsecureBaseUrl;

  final http.Client _httpClient;

  static String _stripTrailingSlashes(String value) {
    int end = value.length;
    while (end > 0 && value.codeUnitAt(end - 1) == 0x2f) {
      end -= 1;
    }
    return value.substring(0, end);
  }

  /// `GET /v1/browser/payment_intents/{id}?key=…&client_secret=…`.
  Future<PaymentIntentResult> retrievePaymentIntent(String clientSecret) async {
    final _ParsedSecret? parsed = _parseSecretOrNull(clientSecret, 'pi_');
    if (parsed == null) {
      return PaymentIntentResult.err(
        VpayError.invalidRequest(
          'clientSecret is not a vpay payment-intent client secret.',
          param: 'clientSecret',
        ),
      );
    }
    final Uri uri =
        Uri.parse(
          '$baseUrl/v1/browser/payment_intents/${Uri.encodeComponent(parsed.id)}',
        ).replace(
          queryParameters: {
            'key': publishableKey,
            'client_secret': clientSecret,
          },
        );
    final _FetchOutcome outcome = await _fetchJson(uri);
    if (outcome.error != null) {
      return PaymentIntentResult.err(outcome.error!);
    }
    if (outcome.ok! && PaymentIntent.isPaymentIntentJson(outcome.body)) {
      final PaymentIntent? intent = _decode(
        () => PaymentIntent.fromJson((outcome.body! as Map).cast()),
      );
      if (intent != null) {
        return PaymentIntentResult.ok(intent);
      }
      return PaymentIntentResult.err(
        VpayError.unexpectedResponse(outcome.status!),
      );
    }
    if (outcome.ok!) {
      return PaymentIntentResult.err(
        VpayError.unexpectedResponse(outcome.status!),
      );
    }
    return PaymentIntentResult.err(
      VpayError.fromEnvelope(outcome.body) ??
          VpayError.unexpectedResponse(outcome.status!),
    );
  }

  /// `GET /v1/browser/checkout/sessions/{id}?key=…&client_secret=…`.
  Future<CheckoutSessionResult> retrieveCheckoutSession(
    String clientSecret,
  ) async {
    final _ParsedSecret? parsed = _parseSecretOrNull(clientSecret, 'cs_');
    if (parsed == null) {
      return CheckoutSessionResult.err(
        VpayError.invalidRequest(
          'clientSecret is not a vpay checkout-session client secret.',
          param: 'clientSecret',
        ),
      );
    }
    final Uri uri =
        Uri.parse(
          '$baseUrl/v1/browser/checkout/sessions/${Uri.encodeComponent(parsed.id)}',
        ).replace(
          queryParameters: {
            'key': publishableKey,
            'client_secret': clientSecret,
          },
        );
    final _FetchOutcome outcome = await _fetchJson(uri);
    if (outcome.error != null) {
      return CheckoutSessionResult.err(outcome.error!);
    }
    if (outcome.ok! && CheckoutSession.isCheckoutSessionJson(outcome.body)) {
      final CheckoutSession? session = _decode(
        () => CheckoutSession.fromJson(
          (outcome.body! as Map).cast(),
          clientSecret: clientSecret,
        ),
      );
      if (session != null) {
        return CheckoutSessionResult.ok(session);
      }
      return CheckoutSessionResult.err(
        VpayError.unexpectedResponse(outcome.status!),
      );
    }
    if (outcome.ok!) {
      return CheckoutSessionResult.err(
        VpayError.unexpectedResponse(outcome.status!),
      );
    }
    return CheckoutSessionResult.err(
      VpayError.fromEnvelope(outcome.body) ??
          VpayError.unexpectedResponse(outcome.status!),
    );
  }

  /// The transport, shared by both reads — mirrors
  /// `sdks/stripe-js/src/client.ts`'s `#fetchJson`: one place that sets no
  /// `Authorization` header and no cookie, and where no thrown value ever
  /// reaches a message (the query string holds `client_secret`).
  Future<_FetchOutcome> _fetchJson(Uri uri) async {
    late final http.Response response;
    try {
      response = await _httpClient.get(uri);
    } on Object {
      // Deliberately not interpolated — see the module doc comment above.
      return _FetchOutcome.error(VpayError.connection());
    }
    Object? decoded;
    try {
      decoded = response.body.isEmpty ? null : jsonDecode(response.body);
    } on FormatException {
      decoded = null;
    }
    return _FetchOutcome(
      status: response.statusCode,
      ok: response.statusCode >= 200 && response.statusCode < 300,
      body: decoded,
    );
  }

  /// D6: this object holds no secret to redact — a publishable key names a
  /// tenant and authorises nothing
  /// (`backends/crates/vpay-api/src/browser/mod.rs`'s module doc), so it is
  /// rendered as-is, exactly as `sdks/stripe-js`'s `VpayStripe` renders its
  /// own.
  @override
  String toString() =>
      'BrowserClient(baseUrl: $baseUrl, publishableKey: $publishableKey)';
}

final class _FetchOutcome {
  const _FetchOutcome({required this.status, required this.ok, this.body})
    : error = null;

  const _FetchOutcome.error(VpayError this.error)
    : status = null,
      ok = null,
      body = null;

  final int? status;
  final bool? ok;
  final Object? body;
  final VpayError? error;
}
