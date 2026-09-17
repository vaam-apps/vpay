import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

void main() {
  group('formatAmountForDisplay', () {
    test('a zero-decimal currency (XAF) never gains a decimal point', () {
      expect(formatAmountForDisplay(5000, 'xaf', true), contains('5'));
      expect(formatAmountForDisplay(5000, 'xaf', true), isNot(contains('.')));
      expect(formatAmountForDisplay(5000, 'xaf', true), isNot(contains(',')));
    });

    test('groups thousands for French with a narrow no-break space', () {
      final String result = formatAmountForDisplay(1234567, 'xaf', true);
      expect(result, contains('1 234 567'));
      expect(result, contains('XAF'));
    });

    test('groups thousands for English with a comma', () {
      final String result = formatAmountForDisplay(1234567, 'xaf', false);
      expect(result, contains('1,234,567'));
      expect(result, contains('XAF'));
    });

    test('a two-decimal currency keeps its decimal point after grouping', () {
      final String result = formatAmountForDisplay(123456789, 'usd', false);
      // 123456789 minor units at exponent 2 -> "1234567.89"
      expect(result, contains('1,234,567.89'));
    });

    test('French decimal separator is a comma, not a period', () {
      final String result = formatAmountForDisplay(123456789, 'usd', true);
      expect(result, contains('89'));
      expect(result, isNot(contains('.')));
    });

    test('never touches a float — digit surgery only, reusing money.dart', () {
      // A value the classic float bug would corrupt: 0.1 + 0.2 style drift
      // never has anywhere to enter, because every step here is string
      // surgery on `minorUnitsToDecimalString`'s own output.
      expect(formatAmountForDisplay(1, 'xaf', true), '1 XAF');
      expect(formatAmountForDisplay(100, 'usd', false), 'USD 1.00');
    });

    test('a negative amount keeps its sign', () {
      expect(formatAmountForDisplay(-500, 'xaf', true), contains('-'));
    });
  });
}
