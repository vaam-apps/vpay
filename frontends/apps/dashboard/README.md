# `@vpay/dashboard`

The staff dashboard. What it is, and what it is not, is
[`docs/flows/dashboard.md`](../../../docs/flows/dashboard.md)'s job — this
file is the frontend's own, narrower one: how a screen in this app is built,
and where each decision lives.

## What this app is, in one paragraph

**The OAuth client**
([ADR-0017](../../../docs/adr/0017-staff-authentication.md) decision 4). It
holds the staff session in an httpOnly, Secure, SameSite=Lax cookie on its own
origin, presents it to vpay in `X-Vpay-Staff-Session` server-side, and runs the
authorization-code leg — request, follow the `302`, exchange — inside its own
process. A browser never sees a code, a verifier or a token, and the `/dash/v1`
access token is read back out of the `staff_sessions` row on every render
rather than kept here, which is what makes signing out a revocation.

## The pages

| Route             | What it does                                                             |
| ----------------- | ------------------------------------------------------------------------ |
| `/`               | Redirects to `/payments` or `/login`. Nothing of its own                 |
| `/login`          | Work email + argon2id password — leg one                                 |
| `/login/totp`     | Six digits. A **first** sign-in also renders the enrolment QR and secret |
| `/login/password` | Replaces the printed one-time password. Not optional — see below         |
| `/payments`       | The bound merchant's intents; status and date filters, cursor paging     |
| `/payments/{id}`  | Intent, charge, refunds, last error, event timeline                      |

Every one of them is `dynamic = 'force-dynamic'`, and has to be: they all read
cookies.

### There is no route at `redirect_uri`, and nothing is missing

`dashboard_client.redirect_uris` names `http://localhost:3000/dash/v1/callback`
and **no browser ever visits it.** Decision 4 has this app's own server follow
the `302`, so that string is an identifier the two OAuth legs must spell
identically — matched byte for byte by
`ClientRegistration::allows_redirect_uri`, no prefix, no wildcard — and not a
page. A route there would be a page nobody can reach.

It is said here because a reader who knows OAuth will look for it, not find it,
and reasonably conclude the flow is unfinished.

### `/login/password` is a step, not a nag

`vpay-server staff add` sets `password_change_required`, and ADR-0017 decision
1 refuses **every** authenticated route to a session carrying it,
`/oauth/authorize` included. So no `/dash/v1` token can exist until it is done,
and this page is the only one such a session can reach. There is no
"current password" field: the session has already presented both factors, and
re-checking a password this endpoint then overwrites would be a second copy of
that check in the wrong layer.

## The BFF, which is a new browser-reachable surface

Two `GET` route handlers, added 2026-09-11:

| Route                            | What it does                                       |
| -------------------------------- | -------------------------------------------------- |
| `/api/dash/payment_intents`      | One page of this merchant's intents, as JSON       |
| `/api/dash/payment_intents/{id}` | One payment's intent, charge, refunds and timeline |

**Read the paragraph before reaching for these.** Until they existed nothing in
this app was reachable from a browser except a page and four Server Actions,
and every `/dash/v1` read happened inside a render. They authenticate on the
same httpOnly session cookie and proxy to `/dash/v1` with the token read out of
the `staff_sessions` row on that request — the bearer never leaves this
process — but they are an authenticated surface a script on this origin can
call, which the app did not have before.

**Nothing calls them.** No page, no component, no test but their own. They
exist so that a client-side data layer has a transport when one is written, and
**whether this app should have such a surface at all is the maintainer's
decision, not this code's** — it is RD5 in
[the Refine plan](../../../docs/plans/exp55-refine-seam-bff-notes/refine-plan.md)
§8, and it reverses a stated property of the app's security model. Deleting the
two files under `app/api/` and `src/server/bff.ts` breaks nothing else.

What they do about it, each written as a mutation in `src/server/bff.test.ts`:

