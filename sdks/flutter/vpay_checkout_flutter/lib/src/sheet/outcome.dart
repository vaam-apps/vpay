/// The single terminal-rule implementation both screen machines share —
/// `machine.ts`'s `intentOutcome`, restated once so the payment machine
/// (`checkout_screen.dart`) and the return machine (`return_screen.dart`)
/// cannot independently drift on the one rule in this port with the
/// highest cost if it regresses.
///
/// **`requires_payment_method` with no `last_payment_error` is NOT
/// terminal.** It is indistinguishable from "nobody has confirmed this
/// intent yet", and treating it as terminal reports failure on a payment
/// that may still be live — issue #189 calls this "the worst defect
/// available here". [outcomeFor] reads the exact two fields
/// `sdks/stripe-js/src/client.ts:673-679`'s poll ladder does, and
/// `models.dart`'s `PaymentIntent.hasStoppedMoving` reads the same two
/// fields independently; the two must never disagree, and both are pinned
/// by their own tests rather than one trusting the other.
library;

import '../models.dart';
import 'failures.dart';

enum OutcomeKind { succeeded, failed, canceled }

/// How a payment ended: the kind, the closed-vocabulary code, and the
/// rail's own sentence where the API gave one.
///
/// [reason] is **not** a substitute for [failure] and must never be shown on
/// its own — see [providerReason] for why one is translated (a later lane's
/// job) and the other is shown as data.
final class Outcome {
  const Outcome({
    required this.kind,
    required this.failure,
    required this.reason,
  });

  final OutcomeKind kind;
  final FailureCode? failure;
  final String? reason;
}

/// Whether `status`/`lastPaymentError` describe a terminal intent, and how.
///
/// `null` means "still in flight". This is the whole rule, and it is
/// intentionally the only place either screen machine evaluates it:
/// [PaymentIntentStatus.succeeded] and [PaymentIntentStatus.canceled] are
/// terminal outright; [PaymentIntentStatus.requiresPaymentMethod] is
/// terminal **only** paired with a non-null [LastPaymentError]. Every other
/// status — `requires_action`, `processing`, `unknown` — is still moving.
Outcome? outcomeFor(
  PaymentIntentStatus status,
  LastPaymentError? lastPaymentError,
) {
  if (status == PaymentIntentStatus.succeeded) {
    return const Outcome(
      kind: OutcomeKind.succeeded,
      failure: null,
      reason: null,
    );
  }
  if (status == PaymentIntentStatus.canceled) {
    return const Outcome(
      kind: OutcomeKind.canceled,
      failure: null,
      reason: null,
    );
  }
  if (status == PaymentIntentStatus.requiresPaymentMethod &&
      lastPaymentError != null) {
    return Outcome(
      kind: OutcomeKind.failed,
      failure: lastPaymentError.code,
      reason: providerReason(lastPaymentError.message),
    );
  }
  return null;
}

/// [outcomeFor] applied to a full [PaymentIntent] — the payment machine's
/// own intent, `client_secret` and all.
Outcome? intentOutcome(PaymentIntent intent) =>
    outcomeFor(intent.status, intent.lastPaymentError);
