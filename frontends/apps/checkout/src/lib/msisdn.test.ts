import { describe, expect, it } from 'vitest';

import { formatCameroonMsisdn, normalizeCameroonMsisdn } from './msisdn';

describe('normalizeCameroonMsisdn', () => {
  it('accepts the three shapes a payer types and answers one canonical string', () => {
    for (const input of [
      '+237671234567',
      '237671234567',
      '671234567',
      '+237 6 71 23 45 67',
      '237-671-234-567',
      '(237) 671 234 567',
      ' 237 671234567 ',
    ]) {
      expect(normalizeCameroonMsisdn(input), input).toBe('237671234567');
    }
  });

  it('refuses a national number that does not start with 6', () => {
    expect(normalizeCameroonMsisdn('771234567')).toBeNull();
    expect(normalizeCameroonMsisdn('237771234567')).toBeNull();
  });

  it('refuses the wrong number of digits', () => {
    expect(normalizeCameroonMsisdn('67123456')).toBeNull();
    expect(normalizeCameroonMsisdn('6712345678')).toBeNull();
    expect(normalizeCameroonMsisdn('')).toBeNull();
  });

  it('refuses another country', () => {
    expect(normalizeCameroonMsisdn('+33612345678')).toBeNull();
  });

  it('refuses letters — including the hex steering "numbers" the rail stubs key on', () => {
    // `237600000ce0` selects WireMock scenario `mtn-e2e-poll`. It is a
    // steering token, not a phone number, and a form that accepted it would
    // be accepting letters as a phone number for every payer. Recorded in
    // docs/plans/step9-notes/lane-3.md as a coordination item for lane 6.
    expect(normalizeCameroonMsisdn('237600000ce0')).toBeNull();
    expect(normalizeCameroonMsisdn('237600000f01')).toBeNull();
  });

  it('accepts the digits-only documentation numbers the conformance suite uses', () => {
    expect(normalizeCameroonMsisdn('237600000000')).toBe('237600000000');
    expect(normalizeCameroonMsisdn('237600000400')).toBe('237600000400');
  });

  it('refuses a value with a scheme or an injection attempt rather than stripping it', () => {
    expect(normalizeCameroonMsisdn('tel:237671234567')).toBeNull();
    expect(normalizeCameroonMsisdn("237671234567' OR 1=1")).toBeNull();
  });
});

describe('formatCameroonMsisdn', () => {
  it('groups the national part the way a Cameroonian reads it', () => {
    expect(formatCameroonMsisdn('237671234567')).toBe('+237 6 71 23 45 67');
  });

  it('leaves anything else untouched', () => {
    expect(formatCameroonMsisdn('123')).toBe('123');
  });
});