- a request with **no session cookie** is refused and reaches vpay **not at
  all** — the test asserts the stub `fetch` was never called, because a `401`
  answered _after_ an upstream round trip is a surface anyone can use to make
  this server open connections;
- a request this dashboard did not issue is refused by `csrf.ts`'s own
  `originIsAllowed`, plus `Sec-Fetch-Site: same-origin`, which is what a
  browser sends instead of an `Origin` on a same-origin `GET`;
- the bearer token appears in no response header and no response body, asserted
  against the serialised response while the stubbed vpay echoes the token into
  a header, into the envelope's `url` and into an extra top-level field;
- no caller-supplied merchant id, audience or scope is forwarded, because the
  upstream query string is built by `apiQueryString` from the five parameters
  `queryFrom` reads and nothing else is ever looked at;
- a repeated parameter takes its **first** value, the same as the page does.
  `Object.fromEntries(searchParams.entries())` keeps the last, which would
  have made one URL mean two different pages depending on which surface read
  it.

Only `GET` is exported and **no write can reach the file** — the same shape
`/dash/v1` has, where `dash::required_scope` refuses a non-`GET` before the
router matches.

This line went on ~~"so Next answers `405` to everything else"~~ until
**2026-09-11**, when the exp55 security review read Next 16.3.4's
`app-route/helpers/auto-implement-methods` instead of assuming it. Next
auto-implements two methods, so the reachable set is three, not one:

- **`HEAD` is bound to the `GET` handler itself** (`methods.HEAD =
handlers.GET`), body discarded — so a `HEAD` costs the same session read,
  the same possible token mint and the same upstream call, and answers the
  status and the headers. Consistent with `/dash/v1`, which admits `GET` and
  `HEAD`.
- **`OPTIONS` answers `204` with `Allow: GET, HEAD, OPTIONS` before the
  handler runs at all**, so before the origin check and before the cookie is
  read. It carries no data, but it tells an unauthenticated caller that the
  route exists where a `404` would not. Suppressing it needs a middleware and
  this app has none, so it is recorded rather than fixed.

Everything that is not one of those three is Next's `405`.

Two more things the same review corrected, both of them claims rather than
code. The third bullet above is pinned by the **detail** read's case and not
the list's: `src/dash/provider.ts`'s `getList` has already rebuilt
`{ data, hasMore, cursor }` field by field, so serving the upstream document
verbatim leaves the list case green and only the detail case red. And
`Sec-Fetch-Site` is not merely "a pre-2020 browser" question — **Safari has
sent it only since 16.4 (March 2023)**, so this surface answers `403` to every
older WebKit. That costs nothing while nothing calls it, and is a decision for
whatever eventually does.

## How the code is laid out

| Directory               | What lives there                                                                          |
| ----------------------- | ----------------------------------------------------------------------------------------- |
| `app/`                  | Routes only. Composition, a redirect, and a fetch — no logic worth testing alone          |
| `app/api/dash/`         | The BFF's two route handlers. Four lines each; `src/server/bff.ts` is the substance       |
| `src/components/`       | Every rendered component. Pure props in, markup out; no `fetch`, no `next/headers`        |
| `src/server/`           | Everything that touches vpay, cookies, or PKCE. Imported only by `app/` and itself        |
| `src/dash/`             | The `/dash/v1` read seam — `getList` / `getOne`, over `readDash`. No framework            |
| `src/config/`           | `settings.ts` decides what a configuration means; `runtime.ts` reads the environment once |
| `src/format.ts`         | Money, instants, the em dash. The numbers a reader is entitled to have right              |
| `src/payments-query.ts` | The URL's filter vocabulary ↔ the API's, and the two paging links                         |
| `src/testing/`          | Fixtures. Imported by tests and by nothing under `app/`                                   |

The split between `src/server/` and `src/components/` is the one that matters:
a component that fetched would be a component no test could render, and a
`fetch` inside a component is how a page ends up unable to say _why_ it is
empty.

