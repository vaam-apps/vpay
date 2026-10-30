/**
 * The two cookies this app sets on its own origin, and the attributes that
 * are not negotiable.
 *
 * ADR-0017 decision 2: the staff session token is 256 bits from the OS
 * CSPRNG, minted by vpay, and **held by the dashboard app** in an httpOnly,
 * Secure, SameSite=Lax cookie. vpay itself never sets a cookie and never
 * reads one — the app presents the token in `X-Vpay-Staff-Session`,
 * server-side.
 *
 * # Why each attribute, and what breaks without it
 *
 * `httpOnly` — the token is a bearer credential for a payments dashboard.
 * Without it any script on this origin reads it, and the whole point of
 * keeping the `/dash/v1` access token server-side (decision 4) is undone by
 * the session token that fetches it.
 *
 * `secure` — unconditionally, in every deployment. Browsers treat
 * `http://localhost` as a secure context, so this holds for the compose
 * stack too; there is no environment branch here and AGENTS.md forbids one.
 * A cookie that drops `Secure` outside production is a cookie that travels
 * in clear text the day somebody points a staging hostname at the image.
 *
 * `sameSite: 'lax'` — a cross-site POST must not carry the session. `lax`
 * rather than `strict` because a staff member following a link to
 * `/payments/pi_…` from a chat window should arrive signed in; that is a
 * top-level GET, which `lax` sends and `strict` does not.
 *
 * `path: '/'` — every page in this app is behind the session.
 */

/**
 * # The `__Host-` prefix
 *
 * Both names carry it (VPA-5a review). A browser stores a `__Host-` cookie
 * only if it is `Secure`, was set over a secure connection, has `Path=/` and
 * has **no `Domain`** — which is exactly what {@link COOKIE_ATTRIBUTES} already
 * said, so the prefix costs nothing and gains the one thing the attributes
 * cannot: it stops a sibling subdomain, or a plain-`http` response on the
 * same host, from planting or overwriting the session cookie.
 *
 * Where that bites:
 *
 * * `http://localhost` (`next dev`, the compose stacks, Cypress): Chrome and
 *   Firefox treat it as a secure context, so a `Secure` and a `__Host-`
 *   cookie are both stored. Safari has historically refused a `Secure` cookie
 *   over plain `http`, localhost included; this was not tried there, and the
 *   cookie was already `Secure` before the prefix.
 * * Plain `http` on any other host: **no browser stores the cookie**. The
 *   sign-in action succeeds and redirects, the next request carries nothing,
 *   and `/` sends the browser back to `/login`. The prefix does not change
 *   that — `secure: true` was already unconditional — but it makes the
 *   failure certain rather than browser-dependent. Put the dashboard behind
 *   TLS (`docs/flows/dashboard-auth/human-login.md`, "The cookie needs a
 *   secure origin").
 * * A rename signs out every browser holding the old name: the old cookie is
 *   never read again and expires on its own. Nothing is lost server-side; the
 *   row is revoked by its own expiry.
 */

/** The staff session token vpay minted. */
export const SESSION_COOKIE = "__Host-vpay_dash_session";

/**
 * The **sealed** TOTP secret, between the password leg and the first code.
 *
 * `POST /dash/v1/staff/login` answers a first sign-in with `enrolment`: the
 * secret sealed with AES-256-GCM under a deployment key this app does not
 * have, to be handed back to `/staff/totp` with the first code. It cannot be
 * committed at login — a secret written before the person proved they can
 * generate a code from it locks them out permanently — so somebody has to
 * hold it across two requests, and the app is the only party in the flow
 * that can.
 *
 * httpOnly like the session: the browser has no use for it. The *plaintext*
 * secret is on screen, because the staff member has to scan or type it —
 * that is enrolment, not a leak — but the sealed blob is this app's own
 * business.
 */
export const ENROLMENT_COOKIE = "__Host-vpay_dash_enrolment";

/**
 * The attributes every cookie this app sets carries.
 *
 * One object, exported, and asserted on by `cookies.test.ts` rather than
 * spelled at each call site — three call sites drifting apart is exactly how
 * one of them loses `httpOnly`.
 */
export const COOKIE_ATTRIBUTES = {
  httpOnly: true,
  secure: true,
  sameSite: "lax",
  path: "/",
} as const;

/**
 * How long a session cookie may live in the browser.
 *
 * Twelve hours, matching `staff_sessions`' **absolute** bound (ADR-0017
 * decision 2), never the 30-minute idle one: the idle bound moves on every
 * accepted request and a cookie cannot move with it. The cookie expiring
 * first would sign a working session out; the cookie outliving the row is
 * harmless, because the row is what is checked and an expired one is a
 * `401` this app answers by clearing the cookie and showing the login form.
 */
export const SESSION_MAX_AGE_SECONDS = 12 * 60 * 60;

/**
 * How long the enrolment cookie may live.
 *
 * Ten minutes: it exists to bridge two consecutive requests — a QR code
 * scanned and a code typed. A blob that outlived the login attempt would be
 * offered back on a *later* sign-in, where `/staff/totp` ignores it for an
 * enrolled account and refuses it for one enrolled meanwhile.
 */
export const ENROLMENT_MAX_AGE_SECONDS = 10 * 60;
