import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

void main() {
  group('providerReason', () {
    test('null in, null out', () {
      expect(providerReason(null), isNull);
    });

    test('a clean, short message is returned unchanged', () {
      expect(providerReason('Insufficient funds.'), 'Insufficient funds.');
    });

    test('an empty or whitespace-only message becomes null, not ""', () {
      expect(providerReason(''), isNull);
      expect(providerReason('   '), isNull);
    });

    test('internal whitespace runs collapse to a single space', () {
      expect(
        providerReason('Insufficient   funds\t\tnow'),
        'Insufficient funds now',
      );
    });

    test('leading and trailing whitespace is trimmed', () {
      expect(providerReason('  Insufficient funds.  '), 'Insufficient funds.');
    });

    test('C0 control characters (e.g. NUL, a bell) are stripped', () {
      expect(
        providerReason('Insufficient\u0000funds\u0007.'),
        'Insufficient funds .',
      );
    });

    test('C1 control characters are stripped', () {
      expect(providerReason('MTN\u0085error'), 'MTN error');
    });

    test('the Unicode line separators are stripped', () {
      expect(providerReason('MTN\u2028error\u2029code'), 'MTN error code');
    });

    test('a message at exactly the cap is untouched', () {
      final String exact = 'a' * maxProviderReasonLength;
      final String? result = providerReason(exact);
      expect(result, exact);
      expect(result!.length, maxProviderReasonLength);
    });

    test(
      'a message over the cap is truncated to 300 chars with an ellipsis',
      () {
        final String long = 'a' * (maxProviderReasonLength + 50);
        final String? result = providerReason(long);
        expect(result, isNotNull);
        expect(result!.length, maxProviderReasonLength);
        expect(result, endsWith('…'));
        expect(
          result.substring(0, maxProviderReasonLength - 1),
          'a' * (maxProviderReasonLength - 1),
        );
      },
    );

    test('shown beside the translated failure, never instead of it: this '
        'function never answers a FailureCode or a MessageKey, only the '
        'rail\'s own cleaned text — the pairing is the caller\'s job', () {
      // There is no failureMessage/FAILURE_MESSAGES lookup in this port
      // (see failures.dart's own module doc comment for why) — asserting
      // that fact structurally: providerReason's signature is
      // String? Function(String?), so it cannot itself decide what the
      // translated line says.
      expect(providerReason('insufficient_funds'), 'insufficient_funds');
    });
  });
}
