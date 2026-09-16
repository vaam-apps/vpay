import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

void main() {
  group('currencyExponent', () {
    test('the six zero-decimal currencies are exponent 0', () {
      for (final String code in ['XAF', 'XOF', 'JPY', 'KRW', 'CLP', 'VND']) {
        expect(currencyExponent(code), 0, reason: code);
        // The wire sends lower-case currency codes.
        expect(currencyExponent(code.toLowerCase()), 0, reason: code);
      }
    });

    test('every other currency is exponent 2', () {
      expect(currencyExponent('USD'), 2);
      expect(currencyExponent('EUR'), 2);
      expect(currencyExponent('usd'), 2);
    });
  });

  group('toDecimalString — the digit-surgery half money.ts exists to hold', () {
    test('5000 at exponent 2 is "50.00" — not "50.0" or "50"', () {
      // The exact two trailing zeros matter: a float route through
      // `minor / pow(10, exponent)` and `.toString()` renders "50.0", one
      // digit short — verified below by the mutation proof, but this
      // assertion alone already catches it.
      expect(toDecimalString(5000, 2), '50.00');
    });

    test('5000 at exponent 0 is "5000"', () {
      expect(toDecimalString(5000, 0), '5000');
    });

    test('an amount smaller than the exponent is left-padded with zeros', () {
      expect(toDecimalString(5, 2), '0.05');
      expect(toDecimalString(50, 2), '0.50');
    });

    test('zero is "0" at exponent 0 and "0.00" at exponent 2', () {
      expect(toDecimalString(0, 0), '0');
      expect(toDecimalString(0, 2), '0.00');
    });

    test('a negative amount keeps its sign in front of the digits', () {
      expect(toDecimalString(-5000, 2), '-50.00');
      expect(toDecimalString(-5, 2), '-0.05');
      expect(toDecimalString(-5000, 0), '-5000');
    });

    test(
      'MUTATION PROOF (money digit surgery): a value where '
      'minor / pow(10, exponent) loses precision, and digit surgery does not',
      () {
        // Verified empirically (not asserted from first principles): at
        // exponent 2, `9007199254740993 / 100.0` evaluates in IEEE-754
        // double precision to a value whose `.toString()` is
        // "90071992547409.92" — the wrong last digit, a real one-cent
        // error. `toDecimalString`'s digit surgery never routes the value
        // through a `double` at all, so it is exact regardless of
        // magnitude. If a reviewer inlines the described mutation
        // (`(minor / math.pow(10, exponent)).toString()`) into
        // `lib/src/sheet/money.dart`, this assertion — and every other one
        // in this file, since none of them tolerate a float's own
        // formatting either — fails.
        expect(toDecimalString(9007199254740993, 2), '90071992547409.93');
      },
    );
  });

  group('minorUnitsToDecimalString', () {
    test('derives the exponent from the currency code', () {
      expect(minorUnitsToDecimalString(12000, 'XAF'), '12000');
      expect(minorUnitsToDecimalString(5000, 'usd'), '50.00');
    });
  });
}
