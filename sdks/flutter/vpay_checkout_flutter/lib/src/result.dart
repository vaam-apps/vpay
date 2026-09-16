/// D4's result type. `VpayCheckoutPending` exists so the plugin never has to
/// choose between lying (`VpayCheckoutSucceeded`/`VpayCheckoutCanceled` for
/// a payment still in flight) and throwing.
///
/// A `sealed class`, so every `switch` a merchant's app writes over a
/// [VpayCheckoutResult] is exhaustive at compile time — the Dart shape of
/// the design doc's own `sealed class VpayCheckoutResult { … }` snippet.
library;

import 'errors.dart';

sealed class VpayCheckoutResult {
  const VpayCheckoutResult({
    required this.sessionId,
    required this.paymentIntentId,
  });

  final String sessionId;
  final String paymentIntentId;
}

/// D1: a UI fact, not a settlement. "A merchant's server must still act on
/// `payment_intent.succeeded` from the webhook" — stated here rather than
/// only in the design doc, because this is the type a merchant's app
/// actually reads.
final class VpayCheckoutSucceeded extends VpayCheckoutResult {
  const VpayCheckoutSucceeded({
    required super.sessionId,
    required super.paymentIntentId,
  });

  @override
  String toString() =>
      'VpayCheckoutSucceeded(sessionId: $sessionId, '
      'paymentIntentId: $paymentIntentId)';
}

/// The rail failed the payment. [code] is `vpay_core::failure`'s closed
/// vocabulary (`docs/flows/failures.md`); [providerMessage] is the rail's
/// own words, carried as data and labelled as the provider's — the same
/// treatment the hosted checkout page gives it.
final class VpayCheckoutFailed extends VpayCheckoutResult {
  const VpayCheckoutFailed({
    required super.sessionId,
    required super.paymentIntentId,
    required this.code,
    required this.providerMessage,
  });

  final String? code;
  final String? providerMessage;

  @override
  String toString() =>
      'VpayCheckoutFailed(sessionId: $sessionId, '
      'paymentIntentId: $paymentIntentId, code: $code)';
}

/// The intent itself reached `canceled` — never produced merely because the
/// payer dismissed the window (D4); only a poll of the intent can produce
/// this.
final class VpayCheckoutCanceled extends VpayCheckoutResult {
  const VpayCheckoutCanceled({
    required super.sessionId,
    required super.paymentIntentId,
  });

  @override
  String toString() =>
      'VpayCheckoutCanceled(sessionId: $sessionId, '
      'paymentIntentId: $paymentIntentId)';
}

/// D4: still processing. Produced when a poll's budget elapsed with the
/// intent not yet terminal — after a reached stop URL, or after a
/// dismissal. Not a failure, and not lied about as one.
final class VpayCheckoutPending extends VpayCheckoutResult {
  const VpayCheckoutPending({
    required super.sessionId,
    required super.paymentIntentId,
  });

  @override
  String toString() =>
      'VpayCheckoutPending(sessionId: $sessionId, '
      'paymentIntentId: $paymentIntentId)';
}

/// The plugin itself could not decide — a typed [VpayError], never a silent
/// guess. Produced by a poll that itself errored (a connection failure, the
/// uniform 404) rather than by the intent reaching any particular status.
final class VpayCheckoutUnresolved extends VpayCheckoutResult {
  const VpayCheckoutUnresolved({
    required super.sessionId,
    required super.paymentIntentId,
    required this.error,
  });

  final VpayError error;

  @override
  String toString() =>
      'VpayCheckoutUnresolved(sessionId: $sessionId, '
      'paymentIntentId: $paymentIntentId, error: $error)';
}
