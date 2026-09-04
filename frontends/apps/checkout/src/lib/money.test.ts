import { describe, expect, it } from 'vitest';

import { currencyExponent, formatAmount, toDecimalString } from './money';

/**
 * Intl inserts narrow and non-breaking spaces and a locale's own grouping
 * separator; the assertions below are about the digits and the decimal
 * point, which are the part that must agree with the server.
 */
function digits(value: string): string {
  return value.replace(/[^0-9.]/g, '');
}

describe('the exponent table', () => {
  it('is 0 for XAF and 2 for EUR — docs/flows/money.md', () => {
    expect(currencyExponent('XAF')).toBe(0);
    expect(currencyExponent('xaf')).toBe(0);
    expect(currencyExponent('EUR')).toBe(2);
  });
});

describe('toDecimalString', () => {
  it('renders the same digits at exponent 0 and pads the fraction at exponent 2', () => {
    expect(toDecimalString(5000, 0)).toBe('5000');
    expect(toDecimalString(5000, 2)).toBe('50.00');
    expect(toDecimalString(5, 2)).toBe('0.05');
    expect(toDecimalString(0, 2)).toBe('0.00');
  });

  it('does not go through a float, so a large amount keeps every digit', () => {
    expect(toDecimalString(900719925474099, 2)).toBe('9007199254740.99');
  });
});

describe('formatAmount', () => {
  it('renders 5000 XAF as five thousand, not fifty', () => {
    expect(digits(formatAmount(5000, 'xaf', 'fr'))).toBe('5000');
    expect(digits(formatAmount(5000, 'xaf', 'en'))).toBe('5000');
  });

  it('renders 5000 EUR as 50.00', () => {
    expect(digits(formatAmount(5000, 'eur', 'en'))).toBe('50.00');
  });

  it('names the currency in both locales', () => {
    expect(formatAmount(5000, 'xaf', 'en')).toMatch(/XAF|FCFA|CFA/);
    expect(formatAmount(5000, 'xaf', 'fr')).toMatch(/XAF|FCFA|CFA/);
  });

  it('refuses a fractional count of minor units rather than rounding it', () => {
    expect(() => formatAmount(50.5, 'xaf', 'en')).toThrow(TypeError);
  });

  it('falls back to digits and the code for a currency ICU does not know', () => {
    expect(formatAmount(5000, 'zzz', 'en')).toContain('ZZZ');
  });
});
