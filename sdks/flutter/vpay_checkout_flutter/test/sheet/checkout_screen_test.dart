import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

const String _piSecret = 'pi_123_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const String _csSecret = 'cs_123_secret_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

PaymentIntent _intent({
  PaymentIntentStatus status = PaymentIntentStatus.requiresPaymentMethod,
  LastPaymentError? lastPaymentError,
  String? redirectUrl,
}) => PaymentIntent(
  id: 'pi_123',
  amount: 5000,
  currency: 'xaf',
  status: status,
  paymentMethodTypes: const ['mtn_momo'],
  lastPaymentError: lastPaymentError,
  metadata: const {},
  description: null,
  created: 1700000000,
  livemode: false,
  clientSecret: _piSecret,
  redirectUrl: redirectUrl,
);

final RailSpec _mtnPush = RailSpec(
  code: 'mtn_momo',
  flow: RailFlow.push,
  labelKey: 'rail.mtn_momo',
  displayName: null,
  fields: const [
    RailField(
      name: 'msisdn',
      kind: RailFieldKindPhone(
        region: 'CM',
        phoneType: RailFieldPhoneType.mobile,
      ),
      required: true,
      labelKey: 'msisdn.label',
    ),
  ],
);

final RailSpec _orangeRedirect = RailSpec(
  code: 'orange_money',
  flow: RailFlow.redirect,
  labelKey: 'rail.orange_money',
  displayName: null,
  fields: const [],
);

CheckoutSession _session({
  CheckoutSessionStatus status = CheckoutSessionStatus.open,
  CheckoutSessionPaymentStatus paymentStatus =
      CheckoutSessionPaymentStatus.unpaid,
  List<RailSpec> rails = const [],
  PaymentIntent? intent,
}) => CheckoutSession(
  id: 'cs_123',
  livemode: false,
  paymentIntent: intent ?? _intent(),
  uiMode: CheckoutUiMode.hosted,
  status: status,
  paymentStatus: paymentStatus,
  successUrl: null,
  cancelUrl: null,
  url: null,
  expiresAt: 1700086400,
  created: 1700000000,
  clientSecret: _csSecret,
  rails: rails,
);

CheckoutContext _context({
  CheckoutSession? session,
  PaymentIntent? intent,
  List<String>? allowedMethods,
}) {
  final PaymentIntent theIntent = intent ?? _intent();
  return CheckoutContext(
    session: session ?? _session(intent: theIntent),
    intent: theIntent,
    allowedMethods: allowedMethods,
  );
}

