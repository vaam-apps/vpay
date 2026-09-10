/**
 * The origin check in front of every server action (issue #88 item 4).
 *
 * # What a server action is, and why it needs one
 *
 * Every export of a `'use server'` file is a callable endpoint: Next
 * registers an action id for it and a `POST` from **anywhere** reaches it.
 * The four things a staff member can do — sign in, present a code, replace a
 * password, sign out — are therefore four unauthenticated `POST` targets on
 * this app's origin, and the session cookie is `SameSite=Lax`, which is not
 * `Strict`: a top-level form submission from another site carries it.
 *
 * Next has its own check for this and it is not the one this deployment
 * wants. `next/dist/server/app-render/action-handler` compares the `Origin`
 * header against the host, and the host it uses is **`x-forwarded-host` when
 * present**, falling back to `Host`. Behind a proxy that does not strip
 * incoming forwarding headers — which is the default for several — a caller
 * who sets `X-Forwarded-Host: evil.example` and `Origin: https://evil.example`
 * makes the two agree and the check passes. It compares two caller-supplied
 * values with each other.
 *
 * # What this compares instead
 *
 * The configured public origin, which no caller can influence:
 * `VPAY_DASHBOARD_PUBLIC_ORIGIN`. **`X-Forwarded-Host` is never read, by
 * anything in this file**, and `originIsAllowed`'s signature is the guard —
 * it takes the two headers it is allowed to see and there is no third
 * parameter for a forwarding header to arrive through.
 *
 * # The fallback, and why it is not the same thing
 *
 * A deployment that has **not** set `VPAY_DASHBOARD_PUBLIC_ORIGIN` gets the
 * `Host` header compared instead — still never `X-Forwarded-Host`, so it is
 * strictly stronger than Next's own check, and weaker than the configured
 * one in exactly one way that is stated rather than hidden: behind a reverse
 * proxy `Host` is whatever the proxy sends upstream, which is often the
 * internal service name, and it will not equal the public `Origin` a browser
 * sends. **Such a deployment must set the variable**, and the symptom if it
 * does not is that every action is refused with the sentence below rather
 * than that the check quietly stops checking.
 *
 * The variable is optional rather than required for one reason and it is not
 * a good one on its own: making it required would take the sign-in down for
 * every deployment that has not been reconfigured, including this
 * repository's own compose stacks, which another change is editing
 * concurrently. It is recorded in `docs/status.md` as the follow-up.
 *
 * # An absent `Origin` is refused
 *
 * Fail closed. Every browser has sent `Origin` on a cross-origin `POST` for
 * years and on same-origin form posts since 2020; a `POST` with none is
 * `curl`, a very old client, or something stripping it — and the whole point
 * of this file is that a caller must not be able to remove the check by
 * removing a header.
 */
import { headers } from 'next/headers';

import { dashboardConfig } from '../config/runtime';
import type { FormState } from '../form-state';

/** The header naming the origin a request was issued from. */
export const ORIGIN_HEADER = 'origin';
/** The host a request was addressed to, as the client wrote it. */
export const HOST_HEADER = 'host';

/**
 * The sentence a refused action answers with.
 *
 * It names no header and no configured value: a caller who reached this is
 * either somebody's browser on a misconfigured deployment, in which case the
 * operator needs the container log and not this string, or an attacker, in
 * which case telling them which origin *would* work is telling them what to
 * forge.
 */
export const CROSS_ORIGIN_REFUSED =
  'This request did not come from this dashboard. Reload the page and try again.';

/**
 * `scheme://host[:port]`, lower-cased, with no trailing slash and no path.
 *
 * Both sides of every comparison go through this, so a configured
 * `https://Dash.Example/` and a browser's `https://dash.example` are one
 * value. Anything that does not parse as an absolute URL is `null`, which
 * compares equal to nothing — an `Origin: null` (a sandboxed iframe, a
 * `file://` document, some redirects) is therefore refused rather than
 * treated as absent.
 */
export function normaliseOrigin(value: string | null | undefined): string | null {
  if (typeof value !== 'string' || value.trim().length === 0) {
    return null;
  }
  try {
    const url = new URL(value.trim());
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
      return null;
    }
    return url.origin.toLowerCase();
  } catch {
    return null;
  }
}

/**
 * Whether a request carrying `origin`, addressed to `host`, is one this
 * dashboard issued.
 *
 * A pure function over three strings so that the decision is a unit test
 * rather than something only a browser can exercise — `session.ts`'s split,
 * for `session.ts`'s reason.
 *
 * **There is deliberately no parameter for `X-Forwarded-Host`.** That is the
 * whole of issue #88 item 4: a value the caller can set must not be one side
 * of a check against another value the caller can set.
 *
 * @param origin the `Origin` header, verbatim
 * @param publicOrigin `VPAY_DASHBOARD_PUBLIC_ORIGIN`, or `null` if unset
 * @param host the `Host` header, verbatim — used **only** when
 *   `publicOrigin` is `null`, and never `X-Forwarded-Host`
 */
export function originIsAllowed(
  origin: string | null,
  publicOrigin: string | null,
  host: string | null,
): boolean {
  const seen = normaliseOrigin(origin);
  if (seen === null) {
    return false;
  }

  const configured = normaliseOrigin(publicOrigin);
  if (configured !== null) {
    return seen === configured;
  }

  // No configured origin: compare the authority the client addressed. `Host`
  // carries no scheme, so it is compared as an authority rather than as an
  // origin — which means this fallback cannot distinguish `http` from
  // `https` on one host. Stated rather than papered over; the configured
  // value is what a deployment that cares should set.
  if (typeof host !== 'string' || host.trim().length === 0) {
    return false;
  }
  return seen.slice(seen.indexOf('//') + 2) === host.trim().toLowerCase();
}

/**
 * `null` if this request may proceed, or the refusal a form renders.
 *
 * The Next-bound half: it reads `headers()`, which only works inside a
 * request. Every export of `server/actions.ts` opens with it.
 *
 * `requestId: null` because there is no vpay response behind this — the
 * refusal is this app's own, decided before anything was sent.
 */
export async function originRefusal(): Promise<FormState | null> {
  const store = await headers();
  const { config } = dashboardConfig();
  const allowed = originIsAllowed(
    store.get(ORIGIN_HEADER),
    config?.publicOrigin ?? null,
    store.get(HOST_HEADER),
  );
  if (allowed) {
    return null;
  }
  // eslint-disable-next-line no-console -- an operator whose proxy rewrites Host needs to see this, and it carries no credential: the Origin header of a request that was refused before anything was read from it.
  console.warn(
    `[vpay-dashboard] a server action was refused: Origin ${store.get(ORIGIN_HEADER) ?? '(absent)'} is not this deployment's public origin. Set VPAY_DASHBOARD_PUBLIC_ORIGIN if this app is behind a proxy.`,
  );
  return { error: CROSS_ORIGIN_REFUSED, requestId: null };
}