`src/server/actions.ts` is a `'use server'` file, which may export **only async
functions** — a constant or a route path exported from there is a build error,
not a lint warning. That is why `FormState` and `NO_ERROR` live in
`src/form-state.ts` and the route paths in `src/server/session.ts`.

## The rules this app is held to

### Zero `className` strings

Not one, under `app/` or `src/`, outside the tests. Every visual decision comes
from `@vpay/ui` — `PageShell`, `Stack`, `Heading`, `Text`, `Table`, `Field`,
`Input`, `Button`, `Alert`, `Select`, `List`, `StatusBadge`. `just verify-ui`
is the gate for the parts of that a grep can see (`docs/plans/2026-09-07-ui-revamp.md`
§3, §7); the rest is a reviewer's job.

`data-theme` is `bumblebee`, and `src/layout.test.tsx` pins it:
`frontends/packages/ui/src/styles.css` compiles that theme and no other, so a
`data-theme` that says anything else renders the page **completely unthemed**
in a real browser with no error anywhere.

### Status colour comes from `@vpay/tokens`, always

`StatusBadge` takes its tone from `statusTone`, never from a local map, so a
status cannot read one colour in the list and another on the detail page. A
status this build cannot name renders as **text**, not as a coloured pill — a
green badge on an unfamiliar value is a claim.

### The nav rule, as a gate rather than a comment

> "The navigation is only ever allowed to link to slices that exist. A menu
> entry for a page nobody wrote is the same lie as an empty table."
> — `docs/flows/dashboard.md`

`NAV_LINKS` in [`src/nav.tsx`](src/nav.tsx) is an exported constant, and
[`src/layout.test.tsx`](src/layout.test.tsx) checks it **three ways**: every
entry resolves to an `app/**/page.tsx` on disk; every internal `href` in the
_rendered_ markup does too; and the nav renders what it declares. The pair of
the first two is the point — the constant catches a link rendered only in a
branch no test exercises, the markup catches a link written straight into the
JSX. Adding `{ href: '/webhooks' }` fails two of the three. Measured.

There is no "Sign out" in the nav, and that is the same rule: this layout
renders on `/login` too. Who is signed in and the way out are in
`SignedInBar`, which only the pages behind the gate render.

### Configuration fails closed

Four environment variables — `VPAY_DASH_API`, `VPAY_DASHBOARD_CLIENT_ID`,
`VPAY_DASHBOARD_REDIRECT_URI`, `VPAY_DASHBOARD_SCOPE` — read **once at start**
through bracket notation, because Next inlines a statically-written
`process.env.FOO` at build time and a value baked into the image is not a value
an operator can change.

A missing one is **not defaulted**. `/login` renders the variable names an
operator has to set and offers no form. That is the opposite of
`frontends/apps/checkout`'s runtime config, which defaults everything and
warns, and deliberately so: a payment page with no branding still takes a
payment; a dashboard with no client registration is a login form that cannot
log anybody in, and rendering one invites somebody to retype a password.

The **merchant** is not configured here at all — it comes from
`GET /dash/v1/staff/session`, and is rendered beside the staff member's address
on every signed-in page, because an operator looking at an empty list has to be
able to tell "this merchant has no payments" from "I am looking at the wrong
merchant".

## Two things the pages do not show, and why absence is the honest shape

**No "Rail" column on the list.** `GET /dash/v1/payment_intents` returns no
charge, so the only rail-shaped value in that response is
`payment_method_types` — the rails an intent _may_ be confirmed against. The
column is headed **Methods**. A "Rail" heading over it would be wrong for every
intent that offers two and was taken by one, and wrong _invisibly_. The detail
page has a real `Rail`, from `charge.provider_code`.

**The masked payer is an em dash, and it is a real `null`.**
`charges.payer_ref_masked` is never written by anything (`docs/status.md`), so
the detail page renders it **from the column** and never derives it — the only
other value that could produce a mask is the payer's unmasked phone number, and
reading that into a staff surface to make a row look populated is the trade
this refuses. The row exists rather than being omitted so the value appears the
day the column is written. `payment-detail.test.tsx` pins both directions, and
`dashboard.cy.ts` walks the list until it finds a payment that _has_ a charge
before asserting it — a dash on an intent nobody confirmed proves nothing.

