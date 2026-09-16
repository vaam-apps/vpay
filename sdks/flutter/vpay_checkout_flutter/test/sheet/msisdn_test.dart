import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

void main() {
  group(
    'normalizeCameroonMsisdn — the three shapes a payer actually types',
    () {
      test('bare national: 6XX XX XX XX', () {
        expect(normalizeCameroonMsisdn('671234567'), '237671234567');
      });

      test('with the country code, no plus: 2376XXXXXXXX', () {
        expect(normalizeCameroonMsisdn('237671234567'), '237671234567');
      });

      test('E.164 with a leading plus: +237 6XX XX XX XX', () {
        expect(normalizeCameroonMsisdn('+237671234567'), '237671234567');
      });

      test(
        'ASCII space, tab, hyphen, dot and parentheses are all separators',
        () {
          expect(normalizeCameroonMsisdn('+237 671 234 567'), '237671234567');
          expect(
            normalizeCameroonMsisdn('+237\t671\t234\t567'),
            '237671234567',
          );
          expect(normalizeCameroonMsisdn('+237-671-234-567'), '237671234567');
          expect(normalizeCameroonMsisdn('+237.671.234.567'), '237671234567');
          expect(normalizeCameroonMsisdn('+237 (671) 234 567'), '237671234567');
        },
      );

      test('NBSP (U+00A0) and narrow NBSP (U+202F) are accepted separators — '
          'real payers paste from real keyboards', () {
        expect(
          normalizeCameroonMsisdn('+237\u00a0671\u00a0234\u00a0567'),
          '237671234567',
        );
        expect(
          normalizeCameroonMsisdn('+237\u202f671\u202f234\u202f567'),
          '237671234567',
        );
        // The two mixed, as a French keyboard's own grouping might produce.
        expect(
          normalizeCameroonMsisdn('+237\u00a0671\u202f234\u00a0567'),
          '237671234567',
        );
      });

      test('leading/trailing whitespace is trimmed first', () {
        expect(normalizeCameroonMsisdn('  +237671234567  '), '237671234567');
      });

      test('every Cameroon mobile number begins with 6 — a 7 is refused', () {
        expect(normalizeCameroonMsisdn('771234567'), isNull);
        expect(normalizeCameroonMsisdn('237771234567'), isNull);
      });

      test('wrong digit count is refused, not truncated or padded', () {
        expect(normalizeCameroonMsisdn('67123456'), isNull); // one short
        expect(normalizeCameroonMsisdn('6712345678'), isNull); // one long
      });

      test('a second plus sign is refused', () {
        expect(normalizeCameroonMsisdn('++237671234567'), isNull);
      });

      test('punctuation outside the separator set is refused', () {
        expect(normalizeCameroonMsisdn('237#671234567'), isNull);
      });

      test('an empty or whitespace-only string is refused', () {
        expect(normalizeCameroonMsisdn(''), isNull);
        expect(normalizeCameroonMsisdn('   '), isNull);
      });

      test(
        'the WireMock hex-steering fixtures are letters, not digits, and are '
        'refused — a fixture problem, not a validator bug (issue #189)',
        () {
          expect(normalizeCameroonMsisdn('237600000ce0'), isNull);
          expect(normalizeCameroonMsisdn('237600000f01'), isNull);
        },
      );
    },
  );

  group('formatCameroonMsisdn — display only, never sent to a rail', () {
    test('groups the canonical form the way a Cameroonian reads it', () {
      expect(formatCameroonMsisdn('237671234567'), '+237 6 71 23 45 67');
    });

    test('a string of the wrong length is returned unchanged', () {
      expect(formatCameroonMsisdn('12345'), '12345');
    });
  });
}