void main() {
  group('stateForContext — the session and intent decide, never a widget', () {
    test(
      'a complete session is an outcome:succeeded, regardless of the intent',
      () {
        final CheckoutContext context = _context(
          session: _session(status: CheckoutSessionStatus.complete),
        );
        final CheckoutScreenState state = stateForContext(context);
        expect(state, isA<CheckoutOutcome>());
        expect((state as CheckoutOutcome).kind, OutcomeKind.succeeded);
      },
    );

    test('an expired session with payment_status != failed is the neutral expired screen', () {
      final CheckoutContext context = _context(
        session: _session(status: CheckoutSessionStatus.expired),
      );
      expect(stateForContext(context), isA<CheckoutExpired>());
    });

    test('an expired session with payment_status == failed reports the '
        'intent\'s own outcome instead of the neutral expired screen', () {
      final PaymentIntent failedIntent = _intent(
        lastPaymentError: const LastPaymentError(
          code: 'insufficient_funds',
          message: 'Not enough funds.',
        ),
      );
      final CheckoutContext context = _context(
        intent: failedIntent,
        session: _session(
          status: CheckoutSessionStatus.expired,
          paymentStatus: CheckoutSessionPaymentStatus.failed,
          intent: failedIntent,
        ),
      );
      final CheckoutScreenState state = stateForContext(context);
      expect(state, isA<CheckoutOutcome>());
      final CheckoutOutcome outcome = state as CheckoutOutcome;
      expect(outcome.kind, OutcomeKind.failed);
      expect(outcome.failure, 'insufficient_funds');
      expect(outcome.reason, 'Not enough funds.');
    });

    test(
      'expired + payment_status failed, but the intent itself never went '
      'terminal, still falls back to failed rather than the neutral screen',
      () {
        // machine.ts's own `?? "failed"` fallback: the session already says
        // the payment failed even if the intent read cannot corroborate a
        // reason.
        final CheckoutContext context = _context(
          session: _session(
            status: CheckoutSessionStatus.expired,
            paymentStatus: CheckoutSessionPaymentStatus.failed,
          ),
        );
        final CheckoutScreenState state = stateForContext(context);
        expect(state, isA<CheckoutOutcome>());
        final CheckoutOutcome outcome = state as CheckoutOutcome;
        expect(outcome.kind, OutcomeKind.failed);
        expect(outcome.failure, isNull);
        expect(outcome.reason, isNull);
      },
    );

    test(
      'an open session whose intent already succeeded is outcome:succeeded',
      () {
        final PaymentIntent intent = _intent(
          status: PaymentIntentStatus.succeeded,
        );
        final CheckoutContext context = _context(intent: intent);
        final CheckoutScreenState state = stateForContext(context);
        expect(state, isA<CheckoutOutcome>());
        expect((state as CheckoutOutcome).kind, OutcomeKind.succeeded);
      },
    );

    test(
      'an open session whose intent already canceled is outcome:canceled',
      () {
        final PaymentIntent intent = _intent(
          status: PaymentIntentStatus.canceled,
        );
        final CheckoutContext context = _context(intent: intent);
        final CheckoutScreenState state = stateForContext(context);
        expect(state, isA<CheckoutOutcome>());
        expect((state as CheckoutOutcome).kind, OutcomeKind.canceled);
      },
    );

    test('MUTATION PROOF (poll terminal rule): requires_payment_method with NO '
        'last_payment_error is NOT terminal — the session stays on a form '
        'screen, never outcome:failed', () {
      final PaymentIntent stillUnconfirmed = _intent(
        status: PaymentIntentStatus.requiresPaymentMethod,
        lastPaymentError: null,
      );
      final CheckoutContext context = _context(
        intent: stillUnconfirmed,
        session: _session(rails: [_mtnPush], intent: stillUnconfirmed),
      );
      final CheckoutScreenState state = stateForContext(context);
      // Not an outcome of any kind — a bare requires_payment_method with
      // no error is indistinguishable from "nobody has confirmed yet".
      expect(state, isNot(isA<CheckoutOutcome>()));
      expect(state, isA<CheckoutCollectMsisdn>());
    });

    test('requires_payment_method WITH a last_payment_error IS terminal — '
        'outcome:failed, with the rail\'s reason carried as data', () {
      final PaymentIntent failed = _intent(
        lastPaymentError: const LastPaymentError(
          code: 'payer_declined',
          message: 'The payer declined the prompt.',
        ),
      );
      final CheckoutContext context = _context(intent: failed);
      final CheckoutScreenState state = stateForContext(context);
      expect(state, isA<CheckoutOutcome>());
      final CheckoutOutcome outcome = state as CheckoutOutcome;
      expect(outcome.kind, OutcomeKind.failed);
      expect(outcome.failure, 'payer_declined');
      expect(outcome.reason, 'The payer declined the prompt.');
    });

    test('a still-moving intent with zero supported rails is refused (D9)', () {
      final CheckoutContext context = _context(
        session: _session(rails: const []),
      );
      final CheckoutScreenState state = stateForContext(context);
      expect(state, isA<CheckoutRefused>());
      final CheckoutRefused refused = state as CheckoutRefused;
      expect(refused.reason, RefusalReason.noSupportedRail);
      expect(refused.context, isNotNull);
    });

    test(
      'exactly one supported push rail skips straight to collect_msisdn',
      () {
        final CheckoutContext context = _context(
          session: _session(rails: [_mtnPush]),
        );
        final CheckoutScreenState state = stateForContext(context);
        expect(state, isA<CheckoutCollectMsisdn>());
        expect((state as CheckoutCollectMsisdn).rail.code, 'mtn_momo');
        expect(state.problem, isNull);
      },
    );

    test(
      'exactly one supported redirect rail skips straight to ready_redirect',
      () {
        final CheckoutContext context = _context(
          session: _session(rails: [_orangeRedirect]),
        );
        final CheckoutScreenState state = stateForContext(context);
        expect(state, isA<CheckoutReadyRedirect>());
        expect((state as CheckoutReadyRedirect).rail.code, 'orange_money');
      },
    );

    test('more than one supported rail is select_rail', () {
      final CheckoutContext context = _context(
        session: _session(rails: [_mtnPush, _orangeRedirect]),
      );
      final CheckoutScreenState state = stateForContext(context);
      expect(state, isA<CheckoutSelectRail>());
      expect((state as CheckoutSelectRail).rails.supported, hasLength(2));
    });

    test('a processing intent is "waiting" with no rail named, even with several rails offered', () {
      final PaymentIntent processing = _intent(
        status: PaymentIntentStatus.processing,
      );
      final CheckoutContext context = _context(
        intent: processing,
        session: _session(
          rails: [_mtnPush, _orangeRedirect],
          intent: processing,
        ),
      );
      final CheckoutScreenState state = stateForContext(context);
      expect(state, isA<CheckoutWaiting>());
      expect((state as CheckoutWaiting).rail, isNull);
      expect(state.notice, isNull);
    });

    test('requires_action offers to resume the redirect, not a false "check '
        'your phone"', () {
      // The bug this pins, fixed 2026-09-17 on both surfaces:
      // `requires_action` used to share the `waiting` branch with
      // `processing`. They are opposites — `processing` means the rail is
      // still moving, `requires_action` means the PAYER has a redirect to
      // finish and, if they abandoned it, nothing changes again until
      // they do.
      final PaymentIntent requiresAction = _intent(
        status: PaymentIntentStatus.requiresAction,
        redirectUrl: 'https://rail.example/hosted/tok_abc',
      );
      final CheckoutContext context = _context(intent: requiresAction);
      final CheckoutScreenState state = stateForContext(context);
      expect(state, isA<CheckoutResumeRedirect>());
      expect(
        (state as CheckoutResumeRedirect).url,
        'https://rail.example/hosted/tok_abc',
      );
      // Not recoverable from a bare intent, same as `CheckoutWaiting`.
      expect(state.rail, isNull);
    });

    test('requires_action with no redirect url falls back to waiting rather '
        'than inventing a screen with nowhere to go', () {
      // `vpay-api`'s `rendered_intent` guarantees `next_action` on a
      // `requires_action` intent, so this is the server-is-broken path.
      // Falling back is the same "learnt nothing that says otherwise"
      // answer the reducer gives elsewhere.
      final PaymentIntent requiresAction = _intent(
        status: PaymentIntentStatus.requiresAction,
      );
      final CheckoutContext context = _context(intent: requiresAction);
      expect(stateForContext(context), isA<CheckoutWaiting>());
    });
  });

  group('reduceCheckoutScreen — loading, load_failed, refuse', () {
    test('loaded from loading enters stateForContext\'s answer', () {
      final CheckoutContext context = _context(
        session: _session(rails: [_mtnPush]),
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        checkoutInitialState,
        CheckoutLoaded(context),
      );
      expect(state, isA<CheckoutCollectMsisdn>());
    });

    test('loaded is dropped once past loading', () {
      final CheckoutScreenState state = reduceCheckoutScreen(
        CheckoutExpired(_TestContext.dummy),
        CheckoutLoaded(_context()),
      );
      expect(state, isA<CheckoutExpired>());
    });

    test('load_failed from loading becomes a typed error', () {
      const VpayError error = VpayError(
        type: 'api_error',
        code: 'unexpected_response',
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        checkoutInitialState,
        const CheckoutLoadFailed(error),
      );
      expect(state, isA<CheckoutLoadError>());
      expect((state as CheckoutLoadError).error, same(error));
    });

    test('refuse is reachable from any state, carrying whatever context that state had', () {
      final CheckoutContext context = _context();
      final CheckoutScreenState fromLoading = reduceCheckoutScreen(
        checkoutInitialState,
        const CheckoutRefuse(RefusalReason.embedNotAllowed),
      );
      expect(fromLoading, isA<CheckoutRefused>());
      expect((fromLoading as CheckoutRefused).context, isNull);

      final CheckoutScreenState fromExpired = reduceCheckoutScreen(
        CheckoutExpired(context),
        const CheckoutRefuse(RefusalReason.embedNotAllowed),
      );
      expect((fromExpired as CheckoutRefused).context, same(context));
    });
  });

  group('reduceCheckoutScreen — choosing a rail and going back', () {
    test(
      'choose_rail from select_rail enters the chosen rail\'s entry screen',
      () {
        final CheckoutContext context = _context(
          session: _session(rails: [_mtnPush, _orangeRedirect]),
        );
        final RailChoices choices = railChoices([_mtnPush, _orangeRedirect]);
        final CheckoutScreenState selectRail = CheckoutSelectRail(
          context: context,
          rails: choices,
        );
        final CheckoutScreenState state = reduceCheckoutScreen(
          selectRail,
          CheckoutChooseRail(choices.supported.first),
        );
        expect(state, isA<CheckoutCollectMsisdn>());
      },
    );

    test('choose_rail is dropped outside select_rail', () {
      final CheckoutScreenState state = reduceCheckoutScreen(
        checkoutInitialState,
        CheckoutChooseRail(SupportedRail(_mtnPush)),
      );
      expect(state, isA<CheckoutLoading>());
    });

    test(
      'back from collect_msisdn with >1 supported rails returns to select_rail',
      () {
        final CheckoutContext context = _context(
          session: _session(rails: [_mtnPush, _orangeRedirect]),
        );
        final RailChoices choices = railChoices([_mtnPush, _orangeRedirect]);
        final CheckoutScreenState collecting = CheckoutCollectMsisdn(
          context: context,
          rails: choices,
          rail: choices.supported.first,
        );
        final CheckoutScreenState state = reduceCheckoutScreen(
          collecting,
          const CheckoutBack(),
        );
        expect(state, isA<CheckoutSelectRail>());
      },
    );

    test('back from the entry screen of the ONLY supported rail re-derives from scratch', () {
      final CheckoutContext context = _context(
        session: _session(rails: [_mtnPush]),
      );
      final RailChoices choices = railChoices([_mtnPush]);
      final CheckoutScreenState collecting = CheckoutCollectMsisdn(
        context: context,
        rails: choices,
        rail: choices.supported.first,
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        collecting,
        const CheckoutBack(),
      );
      // Only one supported rail: `stateForContext` lands back on the same
      // entry screen, not select_rail (there is nothing to select between).
      expect(state, isA<CheckoutCollectMsisdn>());
    });

    test('back is dropped outside collect_msisdn/ready_redirect', () {
      final CheckoutScreenState state = reduceCheckoutScreen(
        checkoutInitialState,
        const CheckoutBack(),
      );
      expect(state, isA<CheckoutLoading>());
    });
  });

  group('reduceCheckoutScreen — problem', () {
    test('on collect_msisdn, attaches the problem without losing the rail', () {
      final CheckoutContext context = _context(
        session: _session(rails: [_mtnPush]),
      );
      final RailChoices choices = railChoices([_mtnPush]);
      final CheckoutScreenState collecting = CheckoutCollectMsisdn(
        context: context,
        rails: choices,
        rail: choices.supported.first,
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        collecting,
        const CheckoutProblem('msisdn.invalid'),
      );
      expect(state, isA<CheckoutCollectMsisdn>());
      expect((state as CheckoutCollectMsisdn).problem, 'msisdn.invalid');
      expect(state.rail.code, 'mtn_momo');
    });

    test('on waiting, becomes the notice', () {
      final CheckoutContext context = _context();
      final CheckoutScreenState waiting = CheckoutWaiting(context: context);
      final CheckoutScreenState state = reduceCheckoutScreen(
        waiting,
        const CheckoutProblem('error.network'),
      );
      expect(state, isA<CheckoutWaiting>());
      expect((state as CheckoutWaiting).notice, 'error.network');
    });

    test('on confirming, returns to the entry screen the payer submitted from, '
        'with the problem shown there', () {
      final CheckoutContext context = _context(
        session: _session(rails: [_mtnPush]),
      );
      final SupportedRail rail = railChoices([_mtnPush]).supported.first;
      final CheckoutScreenState confirming = CheckoutConfirming(
        context: context,
        rail: rail,
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        confirming,
        const CheckoutProblem('error.unexpected'),
      );
      expect(state, isA<CheckoutCollectMsisdn>());
      expect((state as CheckoutCollectMsisdn).problem, 'error.unexpected');
    });

    test('on confirming for a redirect rail, returns to ready_redirect', () {
      final CheckoutContext context = _context(
        session: _session(rails: [_orangeRedirect]),
      );
      final SupportedRail rail = railChoices([_orangeRedirect]).supported.first;
      final CheckoutScreenState confirming = CheckoutConfirming(
        context: context,
        rail: rail,
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        confirming,
        const CheckoutProblem('error.unexpected'),
      );
      expect(state, isA<CheckoutReadyRedirect>());
      expect((state as CheckoutReadyRedirect).problem, 'error.unexpected');
    });

    test('problem is dropped outside the four states that accept it', () {
      final CheckoutScreenState state = reduceCheckoutScreen(
        checkoutInitialState,
        const CheckoutProblem('error.unexpected'),
      );
      expect(state, isA<CheckoutLoading>());
    });
  });

  group('reduceCheckoutScreen — confirm, poll, redirect', () {
    test(
      'confirm_started moves collect_msisdn/ready_redirect to confirming',
      () {
        final CheckoutContext context = _context(
          session: _session(rails: [_mtnPush]),
        );
        final SupportedRail rail = railChoices([_mtnPush]).supported.first;
        final CheckoutScreenState collecting = CheckoutCollectMsisdn(
          context: context,
          rails: railChoices([_mtnPush]),
          rail: rail,
        );
        final CheckoutScreenState state = reduceCheckoutScreen(
          collecting,
          const CheckoutConfirmStarted(),
        );
        expect(state, isA<CheckoutConfirming>());
        expect((state as CheckoutConfirming).rail.code, 'mtn_momo');
      },
    );

    test('confirm_started is dropped from every other state', () {
      final CheckoutScreenState state = reduceCheckoutScreen(
        checkoutInitialState,
        const CheckoutConfirmStarted(),
      );
      expect(state, isA<CheckoutLoading>());
    });

    test('intent_updated on confirming with a non-terminal intent becomes waiting, carrying the rail', () {
      final CheckoutContext context = _context(
        session: _session(rails: [_mtnPush]),
      );
      final SupportedRail rail = railChoices([_mtnPush]).supported.first;
      final CheckoutScreenState confirming = CheckoutConfirming(
        context: context,
        rail: rail,
      );
      final PaymentIntent processing = _intent(
        status: PaymentIntentStatus.processing,
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        confirming,
        CheckoutIntentUpdated(processing),
      );
      expect(state, isA<CheckoutWaiting>());
      final CheckoutWaiting waiting = state as CheckoutWaiting;
      expect(waiting.rail?.code, 'mtn_momo');
      expect(waiting.context.intent.status, PaymentIntentStatus.processing);
    });

    test('MUTATION PROOF (poll terminal rule, reducer path): intent_updated '
        'with requires_payment_method and NO last_payment_error stays on '
        'waiting, never jumps to outcome', () {
      final CheckoutContext context = _context();
      final CheckoutScreenState waiting = CheckoutWaiting(context: context);
      final PaymentIntent stillUnconfirmed = _intent(
        status: PaymentIntentStatus.requiresPaymentMethod,
        lastPaymentError: null,
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        waiting,
        CheckoutIntentUpdated(stillUnconfirmed),
      );
      expect(state, isA<CheckoutWaiting>());
      expect(state, isNot(isA<CheckoutOutcome>()));
    });

    test(
      'intent_updated on waiting with a terminal intent becomes outcome',
      () {
        final CheckoutContext context = _context();
        final CheckoutScreenState waiting = CheckoutWaiting(context: context);
        final PaymentIntent succeeded = _intent(
          status: PaymentIntentStatus.succeeded,
        );
        final CheckoutScreenState state = reduceCheckoutScreen(
          waiting,
          CheckoutIntentUpdated(succeeded),
        );
        expect(state, isA<CheckoutOutcome>());
        expect((state as CheckoutOutcome).kind, OutcomeKind.succeeded);
      },
    );

    test('intent_updated is dropped outside confirming/waiting', () {
      final CheckoutScreenState state = reduceCheckoutScreen(
        checkoutInitialState,
        CheckoutIntentUpdated(_intent(status: PaymentIntentStatus.succeeded)),
      );
      expect(state, isA<CheckoutLoading>());
    });

    test('redirect_required from confirming becomes redirecting', () {
      final CheckoutContext context = _context(
        session: _session(rails: [_orangeRedirect]),
      );
      final SupportedRail rail = railChoices([_orangeRedirect]).supported.first;
      final CheckoutScreenState confirming = CheckoutConfirming(
        context: context,
        rail: rail,
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        confirming,
        const CheckoutRedirectRequired('https://orange.example/pay/abc'),
      );
      expect(state, isA<CheckoutRedirecting>());
      expect(
        (state as CheckoutRedirecting).url,
        'https://orange.example/pay/abc',
      );
    });

    test('redirect_required is dropped outside confirming', () {
      final CheckoutScreenState state = reduceCheckoutScreen(
        checkoutInitialState,
        const CheckoutRedirectRequired('https://orange.example/pay/abc'),
      );
      expect(state, isA<CheckoutLoading>());
    });
  });

  group('reduceCheckoutScreen — outcome, forward, session_refreshed', () {
    test(
      'session_refreshed on outcome replaces the session, keeps the kind',
      () {
        final CheckoutContext context = _context(
          session: _session(status: CheckoutSessionStatus.open),
        );
        final CheckoutScreenState outcome = CheckoutOutcome(
          context: context,
          kind: OutcomeKind.succeeded,
        );
        final CheckoutSession refreshed = _session(
          status: CheckoutSessionStatus.complete,
        );
        final CheckoutScreenState state = reduceCheckoutScreen(
          outcome,
          CheckoutSessionRefreshed(refreshed),
        );
        expect(state, isA<CheckoutOutcome>());
        final CheckoutOutcome next = state as CheckoutOutcome;
        expect(next.kind, OutcomeKind.succeeded);
        expect(next.context.session.status, CheckoutSessionStatus.complete);
      },
    );

    test('session_refreshed is dropped outside outcome — a late read cannot move a payer off what they are reading', () {
      final CheckoutContext context = _context();
      final CheckoutScreenState waiting = CheckoutWaiting(context: context);
      final CheckoutScreenState state = reduceCheckoutScreen(
        waiting,
        CheckoutSessionRefreshed(_session()),
      );
      expect(state, isA<CheckoutWaiting>());
    });

    test('forward from outcome becomes forwarding', () {
      final CheckoutContext context = _context();
      final CheckoutScreenState outcome = CheckoutOutcome(
        context: context,
        kind: OutcomeKind.succeeded,
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        outcome,
        const CheckoutForward('https://shop.example/thanks'),
      );
      expect(state, isA<CheckoutForwarding>());
      expect((state as CheckoutForwarding).url, 'https://shop.example/thanks');
    });

    test('forward is dropped outside outcome — a forward from waiting does nothing', () {
      final CheckoutContext context = _context();
      final CheckoutScreenState waiting = CheckoutWaiting(context: context);
      final CheckoutScreenState state = reduceCheckoutScreen(
        waiting,
        const CheckoutForward('https://shop.example/thanks'),
      );
      expect(state, isA<CheckoutWaiting>());
    });

    test('a second intent_updated after forwarding does nothing', () {
      final CheckoutContext context = _context();
      final CheckoutScreenState forwarding = CheckoutForwarding(
        context: context,
        kind: OutcomeKind.succeeded,
        url: 'https://shop.example/thanks',
      );
      final CheckoutScreenState state = reduceCheckoutScreen(
        forwarding,
        CheckoutIntentUpdated(_intent(status: PaymentIntentStatus.canceled)),
      );
      expect(state, same(forwarding));
    });
  });
}

/// A stand-in `CheckoutContext` for tests that only need `reduce` to see
/// *some* context on the incoming state, never inspect it.
abstract final class _TestContext {
  static final CheckoutContext dummy = _context();
}
