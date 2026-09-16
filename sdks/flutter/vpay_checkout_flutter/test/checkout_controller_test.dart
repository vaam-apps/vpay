import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

const _piSecret = 'pi_123_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _csSecret = 'cs_123_secret_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

/// A clock a test fully controls: `delay` advances the virtual time instead
/// of waiting for real time, so a three-minute polling budget resolves in
/// microseconds of wall-clock test time.
class FakeClock extends VpayClock {
  FakeClock(this._now);

  DateTime _now;

  @override
  DateTime now() => _now;

  @override
  Future<void> delay(Duration duration) async {
    _now = _now.add(duration);
  }
}

http.Response _json(Object body, {int status = 200}) =>
    http.Response(jsonEncode(body), status);

Map<String, Object?> _paymentIntentJson(String status) => {
  'id': 'pi_123',
  'object': 'payment_intent',
  'amount': 5000,
  'currency': 'xaf',
  'status': status,
  'payment_method_types': ['mtn_momo'],
  'next_action': null,
  'last_payment_error': null,
  'metadata': <String, Object?>{},
  'description': null,
  'created': 1700000000,
  'livemode': false,
  'client_secret': _piSecret,
};

Map<String, Object?> _sessionJson({
  String uiMode = 'hosted',
  String? successUrl,
  String? cancelUrl,
}) => {
  'id': 'cs_123',
  'object': 'checkout.session',
  'livemode': false,
  'payment_intent': _paymentIntentJson('requires_payment_method'),
  'ui_mode': uiMode,
  'status': 'open',
  'payment_status': 'unpaid',
  'success_url': successUrl,
  'cancel_url': cancelUrl,
  'return_url': null,
  'url': 'https://checkout.example/c/cs_123#$_csSecret',
  'expires_at': 1700086400,
  'created': 1700000000,
  'rails': const <Object?>[],
  // No `client_secret` key: the real session read never sends the
  // session's own secret back — see `CheckoutSession.clientSecret`'s doc
  // comment in `lib/src/models.dart`.
};

CheckoutPreflightReady _readyFixture({String? successUrl, String? cancelUrl}) =>
    CheckoutPreflightReady(
      sessionId: 'cs_123',
      paymentIntentId: 'pi_123',
      intentClientSecret: _piSecret,
      successStopUrl: StopUrlSpec.fromConfigured(
        successUrl,
        sessionId: 'cs_123',
      ),
      cancelStopUrl: StopUrlSpec.fromConfigured(cancelUrl, sessionId: 'cs_123'),
    );

CheckoutController _controllerAlwaysAnswering(
  String status, {
  FakeClock? clock,
}) => CheckoutController(
  client: BrowserClient(
    baseUrl: 'https://api.example',
    publishableKey: 'pk_test_1',
    httpClient: MockClient(
      (request) async => _json(_paymentIntentJson(status)),
    ),
  ),
  clock: clock ?? FakeClock(DateTime(2026, 9, 13)),
);

