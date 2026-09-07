import { describe, expect, it } from 'vitest';
import {
  CHECKOUT_OUTCOME,
  PAYMENT_STATUS,
  checkoutOutcomeTone,
  statusLabel,
  statusTone,
} from './index.js';

describe('status tokens', () => {
  it('covers every status with a tone and a label', () => {
    for (const s of PAYMENT_STATUS) {
      expect(statusTone[s], `tone for ${s}`).toBeTruthy();
      expect(statusLabel[s], `label for ${s}`).toBeTruthy();
    }
  });

  it('never labels processing as nearly-done', () => {
    expect(statusLabel.processing.toLowerCase()).not.toContain('almost');
    expect(statusLabel.processing.toLowerCase()).not.toContain('complete');
  });

  it('reserves success tone for succeeded alone', () => {
    const successes = PAYMENT_STATUS.filter((s) => statusTone[s] === 'success');
    expect(successes).toEqual(['succeeded']);
  });
});

describe('checkout outcome tones', () => {
  it('covers every outcome', () => {
    for (const outcome of CHECKOUT_OUTCOME) {
      expect(checkoutOutcomeTone[outcome], `tone for ${outcome}`).toBeTruthy();
    }
  });

  it('never renders a FAILED payment as neutral — the defect this table exists for', () => {
    // A payer standing in a shop reading a grey box does not know the payment
    // did not happen. This is the assertion; the rest of the table is taste.
    expect(checkoutOutcomeTone.failed).toBe('error');
  });

  it('is not the operator’s status palette wearing another name', () => {
    // `requires_payment_method` is the intent status a failed attempt leaves
    // behind, and it is legitimately NEUTRAL on a dashboard: it means
    // "awaiting a payment method", not "this failed". The two tables must be
    // able to disagree, and here they do.
    expect(statusTone.requires_payment_method).toBe('neutral');
    expect(checkoutOutcomeTone.failed).not.toBe(statusTone.requires_payment_method);
  });

  it('keeps success for the one outcome that succeeded', () => {
    const successes = CHECKOUT_OUTCOME.filter((o) => checkoutOutcomeTone[o] === 'success');
    expect(successes).toEqual(['succeeded']);
  });

  it('tones a canceled payment warning, not error — decision D4, 2026-09-07', () => {
    // Canceled is the payer's own action; failed is not. Softening one and
    // not the other is the point, so this asserts both sides of it.
    expect(checkoutOutcomeTone.canceled).toBe('warning');
    expect(checkoutOutcomeTone.failed).toBe('error');
  });
});
