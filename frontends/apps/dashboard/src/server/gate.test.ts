import { describe, expect, it } from 'vitest';

import type { ApiFailure, SessionResponse } from './api';
import { gateFor, refusalFor, totpGateFor } from './gate';

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

describe('what a refused session read means for the cookie', () => {
  const failure = (status: number): ApiFailure => ({
    status,
    message: 'vpay said something',
    requestId: 'req_example_1',
  });

  it('signs out on a 401, which is the only status a refused session gets', () => {
    // vpay answers 401 for EVERY session refusal by design — absent, expired,
    // idle, forged, disabled, at the wrong stage. So this mapping is exact
    // rather than conservative.
    expect(refusalFor(failure(401))).toBe('sign-out');
  });

  it('does NOT sign out on a 503, and this is the case the feature exists for', () => {
    // Until 2026-09-10 `requireStaff` sent a browser to /signed-out for every
    // failure of the session read, so a rolling restart of vpay signed every
    // staff member out — indistinguishably from having been signed out on
    // purpose (issue #88 item 2).
    //
    // THE DECISIVE MUTATION: widen `refusalFor` to `status >= 400` or
    // `status !== 200`. This assertion reads 'sign-out'.
    expect(refusalFor(failure(503))).toBe('outage');
  });

  it('does not sign out when there was no response at all', () => {
    // `server/api.ts` turns a rejected `fetch` into `status: 0` rather than
    // throwing, so "the connection was reset" arrives here as a failure like
    // any other. It is the commonest shape of the outage this exists for.
    expect(refusalFor(failure(0))).toBe('outage');
  });

  it('treats a 403 on the session route as an outage rather than a sign-out', () => {
    // On this route a 403 means something in front of vpay refused this app,
    // which is a deployment problem and not a fact about the person.
    expect(refusalFor(failure(403))).toBe('outage');
  });

  it('does not sign out on a 500 or a 502', () => {
    expect(refusalFor(failure(500))).toBe('outage');
    expect(refusalFor(failure(502))).toBe('outage');
  });
});

describe('what /login/totp must do about the session it read', () => {
  const failure = (status: number): ApiFailure => ({
    status,
    message: 'vpay said something',
    requestId: 'req_example_2',
  });

  it('renders the code form for a session that has not presented one yet', () => {
    expect(totpGateFor({ stage: 'pending_totp' }, null).kind).toBe('enter-code');
  });

  it('sends a session that already has both factors on to the payments list', () => {
    // A back button, or a reload after the action redirected. Rendering the
    // code form again would ask for a credential vpay would refuse as a
    // replay.
    expect(totpGateFor({ stage: 'authenticated' }, null).kind).toBe('signed-in');
  });

  it('ends the session only for a 401', () => {
    // The 401 here is the STAGE READ's, never the TOTP action's — that one
    // also means "wrong code". This is the whole of F6's fix.
    expect(totpGateFor(null, failure(401)).kind).toBe('dead');
  });

  it('keeps the cookie when vpay could not be reached', () => {
    // THE DECISIVE MUTATION: map any failure to 'dead'. These two then read
    // 'dead', which is a vpay restart signing everybody out mid-sign-in
    // (issue #88 item 2, on this route).
    const outage = totpGateFor(null, failure(0));
    expect(outage.kind).toBe('outage');
    if (outage.kind === 'outage') {
      expect(outage.failure.status).toBe(0);
    }
    expect(totpGateFor(null, failure(503)).kind).toBe('outage');
  });

  it('fails closed when the read answered neither a stage nor a failure', () => {
    expect(totpGateFor(null, null).kind).toBe('dead');
  });
});
