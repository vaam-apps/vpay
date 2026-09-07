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
 *
 * # A `GET` that clears a cookie, and the rule this app already wrote down
 *
 * `signed-in-bar.tsx` says why signing out is a `<form>` POST: "a sign-out
 * link is something a prefetcher, a link scanner or an `<img src>` in a chat
 * message can fire". This route is a `GET` — it has to be, a page redirects
 * to it — so it would be exactly that link, and forcing a staff member out of
 * their session from any page on the internet is a denial of service however
 * small.
 *
 * {@link isNavigation} is the answer, and it is the one the browser already
 * has: every request a modern browser makes carries `Sec-Fetch-Dest`, and it
 * says `document` only for a **top-level navigation**. An `<img>`, a
 * `<script>`, a `fetch` and a prefetch each say something else, so each gets
 * the redirect and keeps its cookie. A request with no fetch metadata at all
 * — an older browser, `curl` — is treated as a navigation, because that is
 * the direction that keeps the feature working for the people who need it and
 * the attack needs a browser that would have sent the header.
 */
export function signedOutResponse(headers?: Headers): NextResponse {
  const response = new NextResponse(null, {
    status: 303,
    headers: { location: LOGIN_PATH },
  });
  if (isNavigation(headers)) {
    // `maxAge: 0` with the SAME attributes the cookie was written with. A
    // deletion whose `path` or `secure` differs from the original leaves the
    // original in place — the browser treats them as different cookies —
    // which is the drift `COOKIE_ATTRIBUTES` exists to stop (`cookies.ts`).
    response.cookies.set(SESSION_COOKIE, '', { ...COOKIE_ATTRIBUTES, maxAge: 0 });
  }
  return response;
}

/**
 * Whether this request is a person arriving on a page, rather than something
 * a page fetched on their behalf.
 *
 * `Sec-Fetch-Dest: document` is a top-level navigation and cannot be forged
 * by script — the browser sets the `Sec-` prefixed headers and refuses to let
 * a page override them. `Sec-Purpose: prefetch` is a navigation the person
 * did not make, and Next itself prefetches `<Link>` targets, so it is
 * excluded too.
 *
 * Absent means "no fetch metadata", which is `curl` and browsers older than
 * 2020. Treated as a navigation: this route only forgets a cookie whose
 * session vpay has already refused, and refusing to do that for a client that
 * sends no metadata would break the feature for the honest caller while the
 * attack needs a browser that does send it.
 */
export function isNavigation(headers?: Headers): boolean {
  if (headers === undefined) {
    return true;
  }
  if ((headers.get('sec-purpose') ?? '').includes('prefetch')) {
    return false;
  }
  const destination = headers.get('sec-fetch-dest');
  return destination === null || destination === 'document';
}
