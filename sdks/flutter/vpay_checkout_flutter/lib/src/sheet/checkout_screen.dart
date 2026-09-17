/// The payment page's state machine, as a pure reducer — the Dart port of
/// `frontends/apps/checkout/src/lib/machine.ts`.
///
/// Pure on purpose, for the same reason `machine.ts` gives: everything that
/// can go wrong on a payment screen goes wrong in the transitions — a status
/// rendered before it was read, a "succeeded" screen shown for an intent
/// that only reached `processing`, a forward fired twice — and a reducer
/// with no HTTP call, no timer and no widget in it is the only version of
/// those transitions a test can enumerate. The impure half (reading the
/// session, calling confirm, running the poll) is a later lane's
/// `SheetController`; this file only ever turns an event plus the previous
/// [CheckoutScreenState] into the next one.
///
/// **Two state machines, not one.** This is the *payment* machine —
/// `return_screen.dart` is the *return* machine, `return.ts`'s port, and it
/// deliberately has no `confirm`-shaped transition: a payer who has come
/// back from a rail's own page authenticates with a `return_token`, not an
/// intent `client_secret` (D6), and cannot confirm anything with it. The two
/// are separate types on purpose — nothing here is reachable from
/// `return_screen.dart` and nothing there is reachable from here, so a
/// caller cannot accidentally wire a return read into a state that expects
/// to be able to confirm.
///
/// **No state carries a status this screen invented.** [CheckoutOutcome] is
/// reachable only from an event whose intent [intentOutcome] judged
/// terminal, and [intentOutcome] reads the exact two fields
/// `sdks/stripe-js/src/client.ts`'s poll ladder does — see its own doc
/// comment for why "bare `requires_payment_method` is not terminal" is the
/// one rule in this file with the highest cost if it regresses.
library;

import '../errors.dart';
import '../models.dart';
import 'outcome.dart';
import 'rails.dart';

export 'outcome.dart' show Outcome, OutcomeKind, intentOutcome;

/// What the screen knows about this payment. Read from the API, never
/// composed here.
///
/// Unlike `machine.ts`'s `CheckoutContext`, [session] is **not** stripped of
/// its own `client_secret` on the way in: `CheckoutSession.toString()`
/// already redacts it (D6, `models.dart`), so there is no bare-object
/// serialisation path here for a second copy to leak through the way a raw
/// `postMessage` payload could in the browser page. [intent] is carried
/// separately from [session.paymentIntent] because it is the *live* value —
/// updated by [CheckoutIntentUpdated] as polling continues — while
/// [session] is only refreshed by an explicit [CheckoutSessionRefreshed].
final class CheckoutContext {
  const CheckoutContext({
    required this.session,
    required this.intent,
    this.merchantName,
    this.allowedMethods,
  });

  final CheckoutSession session;
  final PaymentIntent intent;

  /// The merchant's display name, or `null` for "show a neutral heading".
  /// Not sourced from [session] by this lane — parsing
  /// `CheckoutSessionForPayer.merchant` is a later lane's job; this
  /// constructor takes whatever a caller already has.
  final String? merchantName;

  /// The deployment's `checkout.allowed_methods` (`config.yaml`), or `null`
  /// for "no opinion" — handed straight to [railChoices]. Carried on the
  /// context, not threaded as a third argument to every call, for
  /// `machine.ts`'s own reason: this reducer is pure, so it has no other way
  /// to see a configuration value, and a test cannot forget to pass what is
  /// already on the state.
  final List<String>? allowedMethods;

  CheckoutContext _withIntent(PaymentIntent next) => CheckoutContext(
    session: session,
    intent: next,
    merchantName: merchantName,
    allowedMethods: allowedMethods,
  );

  CheckoutContext _withSession(CheckoutSession next) => CheckoutContext(
    session: next,
    intent: intent,
    merchantName: merchantName,
    allowedMethods: allowedMethods,
  );
}

