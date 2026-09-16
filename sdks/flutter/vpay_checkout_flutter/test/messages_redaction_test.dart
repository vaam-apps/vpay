/// D6, applied to the one file in this package that nobody wrote:
/// `lib/src/platform/messages.g.dart`.
///
/// The design doc predicted this regression by name — "a `copyWith`-style
/// data class with a generated `toString` is not [safe], and **generated
/// code is how this regresses**" — and it happened anyway. Pigeon v29.0.1
/// generates
///
/// ```text
/// ShowCheckoutRequest(url: $url, stopUrls: …, allowInsecureUrl: …)
/// ```
///
/// and `url` is the session's own hosted URL, whose fragment *is* the
/// checkout session's `client_secret`. One `debugPrint(request)`, one
/// Flutter error dump that quotes its arguments, one crash-reporter
/// breadcrumb, and a live payer credential is in a device log.
///
/// The generated file is hand-edited to redact. These tests are what makes
/// that edit durable: `dart run pigeon --input pigeons/checkout.dart`
/// overwrites the file and undoes it, and this suite is what says so.
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/src/platform/messages.g.dart';

const String _sessionSecret = 'cs_123_secret_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const String _sessionUrl =
    'https://checkout.example/c/cs_123?key=pk_test_1#$_sessionSecret';

void main() {
  group('ShowCheckoutRequest.toString (D6)', () {
    test('never renders the session URL, which carries the session secret in its fragment', () {
      final request = ShowCheckoutRequest(
        url: _sessionUrl,
        stopUrls: const <CheckoutStopUrl?>[],
        allowInsecureUrl: false,
      );

      final rendered = request.toString();

      expect(rendered, isNot(contains(_sessionSecret)));
      expect(rendered, isNot(contains(_sessionUrl)));
      expect(rendered, isNot(contains('checkout.example')));
      expect(rendered, contains('[${_sessionUrl.length} chars redacted]'));
      // The fields that are not credentials still render, so the type
      // stays diagnosable.
      expect(rendered, contains('allowInsecureUrl: false'));
    });

    test('the redaction survives string interpolation, which is how it would actually be logged', () {
      final request = ShowCheckoutRequest(
        url: _sessionUrl,
        stopUrls: const <CheckoutStopUrl?>[],
        allowInsecureUrl: true,
      );

      expect('showing $request', isNot(contains(_sessionSecret)));
      expect(<Object>[request].toString(), isNot(contains(_sessionSecret)));
    });
  });

  group('CheckoutWindowEvent.toString (D1/D6)', () {
    test('never renders the URL the window navigated to', () {
      final event = CheckoutWindowEvent(
        outcome: CheckoutWindowOutcome.stopUrlReached,
        reachedUrl: 'https://shop.example/thanks?order=1&token=abc',
      );

      final rendered = event.toString();

      expect(rendered, isNot(contains('shop.example')));
      expect(rendered, isNot(contains('token=abc')));
      expect(rendered, contains('stopUrlReached'));
    });

    test(
      'renders a null reachedUrl as null, not as a zero-length redaction',
      () {
        final event = CheckoutWindowEvent(
          outcome: CheckoutWindowOutcome.dismissed,
        );

        expect(event.toString(), contains('reachedUrl: null'));
      },
    );
  });

  group('CheckoutStopUrl.toString', () {
    test(
      'renders in full, because a normalised stop URL holds no credential',
      () {
        // Scheme, host, port and path only — the query and fragment a
        // credential would ride in are not carried across the channel at all
        // (`pigeons/checkout.dart`), so there is nothing here to redact and
        // redacting it would only make a diagnostic useless.
        final stopUrl = CheckoutStopUrl(
          scheme: 'https',
          host: 'shop.example',
          port: 443,
          path: '/thanks',
        );

        expect(stopUrl.toString(), contains('shop.example'));
        expect(stopUrl.toString(), contains('/thanks'));
      },
    );
  });
}
