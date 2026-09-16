import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

const _piSecret = 'pi_123_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _csSecret = 'cs_123_secret_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

http.Response _json(Object body, {int status = 200}) =>
    http.Response(jsonEncode(body), status);

Map<String, Object?> _paymentIntentJson() => {
  'id': 'pi_123',
  'object': 'payment_intent',
  'amount': 5000,
  'currency': 'xaf',
  'status': 'processing',
  'payment_method_types': ['mtn_momo'],
  'next_action': null,
  'last_payment_error': null,
  'metadata': <String, Object?>{},
  'description': null,
  'created': 1700000000,
  'livemode': false,
  'client_secret': _piSecret,
};

void main() {
  group('BrowserClient — construction', () {
    test('refuses a non-https base URL by default', () {
      expect(
        () => BrowserClient(
          baseUrl: 'http://api.example',
          publishableKey: 'pk_test_1',
        ),
        throwsArgumentError,
      );
    });

    test('accepts a non-https base URL when allowInsecureBaseUrl is true', () {
      expect(
        () => BrowserClient(
          baseUrl: 'http://localhost:4000',
          publishableKey: 'pk_test_1',
          allowInsecureBaseUrl: true,
        ),
        returnsNormally,
      );
    });

    test(
      'strips a run of trailing slashes so the path cannot become //v1/browser',
      () {
        final client = BrowserClient(
          baseUrl: 'https://api.example///',
          publishableKey: 'pk_test_1',
        );
        expect(client.baseUrl, 'https://api.example');
      },
    );
  });

  group('BrowserClient.retrievePaymentIntent', () {
    test(
      'GETs the browser route with key and client_secret in the query string',
      () async {
        Uri? seenUri;
        final client = BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient((request) async {
            seenUri = request.url;
            return _json(_paymentIntentJson());
          }),
        );

        final result = await client.retrievePaymentIntent(_piSecret);

        expect(result.isError, isFalse);
        expect(seenUri, isNotNull);
        expect(seenUri!.path, '/v1/browser/payment_intents/pi_123');
        expect(seenUri!.queryParameters['key'], 'pk_test_1');
        expect(seenUri!.queryParameters['client_secret'], _piSecret);
      },
    );

    test(
      'maps the uniform 404 every browser credential failure renders',
      () async {
        final client = BrowserClient(
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
        );

        final result = await client.retrievePaymentIntent(_piSecret);

        expect(result.isError, isTrue);
        expect(result.error!.type, 'invalid_request_error');
        expect(result.error!.code, 'resource_missing');
      },
    );

    test('reports a non-envelope failure body as unexpected_response, without quoting it', () async {
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient(
          (request) async => http.Response('<html>bad gateway</html>', 502),
        ),
      );

      final result = await client.retrievePaymentIntent(_piSecret);

      expect(result.isError, isTrue);
      expect(result.error!.code, VpayClientErrorCodes.unexpectedResponse);
      expect(result.error!.message, isNot(contains('<html>')));
    });

    test(
      'reports a 200 that is not a payment intent as unexpected_response',
      () async {
        final client = BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient(
            (request) async => _json({'object': 'balance'}),
          ),
        );

        final result = await client.retrievePaymentIntent(_piSecret);

        expect(result.isError, isTrue);
        expect(result.error!.code, VpayClientErrorCodes.unexpectedResponse);
      },
    );

    test(
      'reports a refused connection as api_connection_error and never rejects',
      () async {
        final client = BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient(
            (request) async => throw const SocketExceptionStub(),
          ),
        );

        final result = await client.retrievePaymentIntent(_piSecret);

        expect(result.isError, isTrue);
        expect(result.error!.type, 'api_connection_error');
      },
    );

    test('refuses a malformed clientSecret without polling at all', () async {
      var called = false;
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async {
          called = true;
          return _json(_paymentIntentJson());
        }),
      );

      final result = await client.retrievePaymentIntent('not-a-secret');

      expect(result.isError, isTrue);
      expect(called, isFalse);
    });

    test('keeps the client secret out of every error it builds', () async {
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient(
          (request) async => http.Response('not json', 502),
        ),
      );

      final result = await client.retrievePaymentIntent(_piSecret);

      expect(
        jsonEncode({'error': result.error.toString()}),
        isNot(contains(_piSecret)),
      );
    });
  });

  group('BrowserClient.retrieveCheckoutSession', () {
    Map<String, Object?> sessionJson() => {
      'id': 'cs_123',
      'object': 'checkout.session',
      'livemode': false,
      'payment_intent': _paymentIntentJson(),
      'ui_mode': 'hosted',
      'status': 'open',
      'payment_status': 'unpaid',
      'success_url': 'https://shop.example/thanks',
      'cancel_url': 'https://shop.example/cancel',
      'return_url': null,
      'url': 'https://checkout.example/c/cs_123#$_csSecret',
      'expires_at': 1700086400,
      'created': 1700000000,
      // No `client_secret` key: the real session read never sends the
      // session's own secret back — see `CheckoutSession.clientSecret`'s
      // doc comment in `lib/src/models.dart`.
    };

    test('GETs the browser checkout-session route with key and client_secret in the query string', () async {
      Uri? seenUri;
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async {
          seenUri = request.url;
          return _json(sessionJson());
        }),
      );

      final result = await client.retrieveCheckoutSession(_csSecret);

      expect(result.isError, isFalse);
      expect(seenUri!.path, '/v1/browser/checkout/sessions/cs_123');
      expect(seenUri!.queryParameters['client_secret'], _csSecret);
    });

    test('refuses a payment-intent secret where a checkout-session secret belongs, without sending anything', () async {
      var called = false;
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async {
          called = true;
          return _json(sessionJson());
        }),
      );

      final result = await client.retrieveCheckoutSession(_piSecret);

      expect(result.isError, isTrue);
      expect(called, isFalse);
    });

    test(
      'maps the uniform 404 every checkout-session credential failure renders',
      () async {
        final client = BrowserClient(
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
        );

        final result = await client.retrieveCheckoutSession(_csSecret);

        expect(result.isError, isTrue);
        expect(result.error!.code, 'resource_missing');
      },
    );
  });

  group(
    'BrowserClient — a body that says it is the right object but is not',
    () {
      Map<String, Object?> sessionJson() => {
        'id': 'cs_123',
        'object': 'checkout.session',
        'livemode': false,
        'payment_intent': _paymentIntentJson(),
        'ui_mode': 'hosted',
        'status': 'open',
        'success_url': 'https://shop.example/thanks',
        'cancel_url': 'https://shop.example/cancel',
        'url': 'https://checkout.example/c/cs_123#$_csSecret',
        'expires_at': 1700086400,
        'created': 1700000000,
        // No `client_secret` key: the real session read never sends the
        // session's own secret back — see
        // `CheckoutSession.clientSecret`'s doc comment in
        // `lib/src/models.dart`.
      };

      test('a payment intent missing a required key answers unexpected_response instead of throwing', () async {
        final broken = Map<String, Object?>.from(_paymentIntentJson())
          ..remove('amount');
        final client = BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient((request) async => _json(broken)),
        );

        final result = await client.retrievePaymentIntent(_piSecret);

        expect(result.isError, isTrue);
        expect(result.error!.code, VpayClientErrorCodes.unexpectedResponse);
        expect(result.paymentIntent, isNull);
      });

      test('a payment intent with a wrongly-typed key never reaches an error message', () async {
        final broken = Map<String, Object?>.from(_paymentIntentJson())
          ..['client_secret'] = 12345;
        final client = BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient((request) async => _json(broken)),
        );

        final result = await client.retrievePaymentIntent(_piSecret);

        expect(result.isError, isTrue);
        // A Dart cast error's own message quotes the offending value; this
        // one is a `client_secret` field (D6), so nothing from it is carried.
        expect(result.error!.message, isNot(contains('12345')));
        expect(result.error!.message, isNot(contains('client_secret')));
      });

      test('a checkout session missing a required key answers unexpected_response instead of throwing', () async {
        final broken = Map<String, Object?>.from(sessionJson())
          ..remove('expires_at');
        final client = BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient((request) async => _json(broken)),
        );

        final result = await client.retrieveCheckoutSession(_csSecret);

        expect(result.isError, isTrue);
        expect(result.error!.code, VpayClientErrorCodes.unexpectedResponse);
        expect(result.checkoutSession, isNull);
      });

      test('a session whose expanded payment_intent is broken fails the session read, not the poll later', () async {
        final brokenIntent = Map<String, Object?>.from(_paymentIntentJson())
          ..remove('client_secret');
        final broken = Map<String, Object?>.from(sessionJson())
          ..['payment_intent'] = brokenIntent;
        final client = BrowserClient(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient((request) async => _json(broken)),
        );

        final result = await client.retrieveCheckoutSession(_csSecret);

        expect(result.isError, isTrue);
        expect(result.error!.code, VpayClientErrorCodes.unexpectedResponse);
      });
    },
  );
}

/// A stand-in for whatever `http.Client.get` throws on a refused connection
/// (a `SocketException` on native platforms) — the client must not care
/// about the exception's *type*, only that the call threw.
class SocketExceptionStub implements Exception {
  const SocketExceptionStub();

  @override
  String toString() => 'SocketExceptionStub(connection refused)';
}