/// Why the screen will not show a payment form.
///
/// [embedNotAllowed] has no analogue in this port — issue #189 records it as
/// an ADR-worthy drop (`frame.ts`/`origins.ts`/`csp.ts` are web-platform
/// iframe-embedding concepts) — but the *reason* stays in this enum so a
/// caller from a future web-embedding lane has somewhere to put it without
/// this file changing shape again.
enum RefusalReason { embedNotAllowed, noSupportedRail }

/// The whole screen-state union — `machine.ts`'s `CheckoutState`, restated
/// as a Dart `sealed class` hierarchy so every `switch` a caller writes is
/// exhaustive at compile time. Thirteen screens in total, the same count the
/// issue's "bar" section enumerates: [CheckoutRefused]'s two
/// [RefusalReason]s count as two of them, the same way the issue's own list
/// does.
sealed class CheckoutScreenState {
  const CheckoutScreenState();
}

final class CheckoutLoading extends CheckoutScreenState {
  const CheckoutLoading();
}

final class CheckoutLoadError extends CheckoutScreenState {
  const CheckoutLoadError(this.error);

  final VpayError error;
}

final class CheckoutRefused extends CheckoutScreenState {
  const CheckoutRefused({required this.reason, this.context});

  final RefusalReason reason;

  /// `null` when refused before any session read completed (an embed check
  /// that runs before the pre-flight); present when the refusal is D9's
  /// (no supported rail) — a session was read, so its context exists.
  final CheckoutContext? context;
}

final class CheckoutExpired extends CheckoutScreenState {
  const CheckoutExpired(this.context);

  final CheckoutContext context;
}

final class CheckoutSelectRail extends CheckoutScreenState {
  const CheckoutSelectRail({required this.context, required this.rails});

  final CheckoutContext context;

  /// Reached only when [RailChoices.supported] has more than one entry —
  /// `stateForContext`'s own rule; a single supported rail skips straight to
  /// its entry screen.
  final RailChoices rails;
}

/// The push-rail form, and its `msisdn.invalid`-shaped variant (the issue's
/// "collect_msisdn + msisdn.invalid" row) — both this one Dart type, exactly
/// as `machine.ts` renders both as one `{ name: "collect_msisdn" }` member
/// with a nullable `problem`.
final class CheckoutCollectMsisdn extends CheckoutScreenState {
  const CheckoutCollectMsisdn({
    required this.context,
    required this.rails,
    required this.rail,
    this.problem,
  });

  final CheckoutContext context;
  final RailChoices rails;
  final SupportedRail rail;

  /// An i18n key naming the problem (`"msisdn.invalid"`, say) — a `String`
  /// rather than `machine.ts`'s typed `MessageKey`, because the catalogue
  /// that closes that vocabulary is a later lane's (#189, "out of scope for
  /// this lane": i18n catalogue).
  final String? problem;
}

final class CheckoutReadyRedirect extends CheckoutScreenState {
  const CheckoutReadyRedirect({
    required this.context,
    required this.rails,
    required this.rail,
    this.problem,
  });

  final CheckoutContext context;
  final RailChoices rails;
  final SupportedRail rail;
  final String? problem;
}

final class CheckoutConfirming extends CheckoutScreenState {
  const CheckoutConfirming({required this.context, required this.rail});

  final CheckoutContext context;
  final SupportedRail rail;
}

/// The "check your phone" screen, and the issue's "waiting + poll-failure
/// notice with retry" row — again one type with a nullable [notice], not two.
final class CheckoutWaiting extends CheckoutScreenState {
  const CheckoutWaiting({required this.context, this.rail, this.notice});

  final CheckoutContext context;

  /// `null` on a reload/resume: a confirmed intent does not name which rail
  /// confirmed it, so a payer coming back to a `processing` intent sees the
  /// waiting screen with no rail on it — `stateForContext`'s own note.
  final SupportedRail? rail;

  /// A poll that could not be answered. The payer stays here — the payment
  /// is in flight and saying otherwise would be a claim this screen cannot
  /// support — with the reason shown beside a retry affordance.
  final String? notice;
}

