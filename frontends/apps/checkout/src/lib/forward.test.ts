/**
 * `{CHECKOUT_SESSION_ID}` (D5) and the refusal of anything that is not an
 * absolute http(s) URL.
 */
import { describe, expect, it } from 'vitest';

import { forwardKindFor, forwardTarget, substituteSessionId } from './forward';
import type { CheckoutSession } from './types';

const SESSION_ID = 'cs_test_abc123';

function session(overrides: Partial<CheckoutSession> = {}): CheckoutSession {
  return {
    id: SESSION_ID,
    object: 'checkout.session',
    livemode: false,
    ui_mode: 'hosted',
    status: 'complete',
    payment_status: 'paid',
    success_url: 'https://shop.example/ok?sid={CHECKOUT_SESSION_ID}',
    cancel_url: 'https://shop.example/cancel',
    return_url: null,
    url: null,
    expires_at: 1,
    created: 1,
    ...overrides,
  };
}

describe('substituteSessionId', () => {
  it('substitutes the placeholder', () => {
    expect(substituteSessionId('https://shop/ok?sid={CHECKOUT_SESSION_ID}', SESSION_ID)).toBe(
      'https://shop/ok?sid=cs_test_abc123',
    );
  });

  it('substitutes every occurrence, not only the first', () => {
    expect(
      substituteSessionId(
        'https://shop/{CHECKOUT_SESSION_ID}/ok?sid={CHECKOUT_SESSION_ID}',
        SESSION_ID,
      ),
    ).toBe('https://shop/cs_test_abc123/ok?sid=cs_test_abc123');
  });

  it('leaves a URL without the placeholder exactly as the merchant wrote it — D3 stands', () => {
    expect(substituteSessionId('https://shop/ok', SESSION_ID)).toBe('https://shop/ok');
  });

  it('percent-encodes the id, so a widened id alphabet could not break out of a query value', () => {
    expect(substituteSessionId('https://shop/ok?sid={CHECKOUT_SESSION_ID}', 'cs a/b&c')).toBe(
      'https://shop/ok?sid=cs%20a%2Fb%26c',
    );
  });

  it('is case-sensitive: a lower-case spelling is not a placeholder', () => {
    expect(substituteSessionId('https://shop/ok?sid={checkout_session_id}', SESSION_ID)).toBe(
      'https://shop/ok?sid={checkout_session_id}',
    );
  });
});

describe('forwardTarget', () => {
  it('substitutes into the success URL for a paid hosted session', () => {
    expect(forwardTarget(session(), 'success')).toBe('https://shop.example/ok?sid=cs_test_abc123');
  });

  it('is null when the session names no URL for that outcome', () => {
    expect(forwardTarget(session({ return_url: null }), 'return')).toBeNull();
    expect(forwardTarget(session({ cancel_url: null }), 'cancel')).toBeNull();
  });

  it('refuses a javascript: URL rather than handing it to a navigation', () => {
    // The value came out of the database. This is the second lock; the
    // server's create-time validation is the first.
    expect(forwardTarget(session({ success_url: 'javascript:alert(1)' }), 'success')).toBeNull();
  });

  it('refuses a relative URL, which would resolve against vpay rather than the merchant', () => {
    expect(forwardTarget(session({ success_url: '/ok' }), 'success')).toBeNull();
  });
});

describe('forwardKindFor', () => {
  it('sends an embedded session to its single return_url whatever happened', () => {
    const embedded = session({ ui_mode: 'embedded' });
    expect(forwardKindFor(embedded, true)).toBe('return');
    expect(forwardKindFor(embedded, false)).toBe('return');
  });

  it('sends a hosted session to success on payment and cancel otherwise', () => {
    expect(forwardKindFor(session(), true)).toBe('success');
    expect(forwardKindFor(session(), false)).toBe('cancel');
  });
});
