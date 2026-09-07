import { NextResponse } from 'next/server';

import { COOKIE_ATTRIBUTES, SESSION_COOKIE } from './cookies';
import { LOGIN_PATH } from './session';

/**
 * The response that removes a dead session cookie and sends a browser to the
 * sign-in form.
 *
 * # Why this exists at all, and it is not tidiness
 *
 * `cookies().set(…)` throws outside a Server Action or a Route Handler. Next
 * says so in as many words — "Cookies can only be modified in a Server Action
 * or Route Handler" — and a Server Component that calls it does not fail
 * quietly: the exception escapes the render and the browser gets **500**.
 *
 * `requireStaff` used to clear the cookie itself, mid-render, on every refusal
 * it handles. Measured against the real compose stack on 2026-09-07
 * (`docs/plans/exp28-dashboard-pages-notes/opus-review.md`, finding F1): a
 * forged cookie, a session signed out from another browser, and an account an
 * operator had just disabled each answered `500` on `/payments` rather than
 * the sign-in form — and, the cookie never having been cleared, so did every
 * request after it.
 *
 * So the decision is made where it belongs (the page, in `session.ts`) and
 * *acted on* where a cookie may legally be written: `app/signed-out/route.ts`,
 * a Route Handler, which is four lines around this function.
 *
 * # Why the cookie goes on the RESPONSE and not through `cookies()`
 *
 * A Route Handler may use either. Building the response object makes this a
 * plain function — no `next/headers`, no request scope — which is why
 * `signed-out.test.ts` can assert on the `Set-Cookie` it produces. A handler
 * that could only be exercised through a browser is a handler nothing checks,
 * and this is precisely the branch `dashboard.cy.ts` never reached.
 *
 * # A RELATIVE `Location`, and `NextResponse.redirect` cannot give one
 *
 * `NextResponse.redirect` demands an absolute URL, and the only absolute URL
 * available here is built from `request.url` — which inside the container is
 * `http://<container id>:3000/…`. Measured: the first version of this file
 * answered `location: http://be34398149d3:3000/login`, a hostname that
 * resolves nowhere outside the compose network, so a browser followed it into
 * a DNS failure. The header is therefore written by hand.
 *
 * A relative reference is legal (RFC 9110 §10.2.2), is what Next's own
 * `redirect()` emits from a Server Component, and has two properties an
 * absolute one cannot have: it cannot leak the internal hostname, and it
 * cannot be an open redirect however this function is called.
 *
 * # It is not the sign-out button
 *
 * That is `signOut` in `actions.ts`, which deletes the `staff_sessions` row
 * first — the revocation (ADR-0017 decision 2). This only forgets a cookie,
 * which is all a page is entitled to do about a session vpay has already
 * refused.
 */
export function signedOutResponse(): NextResponse {
  const response = new NextResponse(null, {
    status: 303,
    headers: { location: LOGIN_PATH },
  });
  // `maxAge: 0` with the SAME attributes the cookie was written with. A
  // deletion whose `path` or `secure` differs from the original leaves the
  // original in place — the browser treats them as different cookies — which
  // is the drift `COOKIE_ATTRIBUTES` exists to stop (`cookies.ts`).
  response.cookies.set(SESSION_COOKIE, '', { ...COOKIE_ATTRIBUTES, maxAge: 0 });
  return response;
}