final class CheckoutRedirecting extends CheckoutScreenState {
  const CheckoutRedirecting({
    required this.context,
    required this.rail,
    required this.url,
  });

  final CheckoutContext context;
  final SupportedRail rail;
  final String url;
}

/// The issue's "outcome × {succeeded, failed, canceled}" and "outcome with
/// no destination" rows both land here: [kind] carries the three, and "no
/// destination" is simply this state with no [CheckoutForward] event ever
/// having fired — there is nothing about *this* state that differs, which is
/// exactly `machine.ts`'s own shape.
final class CheckoutOutcome extends CheckoutScreenState {
  const CheckoutOutcome({
    required this.context,
    required this.kind,
    this.failure,
    this.reason,
  });

  final CheckoutContext context;
  final OutcomeKind kind;
  final FailureCode? failure;

  /// The rail's own words, cleaned by [providerReason] — never a substitute
  /// for [failure]'s translation.
  final String? reason;
}

final class CheckoutForwarding extends CheckoutScreenState {
  const CheckoutForwarding({
    required this.context,
    required this.kind,
    required this.url,
  });

  final CheckoutContext context;
  final OutcomeKind kind;
  final String url;
}

/// `machine.ts`'s `CheckoutEvent` union.
sealed class CheckoutEvent {
  const CheckoutEvent();
}

final class CheckoutLoaded extends CheckoutEvent {
  const CheckoutLoaded(this.context);

  final CheckoutContext context;
}

final class CheckoutLoadFailed extends CheckoutEvent {
  const CheckoutLoadFailed(this.error);

  final VpayError error;
}

final class CheckoutRefuse extends CheckoutEvent {
  const CheckoutRefuse(this.reason);

  final RefusalReason reason;
}

final class CheckoutChooseRail extends CheckoutEvent {
  const CheckoutChooseRail(this.rail);

  final SupportedRail rail;
}

final class CheckoutBack extends CheckoutEvent {
  const CheckoutBack();
}

final class CheckoutProblem extends CheckoutEvent {
  const CheckoutProblem(this.problem);

  final String problem;
}

final class CheckoutConfirmStarted extends CheckoutEvent {
  const CheckoutConfirmStarted();
}

final class CheckoutIntentUpdated extends CheckoutEvent {
  const CheckoutIntentUpdated(this.intent);

  final PaymentIntent intent;
}

final class CheckoutRedirectRequired extends CheckoutEvent {
  const CheckoutRedirectRequired(this.url);

  final String url;
}

final class CheckoutSessionRefreshed extends CheckoutEvent {
  const CheckoutSessionRefreshed(this.session);

  final CheckoutSession session;
}

final class CheckoutForward extends CheckoutEvent {
  const CheckoutForward(this.url);

  final String url;
}

/// The reducer's own starting point — `machine.ts`'s `INITIAL_STATE`.
const CheckoutScreenState checkoutInitialState = CheckoutLoading();

