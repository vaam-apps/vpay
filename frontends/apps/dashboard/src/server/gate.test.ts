import { describe, expect, it } from 'vitest';

import type { ApiFailure, SessionResponse } from './api';
import { gateFor, refusalFor, REMINT_AFTER_FRACTION, totpGateFor } from './gate';

/** The shipping `staff_auth.access_token_ttl_seconds`. */
const TTL_SECONDS = 900;
/** A round instant to do the expiry arithmetic against. */
const MINTED_AT = Date.parse('2026-09-10T12:00:00Z');

const SESSION: SessionResponse = {
  staff_id: 'stf_example_1',
  display_name: 'Ada',
  email: 'ada@example.test',
  merchant_id: 'demo-merchant-tenant',
  password_change_required: false,
  access_token: 'header.payload.signature',
  access_token_expires_at: new Date(MINTED_AT + TTL_SECONDS * 1000).toISOString(),
  access_token_ttl_seconds: TTL_SECONDS,
};

/** `milliseconds` after the token was minted. */
const at = (seconds: number): number => MINTED_AT + seconds * 1000;

describe('the gate', () => {
  it('sends a session still carrying the printed password to the password page', () => {
    // Even when a token somehow exists: /oauth/authorize refuses such a
    // session, so a token on one would be a token the flow cannot produce,
    // and honouring it here would route around ADR-0017 decision 1.
    const gate = gateFor({ ...SESSION, password_change_required: true }, at(0));
    expect(gate.kind).toBe('must-change-password');
  });

  it('asks for a code exchange when no token has been minted yet', () => {
    const { access_token: _dropped, ...withoutToken } = SESSION;
    expect(gateFor(withoutToken, at(0)).kind).toBe('needs-token');
  });

  it('treats an empty token as no token', () => {
    expect(gateFor({ ...SESSION, access_token: '' }, at(0)).kind).toBe('needs-token');
  });

  it('lets a signed-in session through with the token from the session row', () => {
    const gate = gateFor(SESSION, at(0));
    expect(gate.kind).toBe('ready');
    if (gate.kind === 'ready') {
      expect(gate.accessToken).toBe('header.payload.signature');
    }
  });
});

describe('replacing the token before it expires (issue #88 item 1)', () => {
  // The margin is 20 % of the TTL, so at 900 s the token is replaced with 180
  // seconds still on it.
  const marginSeconds = TTL_SECONDS * (1 - REMINT_AFTER_FRACTION);

  it('reads with the token it holds for the first four fifths of its life', () => {
    // One second inside the margin. The exchange is a round trip and a
    // credential operation; doing it on every render would be the opposite
    // mistake from the one this fixes.
    expect(gateFor(SESSION, at(TTL_SECONDS - marginSeconds - 1)).kind).toBe('ready');
  });

  it('re-mints at the margin, and the token it holds is still good', () => {
    // THE POINT OF THE WHOLE CHANGE: this instant is BEFORE the expiry. Until
    // 2026-09-10 nothing re-minted at all and the first read after 900 s
    // failed — the exp28 review's finding F4, measured in a browser.
    const gate = gateFor(SESSION, at(TTL_SECONDS - marginSeconds));
    expect(gate.kind).toBe('stale-token');
    if (gate.kind === 'stale-token') {
      // Carried, because `requireStaff` falls back to it when vpay cannot be
      // reached for the re-mint: it has not expired, it is merely due.
      expect(gate.accessToken).toBe('header.payload.signature');
    }
  });

  it('is still a stale token rather than a missing one after the expiry', () => {
    // A render that arrives late — a slow request, a suspended laptop. The
    // fallback in `requireStaff` is refused for a `401` either way, and
    // `dash-read.ts`'s reactive retry is what covers a token that really is
    // dead.
    expect(gateFor(SESSION, at(TTL_SECONDS + 60)).kind).toBe('stale-token');
  });

  it('re-mints when vpay sent no expiry at all', () => {
    // THE DECISIVE MUTATION for the fail-closed direction: make an absent or
    // unparseable expiry read as 'ready'. These three then render with a token
    // whose life this app knows nothing about, which is the fifteen minutes
    // over again.
    const { access_token_expires_at: _dropped, ...withoutExpiry } = SESSION;
    expect(gateFor(withoutExpiry, at(0)).kind).toBe('stale-token');
    expect(gateFor({ ...SESSION, access_token_expires_at: 'not a date' }, at(0)).kind).toBe(
      'stale-token',
    );
    expect(gateFor({ ...SESSION, access_token_ttl_seconds: 0 }, at(0)).kind).toBe('stale-token');
  });

  it('scales the margin with a TTL an e2e overlay shortened', () => {
    // `staff_auth.access_token_ttl_seconds` is configuration, and
    // `compose.e2e.yml` sets a few seconds so a browser run can cross an
    // expiry. A FIXED margin — sixty seconds, say — would make every render
    // under that overlay a re-mint, and the Cypress leg would pass while
    // proving nothing about the margin.
    const short: SessionResponse = {
      ...SESSION,
      access_token_ttl_seconds: 20,
      access_token_expires_at: new Date(MINTED_AT + 20_000).toISOString(),
    };
    expect(gateFor(short, at(15)).kind).toBe('ready');
    expect(gateFor(short, at(16)).kind).toBe('stale-token');
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
