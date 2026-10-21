import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

const _piSecret = 'pi_123_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _csSecret = 'cs_123_secret_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _suffix = 'cccccccccccccccccccccccccccccccc';

/// Client secrets whose id no route may turn into a path segment — the
/// same list as `sdks/stripe-js/src/testing/unroutable-secrets.ts`.
///
/// `_tryParseClientSecret` is the **only** thing keeping `.` and `..` out of
/// a `/v1/browser` path: `Uri.encodeComponent` leaves dots alone, and
/// `Uri.parse` resolves a `..` segment — and a `%2E%2E` one — before any
/// request is built (measured on Dart 3.14, 2026-10-21; the merchant SDKs
/// had exactly this hole until #306). What closes it is the prefix rule: an
/// id must start with `pi_`/`cs_` and be longer than that prefix. Every entry
/// is refused by every route, whichever credential it expects.
const List<(String, String)> _unroutableSecrets = [
  ('an empty string', ''),
  ('a bare ..', '..'),
  ('a bare .', '.'),
  ('a bare %2e%2e', '%2e%2e'),
  ('a bare pi_', 'pi_'),
  ('a bare cs_', 'cs_'),
  ('.. as the id', '.._secret_$_suffix'),
  ('. as the id', '._secret_$_suffix'),
  ('%2e%2e as the id', '%2e%2e_secret_$_suffix'),
  ('%2E%2E as the id', '%2E%2E_secret_$_suffix'),
  ('an empty id', '_secret_$_suffix'),
  ("another object's prefix", 're_123_secret_$_suffix'),
  ('pi_ alone as the id', 'pi__secret_$_suffix'),
  ('cs_ alone as the id', 'cs__secret_x'),
  ('a pi_ id with an empty suffix', 'pi_x_secret_'),
  ('a cs_ id with an empty suffix', 'cs_x_secret_'),
];

