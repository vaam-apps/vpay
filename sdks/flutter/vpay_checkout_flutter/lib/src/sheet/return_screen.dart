/// The return page's state machine — the Dart port of
/// `frontends/apps/checkout/src/lib/return.ts`.
///
/// A payer arriving at a return view has come back from a rail's own page,
/// top-level. **There is no intent `client_secret` on this trip.** The
/// `return_token` a payer carries in the return URL authorises reading the
/// session and polling its intent, and nothing else (D6) — so this is a
/// second, separate reducer rather than `checkout_screen.dart`'s reused with
/// a flag: a state machine with a `confirm`-shaped transition in it has no
/// business on a screen that cannot confirm.
///
/// That separation is structural, not just a rule stated in a comment:
/// [ReturnPaymentIntent] has no `client_secret` member at all, so nothing
/// reachable from [ReturnScreenState] can be handed to a confirm call even
/// by mistake.
library;

import '../errors.dart';
import '../models.dart';
import 'outcome.dart';

/// The subset of a payment intent a return read is shown — no
/// `client_secret`. `GET .../checkout/sessions/{id}/return` renders the
/// intent without it (D6), so this type is deliberately missing the member
/// rather than merely unused: there is no way to build a confirm request
/// from it.
final class ReturnPaymentIntent {
  const ReturnPaymentIntent({
    required this.status,
    required this.lastPaymentError,
  });

  final PaymentIntentStatus status;
  final LastPaymentError? lastPaymentError;
}

/// What the return screen knows. The payment machine's `CheckoutContext`
/// carries a full [CheckoutSession] (own `client_secret` included, though
/// redacted by its `toString`); this one is deliberately the same
/// [CheckoutSession] type — the session-level read is identical in both
/// flows — paired with the credential-free [ReturnPaymentIntent] rather than
/// [PaymentIntent].
final class ReturnContext {
  const ReturnContext({
    required this.session,
    required this.intent,
    this.merchantName,
  });

  final CheckoutSession session;
  final ReturnPaymentIntent intent;
  final String? merchantName;
}

/// `return.ts`'s `ReturnState` union: five screens — `loading`, `error`,
/// `expired`, `polling` (with its own poll-failure-notice variant folded
/// into the one type, same as the payment machine's `waiting`), `outcome`,
/// `forwarding`. No `select_rail`, no `collect_msisdn`, no `confirming`: a
/// return read cannot drive a rail.
sealed class ReturnScreenState {
  const ReturnScreenState();
}

final class ReturnLoading extends ReturnScreenState {
  const ReturnLoading();
}

final class ReturnLoadError extends ReturnScreenState {
  const ReturnLoadError(this.error);

  final VpayError error;
}

final class ReturnExpired extends ReturnScreenState {
  const ReturnExpired(this.context);

  final ReturnContext context;
}

final class ReturnPolling extends ReturnScreenState {
  const ReturnPolling({required this.context, this.notice});

  final ReturnContext context;

  /// A poll that could not be answered — the payer stays here, with the
  /// reason shown beside a retry affordance, same treatment the payment
  /// machine's `CheckoutWaiting.notice` gets.
  final String? notice;
}

final class ReturnOutcome extends ReturnScreenState {
  const ReturnOutcome({
    required this.context,
    required this.kind,
    this.failure,
    this.reason,
  });

  final ReturnContext context;
  final OutcomeKind kind;
  final FailureCode? failure;
  final String? reason;
}

final class ReturnForwarding extends ReturnScreenState {
  const ReturnForwarding({
    required this.context,
    required this.kind,
    required this.url,
  });

  final ReturnContext context;
  final OutcomeKind kind;
  final String url;
}

sealed class ReturnEvent {
  const ReturnEvent();
}

final class ReturnRead extends ReturnEvent {
  const ReturnRead(this.context);

  final ReturnContext context;
}

final class ReturnReadFailed extends ReturnEvent {
  const ReturnReadFailed(this.error);

  final VpayError error;
}

final class ReturnPollFailed extends ReturnEvent {
  const ReturnPollFailed(this.problem);

  final String problem;
}

final class ReturnForward extends ReturnEvent {
  const ReturnForward(this.url);

  final String url;
}

const ReturnScreenState returnInitialState = ReturnLoading();

/// The state a read of the return view implies — `return.ts:77-115`'s
/// `stateForReturn`.
///
/// The session's terminal statuses win, then the intent's. Anything else is
/// [ReturnPolling]: the payer is back from the rail but the rail's own
/// answer reaches vpay through the worker's status query, not through the
/// browser, so "I am back" and "it is decided" are different moments this
/// screen must not conflate.
ReturnScreenState stateForReturn(
  ReturnContext context, {
  String? previousNotice,
}) {
  final CheckoutSession session = context.session;
  final ReturnPaymentIntent intent = context.intent;

  if (session.status == CheckoutSessionStatus.complete) {
    return ReturnOutcome(
      context: context,
      kind: OutcomeKind.succeeded,
      failure: null,
      reason: null,
    );
  }
  if (session.status == CheckoutSessionStatus.expired) {
    if (session.paymentStatus == CheckoutSessionPaymentStatus.failed) {
      final Outcome? outcome = outcomeFor(
        intent.status,
        intent.lastPaymentError,
      );
      return ReturnOutcome(
        context: context,
        kind: outcome?.kind ?? OutcomeKind.failed,
        failure: outcome?.failure,
        reason: outcome?.reason,
      );
    }
    return ReturnExpired(context);
  }

  final Outcome? outcome = outcomeFor(intent.status, intent.lastPaymentError);
  if (outcome != null) {
    return ReturnOutcome(
      context: context,
      kind: outcome.kind,
      failure: outcome.failure,
      reason: outcome.reason,
    );
  }
  return ReturnPolling(context: context, notice: previousNotice);
}

/// `return.ts`'s `reduceReturn`. An event that does not apply to the current
/// state returns it unchanged, same rule as the payment machine.
ReturnScreenState reduceReturn(ReturnScreenState state, ReturnEvent event) =>
    switch (event) {
      ReturnRead() => switch (state) {
        ReturnLoading() => stateForReturn(event.context),
        ReturnPolling(:final notice) => stateForReturn(
          event.context,
          previousNotice: notice,
        ),
        _ => state,
      },

      ReturnReadFailed() =>
        state is ReturnLoading ? ReturnLoadError(event.error) : state,

      ReturnPollFailed() =>
        state is ReturnPolling
            ? ReturnPolling(context: state.context, notice: event.problem)
            : state,

      ReturnForward() =>
        state is ReturnOutcome
            ? ReturnForwarding(
                context: state.context,
                kind: state.kind,
                url: event.url,
              )
            : state,
    };
