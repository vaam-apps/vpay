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

| Route              | What it does                                                             |
| ------------------ | ------------------------------------------------------------------------ |
| `/`                | Redirects to `/payments` or `/login`. Nothing of its own                 |
| `/login`           | Work email + argon2id password — leg one                                 |
| `/login/totp`      | Six digits. A **first** sign-in also renders the enrolment QR and secret |
| `/login/password`  | Replaces the printed one-time password. Not optional — see below          |
| `/payments`        | The bound merchant's intents; status and date filters, cursor paging     |
| `/payments/{id}`   | Intent, charge, refunds, last error, event timeline                      |

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

## How the code is laid out

| Directory              | What lives there                                                                  |
| ---------------------- | --------------------------------------------------------------------------------- |
| `app/`                 | Routes only. Composition, a redirect, and a fetch — no logic worth testing alone   |
| `src/components/`      | Every rendered component. Pure props in, markup out; no `fetch`, no `next/headers` |
| `src/server/`          | Everything that touches vpay, cookies, or PKCE. Imported only by `app/` and itself |
| `src/config/`          | `settings.ts` decides what a configuration means; `runtime.ts` reads the environment once |
| `src/format.ts`        | Money, instants, the em dash. The numbers a reader is entitled to have right      |
| `src/payments-query.ts`| The URL's filter vocabulary ↔ the API's, and the two paging links                  |
| `src/testing/`         | Fixtures. Imported by tests and by nothing under `app/`                            |

The split between `src/server/` and `src/components/` is the one that matters:
a component that fetched would be a component no test could render, and a
`fetch` inside a component is how a page ends up unable to say *why* it is
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
*rendered* markup does too; and the nav renders what it declares. The pair of
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
`payment_method_types` — the rails an intent *may* be confirmed against. The
column is headed **Methods**. A "Rail" heading over it would be wrong for every
intent that offers two and was taken by one, and wrong *invisibly*. The detail
page has a real `Rail`, from `charge.provider_code`.

**The masked payer is an em dash, and it is a real `null`.**
`charges.payer_ref_masked` is never written by anything (`docs/status.md`), so
the detail page renders it **from the column** and never derives it — the only
other value that could produce a mask is the payer's unmasked phone number, and
reading that into a staff surface to make a row look populated is the trade
this refuses. The row exists rather than being omitted so the value appears the
day the column is written. `payment-detail.test.tsx` pins both directions, and
`dashboard.cy.ts` walks the list until it finds a payment that *has* a charge
before asserting it — a dash on an intent nobody confirmed proves nothing.

The **list** has no payer column at all, which is the stronger form of the same
point: there is no field there to be null.

## Testing this app

```bash
pnpm --filter @vpay/dashboard test        # 17 files, 128 tests
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

| Mutation                                                        | Fails                                    |
| --------------------------------------------------------------- | ---------------------------------------- |
| `COOKIE_ATTRIBUTES.httpOnly` → `false`                          | `src/server/cookies.test.ts`             |
| exchange a *fresh* PKCE verifier rather than the one the challenge came from | `src/server/oauth.test.ts` |
| `NAV_LINKS` gains a page nobody wrote                           | `src/layout.test.tsx`, twice             |

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