/// Asserts [error] is the parser's refusal, naming [name] on failure.
void _expectRefused(String name, VpayError? error) {
  expect(error?.type, 'invalid_request_error', reason: name);
  expect(error?.param, 'clientSecret', reason: name);
}

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
      'rails': const <Object?>[],
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
        'payment_status': 'unpaid',
        'success_url': 'https://shop.example/thanks',
        'cancel_url': 'https://shop.example/cancel',
        'url': 'https://checkout.example/c/cs_123#$_csSecret',
        'expires_at': 1700086400,
        'created': 1700000000,
        'rails': const <Object?>[],
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

  group('BrowserClient.confirmPaymentIntent', () {
    test('POSTs form-encoded key, client_secret, payment_method_data[type] and the nested field', () async {
      Uri? seenUri;
      String? seenBody;
      String? seenContentType;
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async {
          seenUri = request.url;
          seenBody = request.body;
          seenContentType = request.headers['Content-Type'];
          return _json(_paymentIntentJson());
        }),
      );

      final result = await client.confirmPaymentIntent(
        _piSecret,
        railCode: 'mtn_momo',
        payerFields: const {'msisdn': '237690000000'},
      );

      expect(result.isError, isFalse);
      expect(seenUri!.path, '/v1/browser/payment_intents/pi_123/confirm');
      expect(seenContentType, 'application/x-www-form-urlencoded');

      // The DECISIVE assertion, on the literal wire bytes: the structural
      // brackets must reach the server unencoded — `payment_method_data[type]`,
      // never `payment_method_data%5Btype%5D`. `Uri.splitQueryString` below
      // decodes the whole key before an assertion ever sees it, so it
      // cannot tell the two apart; that gap was measured for real on
      // 2026-09-17 (`_bracketKey`'s own doc comment) — every assertion in
      // this test passed against a MockClient while the real server refused
      // every confirm with "A confirm needs the payment method to use,
      // sent as `payment_method_data[type]`.". This line is what closes it.
      expect(seenBody, contains('payment_method_data[type]=mtn_momo'));
      expect(
        seenBody,
        contains('payment_method_data[mtn_momo][msisdn]=237690000000'),
      );
      expect(seenBody, isNot(contains('%5B')));
      expect(seenBody, isNot(contains('%5D')));

      final Map<String, String> form = Uri.splitQueryString(seenBody!);
      expect(form['key'], 'pk_test_1');
      expect(form['client_secret'], _piSecret);
      expect(form['payment_method_data[type]'], 'mtn_momo');
      expect(form['payment_method_data[mtn_momo][msisdn]'], '237690000000');
    });

    test('sends no return_url field when none is given', () async {
      String? seenBody;
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async {
          seenBody = request.body;
          return _json(_paymentIntentJson());
        }),
      );

      await client.confirmPaymentIntent(_piSecret, railCode: 'orange_money');

      expect(
        Uri.splitQueryString(seenBody!).containsKey('return_url'),
        isFalse,
      );
    });

    test('includes return_url when given, for a redirect rail', () async {
      String? seenBody;
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async {
          seenBody = request.body;
          return _json(_paymentIntentJson());
        }),
      );

      await client.confirmPaymentIntent(
        _piSecret,
        railCode: 'orange_money',
        returnUrl: 'https://example.com/return?a=b',
      );

      final Map<String, String> form = Uri.splitQueryString(seenBody!);
      expect(form['return_url'], 'https://example.com/return?a=b');
    });

    test('refuses a malformed clientSecret without sending anything', () async {
      bool sent = false;
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async {
          sent = true;
          return _json(_paymentIntentJson());
        }),
      );

      final result = await client.confirmPaymentIntent(
        'not-a-secret',
        railCode: 'mtn_momo',
      );

      expect(result.isError, isTrue);
      expect(sent, isFalse);
    });

    test('maps the uniform 404 the same way the reads do', () async {
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient(
          (request) async => _json({
            'error': {'code': 'resource_missing', 'message': 'x'},
          }, status: 404),
        ),
      );

      final result = await client.confirmPaymentIntent(
        _piSecret,
        railCode: 'mtn_momo',
        payerFields: const {'msisdn': '237690000000'},
      );

      expect(result.isError, isTrue);
    });

    test('reports a refused connection as api_connection_error', () async {
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient(
          (request) async => throw const SocketExceptionStub(),
        ),
      );

      final result = await client.confirmPaymentIntent(
        _piSecret,
        railCode: 'mtn_momo',
        payerFields: const {'msisdn': '237690000000'},
      );

      expect(result.isError, isTrue);
      expect(result.error!.type, 'api_connection_error');
    });
  });

  group('BrowserClient — no route puts a dot segment in a path', () {
    /// A client whose every request is recorded, so "nothing was sent" is
    /// an assertion on a list rather than on a flag.
    (BrowserClient, List<Uri>) recordingClient() {
      final List<Uri> sent = <Uri>[];
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async {
          sent.add(request.url);
          return _json(_paymentIntentJson());
        }),
      );
      return (client, sent);
    }

    test('refuses a client secret whose id is empty or only dots on retrievePaymentIntent, sending nothing', () async {
      final (client, sent) = recordingClient();

      for (final (name, secret) in _unroutableSecrets) {
        final result = await client.retrievePaymentIntent(secret);
        expect(result.paymentIntent, isNull, reason: name);
        _expectRefused(name, result.error);
      }

      expect(sent, isEmpty);
    });

    test('refuses a client secret whose id is empty or only dots on confirmPaymentIntent, sending nothing', () async {
      final (client, sent) = recordingClient();

      for (final (name, secret) in _unroutableSecrets) {
        final result = await client.confirmPaymentIntent(
          secret,
          railCode: 'mtn_momo',
          payerFields: const {'msisdn': '237690000000'},
        );
        expect(result.paymentIntent, isNull, reason: name);
        _expectRefused(name, result.error);
      }

      expect(sent, isEmpty);
    });

    test('refuses a client secret whose id is empty or only dots on retrieveCheckoutSession, sending nothing', () async {
      final (client, sent) = recordingClient();

      for (final (name, secret) in _unroutableSecrets) {
        final result = await client.retrieveCheckoutSession(secret);
        expect(result.checkoutSession, isNull, reason: name);
        _expectRefused(name, result.error);
      }

      expect(sent, isEmpty);
    });

    test(
      'keeps an id that carries /../ inside one path segment on every route',
      () async {
        // The other half of the rule: an id that *passes* the prefix check may
        // still hold a `/`, and only `Uri.encodeComponent` keeps `pi_/../x`
        // from resolving to `payment_intents/x`. `request.url` is the `Uri`
        // the client built, after `Uri.parse` has normalised it.
        final (client, sent) = recordingClient();

        await client.retrievePaymentIntent('pi_/../x_secret_$_suffix');
        await client.confirmPaymentIntent(
          'pi_/../x_secret_$_suffix',
          railCode: 'mtn_momo',
        );
        await client.retrieveCheckoutSession('cs_/../x_secret_$_suffix');

        expect(sent.map((uri) => uri.path).toList(), <String>[
          '/v1/browser/payment_intents/pi_%2F..%2Fx',
          '/v1/browser/payment_intents/pi_%2F..%2Fx/confirm',
          '/v1/browser/checkout/sessions/cs_%2F..%2Fx',
        ]);
        expect(sent.map((uri) => uri.pathSegments).toList(), <List<String>>[
          <String>['v1', 'browser', 'payment_intents', 'pi_/../x'],
          <String>['v1', 'browser', 'payment_intents', 'pi_/../x', 'confirm'],
          <String>['v1', 'browser', 'checkout', 'sessions', 'cs_/../x'],
        ]);
      },
    );
  });
}

/// A stand-in for whatever `http.Client.get` throws on a refused connection
/// (a `SocketException` on native platforms) — the client must not care
/// about the exception's *type*, only that the call threw.
class SocketExceptionStub implements Exception {
  const SocketExceptionStub();

  @override
  String toString() => 'SocketExceptionStub(connection refused)';
}
