import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

const String _csSecret = 'cs_123_secret_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

/// Every [PaymentIntent] field the return read is shown carries a
/// `client_secret` (`models.dart`'s own type does not model the credential-
/// free wire shape); [ReturnPaymentIntent] is what this file exercises
/// instead, and it structurally has none.
ReturnPaymentIntent _returnIntent({
  PaymentIntentStatus status = PaymentIntentStatus.requiresPaymentMethod,
  LastPaymentError? lastPaymentError,
}) => ReturnPaymentIntent(status: status, lastPaymentError: lastPaymentError);

PaymentIntent _fullIntentForSession({
  PaymentIntentStatus status = PaymentIntentStatus.requiresPaymentMethod,
}) => PaymentIntent(
  id: 'pi_123',
  amount: 5000,
  currency: 'xaf',
  status: status,
  paymentMethodTypes: const ['mtn_momo'],
  lastPaymentError: null,
  metadata: const {},
  description: null,
  created: 1700000000,
  livemode: false,
  clientSecret: 'pi_123_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
);

CheckoutSession _session({
  CheckoutSessionStatus status = CheckoutSessionStatus.open,
  CheckoutSessionPaymentStatus paymentStatus =
      CheckoutSessionPaymentStatus.unpaid,
}) => CheckoutSession(
  id: 'cs_123',
  livemode: false,
  paymentIntent: _fullIntentForSession(),
  uiMode: CheckoutUiMode.hosted,
  status: status,
  paymentStatus: paymentStatus,
  successUrl: null,
  cancelUrl: null,
  url: null,
  expiresAt: 1700086400,
  created: 1700000000,
  clientSecret: _csSecret,
  rails: const [],
);

ReturnContext _context({
  CheckoutSession? session,
  ReturnPaymentIntent? intent,
}) => ReturnContext(
  session: session ?? _session(),
  intent: intent ?? _returnIntent(),
);

