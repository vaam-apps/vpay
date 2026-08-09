import { describe, expect, it } from 'vitest';
import { PAYMENT_STATUS, statusLabel, statusTone } from './index.js';

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
