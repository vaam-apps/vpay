import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/src/redaction.dart';

void main() {
  group('redacted', () {
    test('renders the exact character count, not the value', () {
      expect(redacted('pi_123_secret_abcdefgh'), '[22 chars redacted]');
      expect(redacted('pi_123_secret_abcdefgh'), isNot(contains('pi_123')));
      expect(redacted('pi_123_secret_abcdefgh'), isNot(contains('abcdefgh')));
    });

    test(
      'renders the literal null for a null value, not [0 chars redacted]',
      () {
        expect(redacted(null), 'null');
      },
    );

    test('renders a zero-length secret distinctly from null', () {
      expect(redacted(''), '[0 chars redacted]');
      expect(redacted(''), isNot('null'));
    });
  });
}
