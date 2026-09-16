import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

const _piSecretSuffix = 'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1';
const _csSecretSuffix = 'b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2';

Map<String, Object?> _samplePaymentIntentJson({
  String status = 'requires_payment_method',
  Object? lastPaymentError,
}) => {
  'id': 'pi_123',
  'object': 'payment_intent',
  'amount': 5000,
  'currency': 'xaf',
  'status': status,
  'payment_method_types': ['mtn_momo'],
  'next_action': null,
  'last_payment_error': lastPaymentError,
  'metadata': <String, Object?>{},
  'description': null,
  'created': 1700000000,
  'livemode': false,
  'client_secret': 'pi_123_secret_$_piSecretSuffix',
};

Map<String, Object?> _sampleCheckoutSessionJson({
  String status = 'open',
  String uiMode = 'hosted',
  String? successUrl,
  String? cancelUrl,
}) => {
  'id': 'cs_123',
  'object': 'checkout.session',
  'livemode': false,
  'payment_intent': _samplePaymentIntentJson(),
  'ui_mode': uiMode,
  'status': status,
  'payment_status': 'unpaid',
  'success_url': successUrl,
  'cancel_url': cancelUrl,
  'return_url': null,
  'url':
      'https://checkout.vpay.example/c/cs_123?key=pk_test_1#cs_123_secret_$_csSecretSuffix',
  'expires_at': 1700086400,
  'created': 1700000000,
  // Deliberately NOT a `client_secret` key: the real
  // `GET /v1/browser/checkout/sessions/{id}` never sends the session's own
  // secret back (`CheckoutSession.clientSecret`'s doc comment) — this
  // fixture used to carry one and every test below decoded it, which is
  // exactly the gap only a real server catches, not a `MockClient`.
};

const _sampleSessionClientSecret = 'cs_123_secret_$_csSecretSuffix';

void main() {
  group('PaymentIntent', () {
    test('renders all thirteen keys of PaymentIntentWithSecret', () {
      final intent = PaymentIntent.fromJson(_samplePaymentIntentJson());
      expect(intent.id, 'pi_123');
      expect(intent.amount, 5000);
      expect(intent.currency, 'xaf');
      expect(intent.status, PaymentIntentStatus.requiresPaymentMethod);
      expect(intent.paymentMethodTypes, ['mtn_momo']);
      expect(intent.lastPaymentError, isNull);
      expect(intent.metadata, isEmpty);
      expect(intent.description, isNull);
      expect(intent.created, 1700000000);
      expect(intent.livemode, isFalse);
      expect(intent.clientSecret, 'pi_123_secret_$_piSecretSuffix');
    });

    test('requires_payment_method with no last_payment_error has not stopped moving', () {
      final intent = PaymentIntent.fromJson(_samplePaymentIntentJson());
      expect(intent.hasStoppedMoving, isFalse);
    });

    test(
      'requires_payment_method WITH a last_payment_error has stopped moving',
      () {
        final intent = PaymentIntent.fromJson(
          _samplePaymentIntentJson(
            lastPaymentError: {
              'code': 'insufficient_funds',
              'message': 'Not enough funds.',
            },
          ),
        );
        expect(intent.hasStoppedMoving, isTrue);
        expect(intent.lastPaymentError!.code, 'insufficient_funds');
      },
    );

    test('succeeded has stopped moving', () {
      final intent = PaymentIntent.fromJson(
        _samplePaymentIntentJson(status: 'succeeded'),
      );
      expect(intent.hasStoppedMoving, isTrue);
    });

    test('canceled has stopped moving', () {
      final intent = PaymentIntent.fromJson(
        _samplePaymentIntentJson(status: 'canceled'),
      );
      expect(intent.hasStoppedMoving, isTrue);
    });

    test('processing has not stopped moving', () {
      final intent = PaymentIntent.fromJson(
        _samplePaymentIntentJson(status: 'processing'),
      );
      expect(intent.hasStoppedMoving, isFalse);
    });

    test('requires_action has not stopped moving', () {
      final intent = PaymentIntent.fromJson(
        _samplePaymentIntentJson(status: 'requires_action'),
      );
      expect(intent.hasStoppedMoving, isFalse);
    });

    test('a PaymentIntent debug output never contains its client_secret', () {
      final intent = PaymentIntent.fromJson(_samplePaymentIntentJson());
      expect(intent.toString(), isNot(contains(_piSecretSuffix)));
      expect(
        intent.toString(),
        contains('[${intent.clientSecret.length} chars redacted]'),
      );
    });
  });

  group('CheckoutSession', () {
    test('reads a hosted session\'s url and both forwarding URLs', () {
      final session = CheckoutSession.fromJson(
        _sampleCheckoutSessionJson(
          successUrl: 'https://shop.example/thanks',
          cancelUrl: 'https://shop.example/cancel',
        ),
        clientSecret: _sampleSessionClientSecret,
      );
      expect(session.id, 'cs_123');
      expect(session.uiMode, CheckoutUiMode.hosted);
      expect(session.status, CheckoutSessionStatus.open);
      expect(session.successUrl, 'https://shop.example/thanks');
      expect(session.cancelUrl, 'https://shop.example/cancel');
      expect(session.url, startsWith('https://checkout.vpay.example/c/cs_123'));
    });

    test('expands payment_intent into the whole intent, with the intent\'s own client_secret typed', () {
      final session = CheckoutSession.fromJson(
        _sampleCheckoutSessionJson(),
        clientSecret: _sampleSessionClientSecret,
      );
      expect(session.paymentIntent, isA<PaymentIntent>());
      expect(session.paymentIntent.clientSecret, contains('pi_123_secret_'));
    });

    test('keeps the expanded intent\'s secret out of the client\'s diagnostics, exactly as it keeps the session\'s', () {
      final session = CheckoutSession.fromJson(
        _sampleCheckoutSessionJson(),
        clientSecret: _sampleSessionClientSecret,
      );
      final rendered = session.toString();
      expect(rendered, isNot(contains(_csSecretSuffix)));
      expect(rendered, isNot(contains(_piSecretSuffix)));
      expect(
        rendered,
        contains('[${session.clientSecret.length} chars redacted]'),
      );
    });

    test('redacts the hosted url, which carries the session secret in its fragment', () {
      final session = CheckoutSession.fromJson(
        _sampleCheckoutSessionJson(),
        clientSecret: _sampleSessionClientSecret,
      );
      expect(session.toString(), isNot(contains('checkout.vpay.example')));
    });
  });
}