void main() {
  group('ReturnPaymentIntent — structurally credential-free (D6)', () {
    test(
      'carries only status and lastPaymentError, nothing a confirm needs',
      () {
        // The type's own shape is the proof: constructing one and reading it
        // back demonstrates there is no client_secret field to reference at
        // all, not merely one that happens to be unused.
        final ReturnPaymentIntent intent = _returnIntent(
          status: PaymentIntentStatus.succeeded,
        );
        expect(intent.status, PaymentIntentStatus.succeeded);
        expect(intent.lastPaymentError, isNull);
      },
    );
  });

  group('stateForReturn', () {
    test('a complete session is outcome:succeeded', () {
      final ReturnContext context = _context(
        session: _session(status: CheckoutSessionStatus.complete),
      );
      final ReturnScreenState state = stateForReturn(context);
      expect(state, isA<ReturnOutcome>());
      expect((state as ReturnOutcome).kind, OutcomeKind.succeeded);
    });

    test('an expired session with payment_status != failed is the neutral expired screen', () {
      final ReturnContext context = _context(
        session: _session(status: CheckoutSessionStatus.expired),
      );
      expect(stateForReturn(context), isA<ReturnExpired>());
    });

    test('an expired session with payment_status == failed reports the intent\'s outcome', () {
      final ReturnPaymentIntent failed = _returnIntent(
        lastPaymentError: const LastPaymentError(
          code: 'provider_error',
          message: 'The rail could not process this payment.',
        ),
      );
      final ReturnContext context = _context(
        session: _session(
          status: CheckoutSessionStatus.expired,
          paymentStatus: CheckoutSessionPaymentStatus.failed,
        ),
        intent: failed,
      );
      final ReturnScreenState state = stateForReturn(context);
      expect(state, isA<ReturnOutcome>());
      final ReturnOutcome outcome = state as ReturnOutcome;
      expect(outcome.kind, OutcomeKind.failed);
      expect(outcome.failure, 'provider_error');
      expect(outcome.reason, 'The rail could not process this payment.');
    });

    test('an open session with a still-moving intent is "polling", carrying the previous notice', () {
      final ReturnContext context = _context();
      final ReturnScreenState state = stateForReturn(
        context,
        previousNotice: 'error.network',
      );
      expect(state, isA<ReturnPolling>());
      expect((state as ReturnPolling).notice, 'error.network');
    });

    test(
      'MUTATION PROOF (poll terminal rule, return machine): '
      'requires_payment_method with no last_payment_error is NOT terminal',
      () {
        final ReturnContext context = _context(
          intent: _returnIntent(
            status: PaymentIntentStatus.requiresPaymentMethod,
            lastPaymentError: null,
          ),
        );
        final ReturnScreenState state = stateForReturn(context);
        expect(state, isNot(isA<ReturnOutcome>()));
        expect(state, isA<ReturnPolling>());
      },
    );

    test(
      'an open session whose intent already succeeded is outcome:succeeded',
      () {
        final ReturnContext context = _context(
          intent: _returnIntent(status: PaymentIntentStatus.succeeded),
        );
        final ReturnScreenState state = stateForReturn(context);
        expect(state, isA<ReturnOutcome>());
        expect((state as ReturnOutcome).kind, OutcomeKind.succeeded);
      },
    );

    test(
      'an open session whose intent already canceled is outcome:canceled',
      () {
        final ReturnContext context = _context(
          intent: _returnIntent(status: PaymentIntentStatus.canceled),
        );
        final ReturnScreenState state = stateForReturn(context);
        expect(state, isA<ReturnOutcome>());
        expect((state as ReturnOutcome).kind, OutcomeKind.canceled);
      },
    );
  });

  group('reduceReturn', () {
    test('read from loading enters stateForReturn\'s answer', () {
      final ReturnContext context = _context();
      final ReturnScreenState state = reduceReturn(
        returnInitialState,
        ReturnRead(context),
      );
      expect(state, isA<ReturnPolling>());
    });

    test('read while polling carries the previous notice forward', () {
      final ReturnContext context = _context();
      final ReturnScreenState polling = ReturnPolling(
        context: context,
        notice: 'error.network',
      );
      final ReturnScreenState state = reduceReturn(
        polling,
        ReturnRead(context),
      );
      expect(state, isA<ReturnPolling>());
      expect((state as ReturnPolling).notice, 'error.network');
    });

    test('read is dropped once an outcome or forwarding is on screen', () {
      final ReturnContext context = _context();
      final ReturnScreenState outcome = ReturnOutcome(
        context: context,
        kind: OutcomeKind.succeeded,
      );
      final ReturnScreenState state = reduceReturn(
        outcome,
        ReturnRead(context),
      );
      expect(state, same(outcome));
    });

    test('read_failed from loading becomes a typed error', () {
      const VpayError error = VpayError(
        type: 'api_error',
        code: 'unexpected_response',
      );
      final ReturnScreenState state = reduceReturn(
        returnInitialState,
        const ReturnReadFailed(error),
      );
      expect(state, isA<ReturnLoadError>());
      expect((state as ReturnLoadError).error, same(error));
    });

    test('poll_failed on polling becomes the notice', () {
      final ReturnContext context = _context();
      final ReturnScreenState polling = ReturnPolling(context: context);
      final ReturnScreenState state = reduceReturn(
        polling,
        const ReturnPollFailed('error.network'),
      );
      expect(state, isA<ReturnPolling>());
      expect((state as ReturnPolling).notice, 'error.network');
    });

    test('poll_failed is dropped outside polling', () {
      final ReturnScreenState state = reduceReturn(
        returnInitialState,
        const ReturnPollFailed('error.network'),
      );
      expect(state, isA<ReturnLoading>());
    });

    test('forward from outcome becomes forwarding', () {
      final ReturnContext context = _context();
      final ReturnScreenState outcome = ReturnOutcome(
        context: context,
        kind: OutcomeKind.succeeded,
      );
      final ReturnScreenState state = reduceReturn(
        outcome,
        const ReturnForward('https://shop.example/thanks'),
      );
      expect(state, isA<ReturnForwarding>());
      expect((state as ReturnForwarding).url, 'https://shop.example/thanks');
    });

    test('forward is dropped outside outcome', () {
      final ReturnContext context = _context();
      final ReturnScreenState polling = ReturnPolling(context: context);
      final ReturnScreenState state = reduceReturn(
        polling,
        const ReturnForward('https://shop.example/thanks'),
      );
      expect(state, isA<ReturnPolling>());
    });
  });
}
