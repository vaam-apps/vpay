import { describe, expect, it } from 'vitest';

import type { SessionResponse } from './api';
import { gateFor } from './gate';

const SESSION: SessionResponse = {
  staff_id: 'stf_example_1',
  display_name: 'Ada',
  email: 'ada@example.test',
  merchant_id: 'demo-merchant-tenant',
  password_change_required: false,
  access_token: 'header.payload.signature',
};

describe('the gate', () => {
  it('sends a session still carrying the printed password to the password page', () => {
    // Even when a token somehow exists: /oauth/authorize refuses such a
    // session, so a token on one would be a token the flow cannot produce,
    // and honouring it here would route around ADR-0017 decision 1.
    const gate = gateFor({ ...SESSION, password_change_required: true });
    expect(gate.kind).toBe('must-change-password');
  });

  it('asks for a code exchange when no token has been minted yet', () => {
    const { access_token: _dropped, ...withoutToken } = SESSION;
    expect(gateFor(withoutToken).kind).toBe('needs-token');
  });

  it('treats an empty token as no token', () => {
    expect(gateFor({ ...SESSION, access_token: '' }).kind).toBe('needs-token');
  });

  it('lets a signed-in session through with the token from the session row', () => {
    const gate = gateFor(SESSION);
    expect(gate.kind).toBe('ready');
    if (gate.kind === 'ready') {
      expect(gate.accessToken).toBe('header.payload.signature');
    }
  });
});
