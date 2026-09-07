/**
 * The response that forgets a dead session cookie.
 *
 * Driven directly, because the branch it exists for is the one no other suite
 * reaches: `dashboard.cy.ts` signs in, looks around and signs out, and every
 * one of those paths either carries no cookie or is redirected without one
 * being cleared. The refusal path — a cookie vpay will not accept — was
 * reachable only through a browser, and was answering `500`.
 */
import { describe, expect, it } from 'vitest';

import { COOKIE_ATTRIBUTES, SESSION_COOKIE } from './cookies';
import { LOGIN_PATH } from './session';
import { signedOutResponse } from './signed-out';

/** The `Location` this response carries. */
function response(): string {
  return signedOutResponse().headers.get('location') ?? '';
}

/** The `Set-Cookie` this response carries. */
function setCookie(response: Response): string {
  const header = response.headers.get('set-cookie');
  expect(header, 'the response must carry a Set-Cookie').not.toBeNull();
  return header ?? '';
}

describe('the signed-out response', () => {
  it('sends the browser to the sign-in form', () => {
    const response = signedOutResponse();
    expect(response.status).toBe(303);
    expect(response.headers.get('location')).toBe(LOGIN_PATH);
  });

  it('names the target relatively, never a host', () => {
    // `NextResponse.redirect` demands an absolute URL and the only one
    // available in a Route Handler is built from `request.url` — which inside
    // the container is `http://<container id>:3000/…`. Measured: a browser
    // followed exactly that into a DNS failure. A relative reference also
    // cannot be an open redirect.
    const location = response();
    expect(location.startsWith('/'), `location was ${location}`).toBe(true);
    expect(location).not.toMatch(/^https?:/);
  });

  it('deletes the session cookie rather than leaving it to expire', () => {
    // A dead cookie left in place is what turned one refusal into a 500 on
    // every request after it.
    const cookie = setCookie(signedOutResponse());
    expect(cookie).toContain(`${SESSION_COOKIE}=`);
    expect(cookie).toMatch(/Max-Age=0/i);
  });

  it('deletes it with the attributes it was written with', () => {
    // A deletion whose Path or Secure differs from the original leaves the
    // original in place: the browser treats them as different cookies.
    expect(COOKIE_ATTRIBUTES.httpOnly, 'the attributes under test').toBe(true);
    const cookie = setCookie(signedOutResponse());
    expect(cookie).toMatch(/HttpOnly/i);
    expect(cookie).toMatch(/Secure/i);
    expect(cookie).toMatch(/Path=\//i);
    expect(cookie).toMatch(/SameSite=Lax/i);
  });


});
