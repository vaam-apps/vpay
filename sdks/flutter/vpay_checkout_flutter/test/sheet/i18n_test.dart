import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

/// #189: "i18n: 2 locales, ~71 keys, French default." These tests check the
/// count mechanically rather than trusting the two dictionaries in
/// `i18n.dart` to agree by inspection — a key present in one and missing
/// from the other is exactly the kind of gap `en.ts`/`fr.ts` catch at
/// compile time (`fr.ts` is typed `Record<MessageKey, string>`) that this
/// Dart port, two plain `Map<String, String>` literals, cannot get from the
/// type system alone.
void main() {
  group('i18n completeness', () {
    test('both dictionaries carry the same 72 keys as en.ts/fr.ts', () {
      expect(vpayCheckoutMessageKeys.length, 72);
      expect(vpayCheckoutMessageKeysFr.length, 72);
      expect(vpayCheckoutMessageKeys, vpayCheckoutMessageKeysFr);
    });

    test('every {placeholder} in the English string also appears in the French one, and vice versa', () {
      final RegExp placeholder = RegExp(r'\{([a-z]+)\}');
      for (final String key in vpayCheckoutMessageKeys) {
        final String en = const VpayCheckoutStrings(VpayLocale.en).t(key);
        final String fr = const VpayCheckoutStrings(VpayLocale.fr).t(key);
        final Set<String> enPlaceholders = placeholder
            .allMatches(en)
            .map((Match m) => m.group(1)!)
            .toSet();
        final Set<String> frPlaceholders = placeholder
            .allMatches(fr)
            .map((Match m) => m.group(1)!)
            .toSet();
        expect(
          enPlaceholders,
          frPlaceholders,
          reason: 'key "$key" has mismatched placeholders between locales',
        );
      }
    });
  });

  group('VpayCheckoutStrings.t', () {
    test('substitutes every {name} placeholder given', () {
      const t = VpayCheckoutStrings(VpayLocale.en);
      expect(
        t.t('page.pay_to', const {'merchant': 'Njangi Store'}),
        'Pay Njangi Store',
      );
    });

    test('a key the catalogue does not carry renders as the bracketed key, never a thrown error', () {
      const t = VpayCheckoutStrings(VpayLocale.fr);
      expect(t.t('not.a.real.key'), '{not.a.real.key}');
    });

    test('French default: VpayLocale.fallback is fr', () {
      expect(VpayLocale.fallback, VpayLocale.fr);
    });
  });

  group('failureMessageKey', () {
    test('null in, null out', () {
      expect(failureMessageKey(null), isNull);
    });

    test('a known code maps to its own failure.* key', () {
      expect(
        failureMessageKey('insufficient_funds'),
        'failure.insufficient_funds',
      );
    });

    test(
      'an unrecognised code falls back to failure.unknown — never the raw code',
      () {
        expect(failureMessageKey('some_future_code'), 'failure.unknown');
      },
    );
  });

  group('railLabelFor', () {
    test('resolves a known rail through the catalogue', () {
      expect(
        railLabelFor(
          locale: VpayLocale.fr,
          labelKey: 'rail.mtn_momo',
          code: 'mtn_momo',
        ),
        'MTN Mobile Money',
      );
    });

    test("an unknown rail's label_key falls back to the deployment's own display_name", () {
      expect(
        railLabelFor(
          locale: VpayLocale.en,
          labelKey: 'rail.some_new_rail',
          code: 'some_new_rail',
          configuredEn: 'New Rail Co',
          configuredFr: 'Nouveau Rail',
        ),
        'New Rail Co',
      );
    });

    test(
      'an unknown rail with no configured name falls back to its raw code',
      () {
        expect(
          railLabelFor(
            locale: VpayLocale.fr,
            labelKey: 'rail.some_new_rail',
            code: 'some_new_rail',
          ),
          'some_new_rail',
        );
      },
    );
  });
}
