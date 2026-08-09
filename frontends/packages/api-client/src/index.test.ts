import { describe, expect, it } from 'vitest';
import { formatAmount, listPayments, NotImplementedError } from './index.js';

describe('formatAmount', () => {
  it('treats XAF as zero-decimal', () => {
    expect(formatAmount(5000, 'xaf')).toBe('5000 XAF');
  });

  it('treats EUR as two-decimal', () => {
    expect(formatAmount(5000, 'EUR')).toBe('50.00 EUR');
    expect(formatAmount(5005, 'EUR')).toBe('50.05 EUR');
  });

  it('rejects non-integer amounts at the boundary', () => {
    expect(() => formatAmount(50.5, 'XAF')).toThrow(TypeError);
  });
});

describe('honesty', () => {
  it('unimplemented calls throw rather than returning empty data', async () => {
    await expect(listPayments()).rejects.toBeInstanceOf(NotImplementedError);
  });
});