void main() {
  group('D1 — the outcome is never read off a URL', () {
    test('reaching success_url with an intent that is processing yields pending, not succeeded', () async {
      final ready = _readyFixture(successUrl: 'https://shop.example/thanks');
      final controller = _controllerAlwaysAnswering('processing');

      final result = await controller.resolveAfterStopUrlReached(
        ready,
        timeout: const Duration(seconds: 10),
        interval: const Duration(seconds: 2),
      );

      expect(result, isA<VpayCheckoutPending>());
      expect(result, isNot(isA<VpayCheckoutSucceeded>()));
    });

    test('reaching success_url with an intent that has actually succeeded reports succeeded', () async {
      final ready = _readyFixture(successUrl: 'https://shop.example/thanks');
      final controller = _controllerAlwaysAnswering('succeeded');

      final result = await controller.resolveAfterStopUrlReached(ready);

      expect(result, isA<VpayCheckoutSucceeded>());
    });

    test('resolveAfterStopUrlReached takes no URL argument at all — the signature itself is the proof', () {
      // `dart:mirrors` is unavailable on every platform this package
      // targets, so this is asserted the same way `no_logging_test.dart`
      // asserts its own property: by reading the source.
      expect(
        RegExp(
          r'Future<VpayCheckoutResult> resolveAfterStopUrlReached\(\s*CheckoutPreflightReady ready,',
        ).hasMatch(File('lib/src/checkout_controller.dart').readAsStringSync()),
        isTrue,
      );
    });
  });

  group('D4 — dismissal polls before it reports', () {
    test(
      'a dismissal with a succeeded intent reports succeeded, not canceled',
      () async {
        final ready = _readyFixture();
        final controller = _controllerAlwaysAnswering('succeeded');

        final result = await controller.resolveAfterDismissal(ready);

        expect(result, isA<VpayCheckoutSucceeded>());
        expect(result, isNot(isA<VpayCheckoutCanceled>()));
      },
    );

    test('a dismissal with an intent still in flight after the short poll reports pending, not canceled', () async {
      final ready = _readyFixture();
      final controller = _controllerAlwaysAnswering('processing');

      final result = await controller.resolveAfterDismissal(
        ready,
        timeout: const Duration(seconds: 5),
        interval: const Duration(seconds: 1),
      );

      expect(result, isA<VpayCheckoutPending>());
      expect(result, isNot(isA<VpayCheckoutCanceled>()));
    });

    test(
      'a dismissal with a genuinely canceled intent reports canceled',
      () async {
        final ready = _readyFixture();
        final controller = _controllerAlwaysAnswering('canceled');

        final result = await controller.resolveAfterDismissal(ready);

        expect(result, isA<VpayCheckoutCanceled>());
      },
    );
  });

  group('resolve — the answer must be about the intent that was asked for', () {
    test(
      'a succeeded intent with a different id is unresolved, never succeeded',
      () async {
        final ready = _readyFixture();
        final controller = CheckoutController(
          client: BrowserClient(
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: MockClient(
              (request) async => _json({
                ..._paymentIntentJson('succeeded'),
                'id': 'pi_somebody_elses',
              }),
            ),
          ),
        );

        final result = await controller.resolveAfterStopUrlReached(ready);

        expect(result, isNot(isA<VpayCheckoutSucceeded>()));
        expect(result, isA<VpayCheckoutUnresolved>());
        expect(
          (result as VpayCheckoutUnresolved).error.code,
          VpayClientErrorCodes.unexpectedResponse,
        );
        expect(result.paymentIntentId, 'pi_123');
      },
    );
  });

  group('resolve — failure mapping', () {
    test('requires_payment_method with a last_payment_error reports failed with the rail code', () async {
      final ready = _readyFixture();
      final controller = CheckoutController(
        client: BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient(
            (request) async => _json({
              ..._paymentIntentJson('requires_payment_method'),
              'last_payment_error': {
                'code': 'insufficient_funds',
                'message': 'Not enough funds.',
              },
            }),
          ),
        ),
      );

      final result = await controller.resolveAfterStopUrlReached(ready);

      expect(result, isA<VpayCheckoutFailed>());
      expect((result as VpayCheckoutFailed).code, 'insufficient_funds');
      expect(result.providerMessage, 'Not enough funds.');
    });

    test(
      'a poll that itself errors resolves unresolved with the typed error',
      () async {
        final ready = _readyFixture();
        final controller = CheckoutController(
          client: BrowserClient(
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: MockClient(
              (request) async => _json({
                'error': {
                  'type': 'invalid_request_error',
                  'code': 'resource_missing',
                  'message': 'No such payment intent: pi_123',
                },
              }, status: 404),
            ),
          ),
        );

        final result = await controller.resolveAfterStopUrlReached(ready);

        expect(result, isA<VpayCheckoutUnresolved>());
        expect(
          (result as VpayCheckoutUnresolved).error.code,
          'resource_missing',
        );
      },
    );
  });

  group('StopUrlSpec — substitution and matching (D2)', () {
    test(
      '{CHECKOUT_SESSION_ID} is substituted before a URL becomes a stop rule',
      () {
        final spec = StopUrlSpec.fromConfigured(
          'https://shop.example/return/{CHECKOUT_SESSION_ID}',
          sessionId: 'cs_123',
        )!;

        expect(spec.path, '/return/cs_123');
        expect(
          spec.matches(Uri.parse('https://shop.example/return/cs_123')),
          isTrue,
        );
        expect(
          spec.matches(
            Uri.parse('https://shop.example/return/{CHECKOUT_SESSION_ID}'),
          ),
          isFalse,
        );
      },
    );

    test(
      'matching is scheme+host+port+path — query and fragment are ignored',
      () {
        final spec = StopUrlSpec.fromConfigured(
          'https://shop.example/thanks',
          sessionId: 'cs_123',
        )!;

        expect(
          spec.matches(
            Uri.parse('https://shop.example/thanks?utm_source=vpay&x=1#top'),
          ),
          isTrue,
        );
      },
    );

    test('a different path does not match', () {
      final spec = StopUrlSpec.fromConfigured(
        'https://shop.example/thanks',
        sessionId: 'cs_123',
      )!;
      expect(spec.matches(Uri.parse('https://shop.example/thanks/')), isFalse);
      expect(spec.matches(Uri.parse('https://shop.example/other')), isFalse);
    });

    test('a different host or scheme does not match', () {
      final spec = StopUrlSpec.fromConfigured(
        'https://shop.example/thanks',
        sessionId: 'cs_123',
      )!;
      expect(spec.matches(Uri.parse('https://evil.example/thanks')), isFalse);
      expect(spec.matches(Uri.parse('http://shop.example/thanks')), isFalse);
    });

    test('an implicit default port matches an explicit one and vice versa', () {
      final implicit = StopUrlSpec.fromConfigured(
        'https://shop.example/thanks',
        sessionId: 'cs_123',
      )!;
      expect(
        implicit.matches(Uri.parse('https://shop.example:443/thanks')),
        isTrue,
      );

      final explicit = StopUrlSpec.fromConfigured(
        'https://shop.example:443/thanks',
        sessionId: 'cs_123',
      )!;
      expect(
        explicit.matches(Uri.parse('https://shop.example/thanks')),
        isTrue,
      );
    });

    test('a null configured URL yields no stop rule at all', () {
      expect(StopUrlSpec.fromConfigured(null, sessionId: 'cs_123'), isNull);
    });
  });

  group('CheckoutController.preflight (D2)', () {
    test('an embedded session is refused here, before any window opens, with a typed error', () async {
      final controller = CheckoutController(
        client: BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient(
            (request) async => _json(_sessionJson(uiMode: 'embedded')),
          ),
        ),
      );

      final outcome = await controller.preflight(_csSecret);

      expect(outcome, isA<CheckoutPreflightFailure>());
      expect(
        (outcome as CheckoutPreflightFailure).error.code,
        VpayClientErrorCodes.embeddedSessionNotSupported,
      );
    });

    test('a hosted session buys the intent id, its client_secret, and derives the stop URLs from the session', () async {
      final controller = CheckoutController(
        client: BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient(
            (request) async => _json(
              _sessionJson(
                successUrl:
                    'https://shop.example/thanks?cs={CHECKOUT_SESSION_ID}',
                cancelUrl: 'https://shop.example/cancel',
              ),
            ),
          ),
        ),
      );

      final outcome = await controller.preflight(_csSecret);

      expect(outcome, isA<CheckoutPreflightSuccess>());
      final ready = (outcome as CheckoutPreflightSuccess).ready;
      expect(ready.paymentIntentId, 'pi_123');
      expect(ready.intentClientSecret, _piSecret);
      expect(ready.successStopUrl!.path, '/thanks');
      expect(ready.cancelStopUrl!.path, '/cancel');
    });

    test('a bad or expired link — the uniform 404 — fails the pre-flight with the same typed error', () async {
      final controller = CheckoutController(
        client: BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient(
            (request) async => _json({
              'error': {
                'type': 'invalid_request_error',
                'code': 'resource_missing',
                'message': 'No such checkout session: cs_123',
              },
            }, status: 404),
          ),
        ),
      );

      final outcome = await controller.preflight(_csSecret);

      expect(outcome, isA<CheckoutPreflightFailure>());
      expect(
        (outcome as CheckoutPreflightFailure).error.code,
        'resource_missing',
      );
    });
  });
}
