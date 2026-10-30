# VPA-5b verification: the dashboard offers single sign-on again

_Written 2026-10-30 while the work was done; every number below was measured on
the tree this page was committed with, on `vpa5b-dashboard-sso` over `master` at
`9099ecc` (VPA-5a, #392, on it). Nothing here ran against a deployment, and no
identity provider was reached: see "Not done"._

What was built is in [docs/status/frontend.md](../frontend.md) ("The dashboard
application offers single sign-on (VPA-5b)"), in the app's
[README](../../../frontends/apps/dashboard/README.md) and in
[dashboard-auth/sso.md](../../flows/dashboard-auth/sso.md) ("The dashboard
application (VPA-5b)"). This page is what was compared, what was run, what was
broken on purpose, and what could not be run.

## Backend facts, checked against `master` before relying on them

Each fact was read in the tree (`backends/crates/vpay-api/src/staff/sso/mod.rs`
unless said otherwise), not taken from the plan:

| Fact                                                                                                                                                                                                                                                                                                           | Where it was read                                                                                         |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `GET /dash/v1/staff/sso` is unauthenticated and answers `{"enabled": bool}` (`staff_auth.sso` is written), and is the `404` of an unmounted staff login                                                                                                                                                        | `status`, `SsoStatusResponse`                                                                             |
| `POST /dash/v1/staff/sso/start` takes no body and no session, is rate-limited per address, and answers `{"session": <pending token>, "authorization_url": <url>}`                                                                                                                                              | `start`, `SsoStartResponse`                                                                               |
| `POST /dash/v1/staff/sso/callback` takes a form of `code` and `state` and the pending token in the session header; it answers `{"outcome": "signed_in", "session": <NEW token>}` or `{"outcome": "reauthenticate", "session": <NEW pending>, "authorization_url": <prompt=login>}`; every refusal is one `401` | `callback`, `SsoCallbackRequest`, `SsoCallbackResponse`                                                   |
| the flow is taken exactly once, so a pending token is spent by any callback, and a `state` mismatch burns it                                                                                                                                                                                                   | the order in the module header, step 1                                                                    |
| a `code` or `state` over 2048 characters is refused; empty is refused                                                                                                                                                                                                                                          | `MAX_CALLBACK_PARAM_CHARS`                                                                                |
| `authorization_endpoint` must have the issuer's **scheme**; its origin is not required to equal the issuer's                                                                                                                                                                                                   | `sso/provider.rs`, the `endpoint` closure ("an endpoint's scheme differs from the issuer's")              |
| the issuer may be `http` outside livemode; livemode refuses `http` and stub hosts for `issuer` and `redirect_uri`                                                                                                                                                                                              | `vpay-config/src/config.rs`, `validate_staff_sso` and `a_livemode_sso_block_needs_https_and_no_stub_host` |
| the dashboard does not learn the issuer from any route: `/staff/sso` answers one boolean                                                                                                                                                                                                                       | the same `SsoStatusResponse`, one field                                                                   |

The last two rows are why the dashboard grew a setting. The ticket asks that the
provider's URL be on "the configured issuer's origin", and the dashboard has no
issuer to compare with unless it is told one. `VPAY_DASHBOARD_SSO_ISSUER` is that
setting (optional; unset means the button is not offered).

## What was compared with `f885170^`

`diff` of `git show f885170^:<path>` against the tree, in lines that differ:

| File                                         | Against `f885170^`                                                                                                                                                                                                                                                          |
| -------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `app/api/auth/sso/{start,callback}/route.ts` | 7 and 3 lines: comments only (the middleware is gone)                                                                                                                                                                                                                       |
| `src/server/sso-paths.ts`                    | 16 lines: the callback constant's comment says it is the path of `staff_auth.sso.redirect_uri`                                                                                                                                                                              |
| `src/server/sso.ts`                          | rewritten in place, 245 to 314 lines: its own `__Host-` pending cookie; `signed_in` replaces the session cookie and clears the pending one; the code leg is the gate's, not the route's; `safeAuthorizationUrl` takes the issuer's origin; the credential is `dash-fetch`'s |
| `src/components/sso-sign-in.tsx`             | 85 lines: no `Separator`, a neutral label, the three notices                                                                                                                                                                                                                |
| `src/server/sso.test.ts`, `sso-api.test.ts`  | adapted: 397 and 85 lines differ (the new cookie, the validator's origin, `getSsoStatus`'s three outcomes)                                                                                                                                                                  |

**Changed from the deleted code, each a decision:**

- the pending token was kept in the **session** cookie, so pressing the button
  overwrote a working session and a refused callback had to delete it. It has its
  own cookie, `__Host-vpay_dash_sso_pending`; a failed callback clears only that
  one, so a stray cross-site `GET` to the callback cannot sign a staff member out;
- `signed_in` no longer runs the authorization-code leg inside the route: the
  gate on `/` mints, and already owns every outcome of a mint that did not happen;
- the provider's URL was any `http(s)` URL without userinfo. It is `https`
  (`http` only for `localhost`, `127.0.0.1` and `[::1]`), on the issuer's
  origin, with no userinfo and no fragment;
- `getSsoStatus` failed soft to "not enabled", which hid the button on any outage.
  It answers `enabled`, `disabled` or `unreadable`, and `/login` says SSO could
  not be checked for the third.

**New**: `VPAY_DASHBOARD_SSO_ISSUER` in `config/settings.ts` (an origin,
answered beside `apiBaseUrl` and `publicOrigin`); `SSO_PENDING_COOKIE` and the
restored `SSO_PENDING_MAX_AGE_SECONDS` in `cookies.ts`; the `alternative` slot of
`one-question-steps.tsx` (drawn under the first question only, outside its form);
`SsoFailedNotice`, `SsoUnchecked`, `SsoNotConfigured`; `sso-paths.test.ts`.

## Gates

Run on the tree this page is committed with, in a cloud session with no Docker
and no Rust build (this ticket is frontend only):

| Gate                                                                                          | Result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pnpm --filter @vpay/dashboard test`                                                          | 38 files, **561 passed, 1 skipped** (the skip is `a11y-gate.test.ts`'s conditional `ctx.skip()`, not new); 35 files and 434 passed before this ticket                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `pnpm --filter @vpay/dashboard typecheck`                                                     | clean                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `pnpm --filter @vpay/dashboard lint`                                                          | clean, `--max-warnings 0`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `pnpm --filter @vpay/dashboard build`                                                         | compiled; `/api/auth/sso/start` and `/api/auth/sso/callback` are dynamic (135 B each), `/login` 262 kB first-load; the first attempt **failed**: a default argument on the page's props (`= {}`) is not a `PageProps`, which only `next build` checks; the default was removed and the tests pass `{}`. `.next` (400 MB) was deleted                                                                                                                                                                                                                                                                                         |
| `just lint-web`                                                                               | exit 0                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `just fmt-check-web`                                                                          | "All matched files use Prettier code style!"                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `just test-storybook`                                                                         | checkout 26 passed; dashboard **39 passed** (31 before: eight new stories, four of them under the light theme), axe `color-contrast` on                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `just test-web` (not asked for; run to see the other packages unchanged)                      | exit 0: config 67, tokens 12, `stripe-js` 152, `nodejs` 227, api-client 4, tauri plugin 71, shop 107, checkout 556 + 1 skipped, dashboard 561 + 1 skipped                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `just verify` (xtask built with `CARGO_PROFILE_DEV_DEBUG=0` into `/tmp/xt-5b`, deleted after) | exit 0: **the seventeen gates passed** (`verify-docs` is advisory), including `verify-ui`, `verify-status` (2 unimplemented items, all declared), `verify-links` (3195 links in 533 tracked files; it first **failed** on three links to this page, which `verify-links` resolves against tracked files, so the page was staged before the rerun) and `verify-doc-counts` (9 counts agree: the verification directory went from 132 to 133 pages and the sentence, the list entry and the history of the count were updated in the same change). The xtask was built into `/tmp/xt-5b` and that directory deleted afterwards |

## Sabotage

Every guard the plan lists, and the checks under the redirect, was broken and
the named test run. **31 mutations: all caught, none survived.** The harness is the scratchpad
script `sab5b/run.py`: it applies one mutation to one file, runs the named test
files under vitest with the JSON reporter, expects a non-zero exit and at least
one failing case, and restores the file with `git checkout -- <file>`; **the tests
were committed first** (`a893c4f`), so a restore cannot lose work. The plan's
seven targets are `START-1`, `CB-1`, `HDR-1`/`HDR-2`, `URL-1`/`URL-2`,
`COOKIE-1`..`COOKIE-3`, `SESS-1`..`SESS-3` and `BTN-1`..`BTN-3`.

| ID       | Mutation                                                               | Changed                               | Run against                          | Failing | First failing case                                                                                                                                      |
| -------- | ---------------------------------------------------------------------- | ------------------------------------- | ------------------------------------ | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| START-1  | origin check removed from start                                        | src/server/sso.ts                     | sso.test.ts                          | 3       | POST /api/auth/sso/start refuses a foreign origin before vpay is asked for anything                                                                     |
| CB-1     | vpay called despite error=                                             | src/server/sso.ts                     | sso.test.ts                          | 2       | GET /api/auth/sso/callback does not call vpay when the provider sent error beside a code and state                                                      |
| HDR-1    | no-store dropped                                                       | src/server/sso.ts                     | sso.test.ts                          | 17      | POST /api/auth/sso/start carries no-store and no-referrer on every response                                                                             |
| HDR-2    | no-referrer dropped                                                    | src/server/sso.ts                     | sso.test.ts                          | 17      | POST /api/auth/sso/start carries no-store and no-referrer on every response                                                                             |
| URL-1    | start follows the provider URL unvalidated                             | src/server/sso.ts                     | sso.test.ts                          | 7       | POST /api/auth/sso/start refuses an authorization_url of "javascript:alert(1)" and keeps the pending token out of the cookie                            |
| URL-2    | reauthenticate follows the provider URL unvalidated                    | src/server/sso.ts                     | sso.test.ts                          | 3       | GET /api/auth/sso/callback refuses a reauthenticate whose url is "javascript:alert(1)", clearing the pending cookie                                     |
| COOKIE-1 | pending cookie not __Host-                                             | src/server/cookies.ts                 | cookies.test.ts, sso.test.ts         | 10      | the cookies are __Host- cookies names the single sign-on pending cookie with the prefix too, and apart from the session's                               |
| COOKIE-2 | pending cookie not httpOnly (set)                                      | src/server/sso.ts                     | sso.test.ts                          | 1       | POST /api/auth/sso/start sets the pending token as a __Host- httpOnly cookie for ten minutes and redirects 303 to the IdP                               |
| COOKIE-3 | shared attributes lose httpOnly                                        | src/server/cookies.ts                 | cookies.test.ts, sso.test.ts         | 1       | the session cookie is httpOnly — no script on this origin may read a payments credential                                                                |
| SESS-1   | session not replaced on signed_in                                      | src/server/sso.ts                     | sso.test.ts                          | 1       | GET /api/auth/sso/callback replaces whatever session the browser held with the NEW token, clears the pending cookie, and lands on /                     |
| SESS-2   | the pending token becomes the session (reused)                         | src/server/sso.ts                     | sso.test.ts                          | 1       | GET /api/auth/sso/callback replaces whatever session the browser held with the NEW token, clears the pending cookie, and lands on /                     |
| BTN-1    | button shown when enabled:false                                        | app/login/page.tsx                    | page.test.tsx                        | 1       | single sign-on on the sign-in page does not offer it when vpay says enabled: false, and says nothing about it                                           |
| BTN-2    | failed read hides the button silently                                  | app/login/page.tsx                    | page.test.tsx, page-timeout.test.tsx | 2       | an SSO status read that times out after discovery answered says SSO could not be checked and keeps the email question — never hides the button silently |
| BTN-3    | failed read treated as enabled                                         | app/login/page.tsx                    | page.test.tsx                        | 1       | single sign-on on the sign-in page says SSO could not be checked — and does not hide the button silently — when the read fails                          |
| BTN-4    | no ?sso=failed message                                                 | app/login/page.tsx                    | page.test.tsx                        | 1       | single sign-on on the sign-in page shows the generic failure after ?sso=failed, with no reason                                                          |
| API-1    | enabled:false read as unreadable... inverted (failed read => disabled) | src/server/api.ts                     | sso-api.test.ts                      | 8       | getSsoStatus reads a 404 as unreadable, carrying the failure — never as disabled                                                                        |
| EXTRA-1  | callback accepts a duplicated code                                     | src/server/sso.ts                     | sso.test.ts                          | 2       | GET /api/auth/sso/callback with a duplicated code: does not call vpay, clears the pending cookie, answers the generic failure                           |
| EXTRA-2  | issuer origin not compared                                             | src/server/sso.ts                     | sso.test.ts                          | 6       | safeAuthorizationUrl refuses another host                                                                                                               |
| EXTRA-3  | http allowed for any host                                              | src/server/sso.ts                     | sso.test.ts                          | 1       | safeAuthorizationUrl admits plain http only for a loopback issuer (a development stack)                                                                 |
| EXTRA-4  | userinfo allowed                                                       | src/server/sso.ts                     | sso.test.ts                          | 4       | safeAuthorizationUrl refuses userinfo with a password                                                                                                   |
| EXTRA-5  | fragment allowed                                                       | src/server/sso.ts                     | sso.test.ts                          | 3       | safeAuthorizationUrl refuses a fragment                                                                                                                 |
| EXTRA-6  | unknown issuer origin still redirects                                  | src/server/sso.ts                     | sso.test.ts                          | 1       | POST /api/auth/sso/start refuses, without asking vpay, when it does not know the provider's origin                                                      |
| EXTRA-7  | failed callback also deletes the session cookie                        | src/server/sso.ts                     | sso.test.ts                          | 16      | GET /api/auth/sso/callback replaces whatever session the browser held with the NEW token, clears the pending cookie, and lands on /                     |
| EXTRA-8  | start sends a session header                                           | src/server/api.ts                     | sso-api.test.ts, sso.test.ts         | 2       | ssoStart posts an empty form with no credential and returns both values                                                                                 |
| EXTRA-9  | pending lifetime 12h                                                   | src/server/cookies.ts                 | cookies.test.ts, sso.test.ts         | 2       | the cookies are __Host- cookies lets the pending cookie live ten minutes — vpay's own bound on a flow                                                   |
| EXTRA-10 | callback route file exports POST                                       | app/api/auth/sso/callback/route.ts    | sso-paths.test.ts                    | 1       | the SSO paths callback answers GET only — it is the identity provider's redirect                                                                        |
| EXTRA-11 | start route exports GET                                                | app/api/auth/sso/start/route.ts       | sso-paths.test.ts                    | 1       | the SSO paths start answers POST only — a GET that sets a cookie and redirects is a link a chat can fire                                                |
| EXTRA-12 | SSO path constant drifts from the route                                | src/server/sso-paths.ts               | sso-paths.test.ts                    | 4       | the SSO paths callback names a route file that exists                                                                                                   |
| EXTRA-13 | button inside the email form                                           | src/components/one-question-steps.tsx | one-question-steps.test.tsx          | 2       | an alternative way in, beside the first question is drawn under the email question and outside its form                                                 |
| EXTRA-14 | issuer origin keeps the path                                           | src/config/settings.ts                | settings.test.ts                     | 2       | a complete configuration reduces the SSO issuer to its origin, and reads an unset or unusable one as unknown                                            |
| SESS-3   | pending cookie not cleared on signed_in                                | src/server/sso.ts                     | sso.test.ts                          | 1       | GET /api/auth/sso/callback replaces whatever session the browser held with the NEW token, clears the pending cookie, and lands on /                     |

Two things the run said about itself. One mutation (`SESS-3`) first matched two
places in `sso.ts` and was reported as a bad mutation, not a survivor; it was
made specific and rerun, and the count above includes that rerun. And the run
found **no gap in the tests**: every mutation failed a case that was written
before it ran. That is a smaller claim than the VPA-5a page's, which found nine;
it is not evidence that none exist, only that the listed guards are held. The
mutations are mine, so they test what I thought of.

## Not done

- **No provider has been reached.** The routes are driven against a stubbed
  `fetch`, the page in jsdom, and the screens in Chromium (contrast, both
  themes). Nothing drove a browser through Keycloak, and no end-to-end case runs
  (VPA-5c).
- **The Helm chart and `compose.e2e.yml` do not set `VPAY_DASHBOARD_SSO_ISSUER`**,
  nor the three VPA-5a client values; those are VPA-5c's.
- **A provider whose authorization endpoint is on another origin than its issuer
  is refused**, though vpay accepts it (it checks the scheme only). Keycloak is
  not such a provider.
- **A session already in the browser is replaced on `signed_in`, not revoked at
  vpay**; it expires on its own bounds.
- **`http` is allowed for a loopback provider.** The ticket says `https`; a
  development stack's Keycloak is `http://localhost`, and vpay itself allows it
  outside livemode. A non-loopback `http` provider is refused even when vpay
  would serve it.
- **A failed callback clears the pending cookie and leaves the session cookie**,
  where the ticket's wording says "clear cookies". Clearing the session on a
  stray `GET` would be a sign-out anyone could fire.
- **No Rust ran** beyond the `just verify` xtask; `helm-check` and
  `docs-check-citations` were not run.