/// The state a freshly-read session lands in — `machine.ts:251-305`'s
/// `stateForContext`.
///
/// The session's own [CheckoutSession.status] wins where it is terminal,
/// because the worker writes it in the settlement transaction and it is
/// what a merchant's `success_url` correlates against. Where the session is
/// still `open` the intent decides, which is what makes a reload during a
/// push land back on "check your phone" rather than on an empty form.
CheckoutScreenState stateForContext(CheckoutContext context) {
  final CheckoutSession session = context.session;
  final PaymentIntent intent = context.intent;

  if (session.status == CheckoutSessionStatus.complete) {
    return CheckoutOutcome(
      context: context,
      kind: OutcomeKind.succeeded,
      failure: null,
      reason: null,
    );
  }
  if (session.status == CheckoutSessionStatus.expired) {
    if (session.paymentStatus == CheckoutSessionPaymentStatus.failed) {
      final Outcome? outcome = intentOutcome(intent);
      return CheckoutOutcome(
        context: context,
        kind: outcome?.kind ?? OutcomeKind.failed,
        failure: outcome?.failure,
        reason: outcome?.reason,
      );
    }
    return CheckoutExpired(context);
  }

  final Outcome? outcome = intentOutcome(intent);
  if (outcome != null) {
    return CheckoutOutcome(
      context: context,
      kind: outcome.kind,
      failure: outcome.failure,
      reason: outcome.reason,
    );
  }

  final RailChoices rails = railChoices(
    session.rails,
    allowedMethods: context.allowedMethods,
  );

  if (intent.status == PaymentIntentStatus.processing ||
      intent.status == PaymentIntentStatus.requiresAction) {
    // Already confirmed — a reload, or a payer coming back to the tab. The
    // rail that was chosen is not recoverable from the intent (a confirmed
    // intent does not name it), so the waiting screen shows without one.
    return CheckoutWaiting(context: context, rail: null, notice: null);
  }

  if (rails.supported.isEmpty) {
    return CheckoutRefused(
      reason: RefusalReason.noSupportedRail,
      context: context,
    );
  }
  if (rails.supported.length == 1) {
    return _entryStateFor(context, rails, rails.supported.first);
  }
  return CheckoutSelectRail(context: context, rails: rails);
}

/// The first screen for a chosen rail: a form for a push, a button for a
/// redirect.
CheckoutScreenState _entryStateFor(
  CheckoutContext context,
  RailChoices rails,
  SupportedRail rail,
) => rail.spec.flow == RailFlow.push
    ? CheckoutCollectMsisdn(context: context, rails: rails, rail: rail)
    : CheckoutReadyRedirect(context: context, rails: rails, rail: rail);

/// The state a [CheckoutBack] from a rail's entry screen returns to.
CheckoutScreenState _backStateFor(CheckoutContext context, RailChoices rails) =>
    rails.supported.length > 1
    ? CheckoutSelectRail(context: context, rails: rails)
    : stateForContext(context);

/// The whole transition table — `machine.ts`'s `reduce`.
///
/// An event that does not apply to the current state returns [state]
/// unchanged rather than throwing: a payment screen that crashed because a
/// late poll answered after the payer pressed Continue would be a worse bug
/// than a dropped event.
CheckoutScreenState reduceCheckoutScreen(
  CheckoutScreenState state,
  CheckoutEvent event,
) => switch (event) {
  CheckoutLoaded() =>
    state is CheckoutLoading ? stateForContext(event.context) : state,

  CheckoutLoadFailed() =>
    state is CheckoutLoading ? CheckoutLoadError(event.error) : state,

  // Reachable from any state: the embed check runs before the session read
  // and can also be re-run when the parent changes.
  CheckoutRefuse() => CheckoutRefused(
    reason: event.reason,
    context: _contextOfAny(state),
  ),

  CheckoutChooseRail() =>
    state is CheckoutSelectRail
        ? _entryStateFor(state.context, state.rails, event.rail)
        : state,

  CheckoutBack() => switch (state) {
    CheckoutCollectMsisdn(:final context, :final rails) => _backStateFor(
      context,
      rails,
    ),
    CheckoutReadyRedirect(:final context, :final rails) => _backStateFor(
      context,
      rails,
    ),
    _ => state,
  },

  CheckoutProblem() => switch (state) {
    CheckoutCollectMsisdn() => CheckoutCollectMsisdn(
      context: state.context,
      rails: state.rails,
      rail: state.rail,
      problem: event.problem,
    ),
    CheckoutReadyRedirect() => CheckoutReadyRedirect(
      context: state.context,
      rails: state.rails,
      rail: state.rail,
      problem: event.problem,
    ),
    CheckoutWaiting() => CheckoutWaiting(
      context: state.context,
      rail: state.rail,
      notice: event.problem,
    ),
    // A confirm that never reached the rail returns the payer to the screen
    // they submitted from, with the reason shown there.
    CheckoutConfirming(:final context, :final rail) => switch (_entryStateFor(
      context,
      railChoices(
        context.session.rails,
        allowedMethods: context.allowedMethods,
      ),
      rail,
    )) {
      final CheckoutCollectMsisdn withEntry => CheckoutCollectMsisdn(
        context: withEntry.context,
        rails: withEntry.rails,
        rail: withEntry.rail,
        problem: event.problem,
      ),
      final CheckoutReadyRedirect withEntry => CheckoutReadyRedirect(
        context: withEntry.context,
        rails: withEntry.rails,
        rail: withEntry.rail,
        problem: event.problem,
      ),
      final CheckoutScreenState other => other,
    },
    _ => state,
  },

  CheckoutConfirmStarted() => switch (state) {
    CheckoutCollectMsisdn(:final context, :final rail) => CheckoutConfirming(
      context: context,
      rail: rail,
    ),
    CheckoutReadyRedirect(:final context, :final rail) => CheckoutConfirming(
      context: context,
      rail: rail,
    ),
    _ => state,
  },

  CheckoutIntentUpdated() => switch (state) {
    CheckoutConfirming(:final context, :final rail) => _afterIntentUpdate(
      context,
      rail,
      event.intent,
    ),
    CheckoutWaiting(:final context, :final rail) => _afterIntentUpdate(
      context,
      rail,
      event.intent,
    ),
    _ => state,
  },

  CheckoutRedirectRequired() =>
    state is CheckoutConfirming
        ? CheckoutRedirecting(
            context: state.context,
            rail: state.rail,
            url: event.url,
          )
        : state,

  // Only where a fresher session changes nothing about what is on screen.
  // Re-deriving the state here would let a late read move a payer off an
  // outcome they are reading.
  CheckoutSessionRefreshed() =>
    state is CheckoutOutcome
        ? CheckoutOutcome(
            context: state.context._withSession(event.session),
            kind: state.kind,
            failure: state.failure,
            reason: state.reason,
          )
        : state,

  CheckoutForward() =>
    state is CheckoutOutcome
        ? CheckoutForwarding(
            context: state.context,
            kind: state.kind,
            url: event.url,
          )
        : state,
};