The **list** has no payer column at all, which is the stronger form of the same
point: there is no field there to be null.

## Testing this app

```bash
pnpm --filter @vpay/dashboard test        # 21 files, 214 tests, 0 skipped
pnpm --filter @vpay/dashboard typecheck
pnpm --filter @vpay/dashboard lint
pnpm --filter @vpay/dashboard build       # also proves the compiled CSS
                                          # actually carries these classes
```

`vitest.config.ts` sets `globals: false`, so Testing Library does **not**
register its own cleanup — `vitest.setup.ts` calls `afterEach(cleanup)`
explicitly. Without it every render in a file accumulates in one
`document.body` and `getByRole` finds the previous test's copy of a control.

[`src/a11y.test.tsx`](src/a11y.test.tsx) runs `axe-core@4.13.0`'s structural
rules over the real rendered `<body>` of the layout and over **every** screen
including its error and pending states. It exists because an earlier draft
dropped the `<main>` landmark and every other gate stayed green: `region` went
0 → 1 violation and nothing said so. Contrast is **not** checked and cannot be
— jsdom computes no paint.

### The decisive cases

Each of these was applied to the tree, the suite run, and the mutation
reverted:

| Mutation                                                                     | Fails                                                                           |
| ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| `COOKIE_ATTRIBUTES.httpOnly` → `false`                                       | `src/server/cookies.test.ts`                                                    |
| exchange a _fresh_ PKCE verifier rather than the one the challenge came from | `src/server/oauth.test.ts`                                                      |
| `NAV_LINKS` gains a page nobody wrote                                        | `src/layout.test.tsx`, twice                                                    |
| `pageCursors` reads `has_more` the same way in both paging directions        | `src/dash/provider.test.ts`, and `src/payments-query.test.ts` twice             |
| the BFF reads the session cookie **after** the upstream session read         | `src/server/bff.test.ts`, twice — vpay is contacted for a caller with no cookie |
| `apiIsSameOrigin` drops its `originIsAllowed` call                           | `src/server/bff.test.ts`, twice                                                 |
| the BFF serves the parsed upstream document instead of the named fields      | `src/server/bff.test.ts`, twice                                                 |

`oauth.test.ts`'s stub echoes the challenge into the code it returns, so the
assertion is that the exchange presents the verifier whose `S256` **is** the
challenge this call sent — not merely that some verifier was present. A
refactor that generated a second pair for the exchange fails it.

### End to end

`frontends/tests/e2e/cypress/e2e/dashboard.cy.ts` signs a staff member in
through the real OP against the real compose stack and computes its TOTP codes
from the secret **the enrolment screen displayed**. `just demo-staff` creates
the staff member; `just test-e2e` runs the whole thing.

## What this app still cannot do

- **Anything at all to a payment.** `/dash/v1` refuses every non-`GET` method
  at the boundary, before the router matches — no re-poll, no replay, no
  refund, no annotation, and therefore no `audit_log`, because there is nothing
  yet to audit.
- **Any other slice.** Webhooks, checkout sessions, balances, settings and rail
  health are not built, and the nav gate fails if it ever links to them.
- **Contrast checking.** jsdom computes no paint, and no `cypress-axe` pass
  against a real browser exists.
- **Anything through the BFF.** The two handlers under `app/api/dash/` are
  real and tested, and **nothing in this app calls them** — there is no
  client-side data layer yet, and whether there should be one on this origin
  is RD5. They are exercised by `src/server/bff.test.ts` and by no browser and
  no Cypress spec, so what is proven about them is what a unit test can prove:
  which requests they refuse, what they send upstream, and what comes back on
  the wire. That a real browser's `Sec-Fetch-Site` and cookie arrive as this
  code expects them to is **not** proven here.
