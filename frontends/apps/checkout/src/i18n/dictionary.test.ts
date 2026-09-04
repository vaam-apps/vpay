/**
 * The dictionaries, and the locale negotiation that picks between them.
 *
 * The parity tests iterate the keys rather than listing them: a test that
 * named the strings it expects would go stale the moment a screen gains one,
 * and the failure this file exists to catch — a French string that quietly
 * lost its `{amount}` — is invisible to a spot check.
 */
import { describe, expect, it } from 'vitest';

import { DEFAULT_LOCALE, DICTIONARIES, LOCALES, en, format, fr, pickLocale, placeholdersOf, translator } from './index';

const KEYS = Object.keys(en) as (keyof typeof en)[];

describe('the dictionaries', () => {
  it('has a non-trivial number of keys, so the tests below are not vacuous', () => {
    expect(KEYS.length).toBeGreaterThan(30);
  });

  it('carries every key in both locales, with a non-empty value', () => {
    for (const locale of LOCALES) {
      const dictionary = DICTIONARIES[locale];
      for (const key of KEYS) {
        expect(dictionary[key], `${locale} is missing ${key}`).toBeTypeOf('string');
        expect(dictionary[key].trim(), `${locale}.${key} is blank`).not.toBe('');
      }
    }
  });

  it('has no key in one locale that the other lacks', () => {
    expect(Object.keys(fr).sort()).toEqual(KEYS.slice().sort());
  });

  it('uses the same placeholders in both locales for every key', () => {
    for (const key of KEYS) {
      expect(placeholdersOf(fr[key]).sort(), `placeholders differ for ${key}`).toEqual(
        placeholdersOf(en[key]).sort(),
      );
    }
  });

  it('translates every key without leaving a placeholder unfilled', () => {
    const values = { merchant: 'Boutique', amount: '5 000 FCFA', seconds: 3, rail: 'zzz_pay' };
    for (const locale of LOCALES) {
      const t = translator(locale);
      for (const key of KEYS) {
        const rendered = t(key, values);
        expect(rendered, `${locale}.${key} kept a placeholder`).not.toMatch(/\{[a-z_]+\}/);
        expect(rendered.trim()).not.toBe('');
      }
    }
  });

  it('actually differs between the two locales, so neither is a copy of the other', () => {
    const identical = KEYS.filter((key) => en[key] === fr[key]);
    // A handful legitimately match — brand names, and the two language
    // names in the switch, which are always shown in their own language.
    expect(identical.length).toBeLessThan(6);
  });
});

describe('format', () => {
  it('substitutes named placeholders', () => {
    expect(format('Pay {merchant} {amount}', { merchant: 'Shop', amount: '5' })).toBe('Pay Shop 5');
  });

  it('leaves a placeholder with no value verbatim, rather than blanking it', () => {
    expect(format('Pay {merchant}', {})).toBe('Pay {merchant}');
  });

  it('leaves an unterminated brace alone', () => {
    expect(format('Pay {merchant', { merchant: 'Shop' })).toBe('Pay {merchant');
  });
});

describe('pickLocale', () => {
  it('is French when the header is absent, empty or a wildcard', () => {
    expect(pickLocale(null)).toBe('fr');
    expect(pickLocale('')).toBe('fr');
    expect(pickLocale('*')).toBe('fr');
    expect(DEFAULT_LOCALE).toBe('fr');
  });

  it('matches on the primary subtag', () => {
    expect(pickLocale('fr-CM')).toBe('fr');
    expect(pickLocale('en-GB')).toBe('en');
  });

  it('honours q-values rather than header order', () => {
    expect(pickLocale('fr;q=0.2, en;q=0.9')).toBe('en');
    expect(pickLocale('en;q=0.1, fr;q=0.8')).toBe('fr');
  });

  it('skips a range the browser explicitly refused with q=0', () => {
    expect(pickLocale('fr;q=0, en')).toBe('en');
  });

  it('falls through a language it has no dictionary for', () => {
    expect(pickLocale('de-DE, en;q=0.5')).toBe('en');
    expect(pickLocale('de-DE')).toBe('fr');
  });
});