CheckoutScreenState _afterIntentUpdate(
  CheckoutContext context,
  SupportedRail? rail,
  PaymentIntent intent,
) {
  final CheckoutContext next = context._withIntent(intent);
  final Outcome? outcome = intentOutcome(intent);
  if (outcome != null) {
    return CheckoutOutcome(
      context: next,
      kind: outcome.kind,
      failure: outcome.failure,
      reason: outcome.reason,
    );
  }
  return CheckoutWaiting(context: next, rail: rail, notice: null);
}

/// The context on any state that carries one, or `null` — the public name
/// for [_contextOfAny], exposed for issue #189 Lane 2's `SheetController`
/// (`sheet/sheet_controller.dart`), which needs the same "does this state
/// have a session on it" question `reduceCheckoutScreen`'s own
/// [CheckoutRefuse] transition asks, to decide what a mid-payment dismissal
/// should report. A thin wrapper rather than making [_contextOfAny] itself
/// public: the underlying `switch` stays private and this file's own
/// exhaustiveness check still catches a state added without updating it.
CheckoutContext? contextOfCheckoutScreen(CheckoutScreenState state) =>
    _contextOfAny(state);

/// The context on any state that carries one, or `null` — used only by the
/// [CheckoutRefuse] transition, which is reachable from every state.
CheckoutContext? _contextOfAny(CheckoutScreenState state) => switch (state) {
  CheckoutLoading() => null,
  CheckoutLoadError() => null,
  CheckoutRefused(:final context) => context,
  CheckoutExpired(:final context) => context,
  CheckoutSelectRail(:final context) => context,
  CheckoutCollectMsisdn(:final context) => context,
  CheckoutReadyRedirect(:final context) => context,
  CheckoutConfirming(:final context) => context,
  CheckoutWaiting(:final context) => context,
  CheckoutRedirecting(:final context) => context,
  CheckoutOutcome(:final context) => context,
  CheckoutForwarding(:final context) => context,
};
