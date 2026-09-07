# exp28 — the dashboard a person can click

Branch `claude/exp28-dashboard-pages`, base `05ae51a` (master with the four UI
lanes, staff sign-in and the `/dash/v1` read surface). Notes on what was
built, what was measured, and what was not done.

## What this pass had to decide, and what it decided

### The callback is not a page, and that is worth saying twice

[ADR-0017](../../adr/0017-staff-authentication.md) decision 4 has the Next.js
app's **own server** request the authorization code, follow the `302` and
exchange it. A reader who knows OAuth opens `app/` looking for a route at
`dashboard_client.redirect_uris`, does not find one, and reasonably concludes
the flow is unfinished.

It is not. `http://localhost:3000/dash/v1/callback` is an identifier the two
legs must spell identically — `ClientRegistration::allows_redirect_uri` matches
it byte for byte, no prefix, no wildcard — and nothing ever fetches it. Adding
a route there would be a page nobody can reach, which is this repository's own
failure mode in a different costume. It is stated in
`src/server/oauth.ts`'s header, in `docs/flows/dashboard.md`, in
`docs/flows/dashboard-auth.md` and in `compose.e2e.yml`'s own comment, because
four places is roughly the number of places somebody will look.

### The PKCE verifier is stored nowhere, and that is the strongest form

In a browser-driven flow the verifier has to survive a redirect, so it goes
into a cookie or a server-side store, and each of those is a place it can leak
from. Here both legs run inside one function call, so the verifier is a local
binding for the length of two `fetch`es. **The absence of a store reads, to
somebody scanning for one, like the verifier having been forgotten** — so
`pkce.ts`'s header says so at length.

`oauth.test.ts`'s decisive case is not "a verifier was sent". The stub echoes
the challenge into the code it returns, so the assertion is that the exchange
presents the verifier whose `S256` **is** the challenge this call sent.
Generating a second pair for the exchange — which a plausible refactor would do
— fails it. Measured.

### The access token is not kept in the app

`vpay_api::staff::oauth::token` writes the minted token into the
`staff_sessions` row before answering. So this app keeps none: every render
reads it back from `GET /dash/v1/staff/session`. That is what makes signing out
a revocation *in practice* rather than only in the schema — a token cached in
the Next process would keep rendering payments for a signed-out session until
it expired, and every unit test would still pass.

### Fail closed on configuration, which is the opposite of the checkout page

`frontends/apps/checkout` defaults everything and warns, deliberately: a
payment page with no branding still takes a payment. A dashboard with no client
registration is a login form that **cannot log anybody in**, and rendering one
invites a staff member to type a password into it repeatedly. `/login` renders
the variable names an operator has to set, and no form.

### Two columns that are absent rather than empty

- **The rail.** `GET /dash/v1/payment_intents` returns no charge at all, so the
  only rail-shaped value in the list response is `payment_method_types` — the
  rails an intent *may* be confirmed against. The column is headed **Methods**.
  A "Rail" heading over that value would be wrong for every intent that offers
  two and was taken by one, and it would be wrong *invisibly*.
- **The payer.** `charges.payer_ref_masked` is never written by anything. The
  **detail** page renders it from the column as an em dash and says so; the
  **list** has no such column, because the list response has no such field —
  it would be sourced from nothing rather than from a null.

  The brief asked for the column on the list. This is the one place this pass
  did something other than what it was asked, and the reason is that a column
  headed "Payer" that is structurally incapable of ever holding a value is a
  worse artefact than its absence. It is called out here rather than quietly
  done.

### The nav gate got a second half

exp26's review made the nav-honesty rule a test, resolving every internal
`href` in the rendered layout against `app/**/page.tsx`. That half cannot see a
link rendered only in a branch the test does not exercise — which is exactly
what a session-aware nav would introduce. `NAV_LINKS` is now an exported
constant and the test checks it *as well as* the markup, plus a third case that
the nav renders what it declares, so neither half can pass while the other is
wrong.

The nav is **not** session-aware, for a related reason: the root layout renders
on `/login` too, and a "Sign out" control on a page where nobody is signed in
is the same kind of claim as a menu entry for a page nobody wrote. Who is
signed in and the way out live in `SignedInBar`, which only the pages behind
the gate render.

## Gates, recipe by recipe

| Gate | Result |
|---|---|
| `pnpm --filter @vpay/dashboard test` | **128 passed, 17 files, 0 skipped** (was 22 in 6) |
| `pnpm --filter @vpay/dashboard typecheck` | clean |
| `pnpm --filter @vpay/dashboard lint` | clean, `--max-warnings 0` |
| `pnpm --filter @vpay/dashboard build` | clean; all seven routes dynamic, as they must be — every one reads cookies |
| `pnpm --filter @vpay/e2e typecheck` / `lint` | clean |
| `just verify-ui` | exit 0 |
| `git grep className` under `app/`+`src/` (non-test) | **zero matches** |

## Decisive mutations

Each one applied to the tree, the suite run, the mutation reverted.

| Mutation | Expected | Measured |
|---|---|---|
| `COOKIE_ATTRIBUTES.httpOnly` → `false` | `cookies.test.ts` fails | **1 failed, 127 passed** — "is httpOnly — no script on this origin may read a payments credential" |
| token exchange sends `createPkce().verifier` (a *fresh* pair) instead of the one the challenge came from | `oauth.test.ts` fails | **1 failed, 127 passed** — "carries the verifier the challenge was derived from — THE decisive case" |
| `NAV_LINKS` gains `{ href: '/webhooks' }` | `layout.test.tsx` fails | **2 failed, 126 passed** — the rendered-markup case *and* the constant case |
| (in a browser) — | `dashboard.cy.ts` asserts the session cookie is `httpOnly`, that `document.cookie` cannot see it, and that no JWT appears in the rendered page | see the Cypress section |

## Two things this pass got wrong first, and what they cost

**`just demo-staff` addressed the wrong stack.** `test-e2e` called it as a bare
`just demo-staff`, which inherits the exported environment but **not** the
recipe's variable overrides — so `demo_project` resolved to its default and the
sub-invocation created a one-off container in `vpay-demo`, a stack this run had
never brought up and which belonged to somebody else. It failed instantly
(that stack's image predates the `staff` subcommand) and `--rm` removed the
container, so nothing was left behind. The fix repeats all six overrides on the
sub-invocation, with a comment saying why.

**Cypress test isolation cleared the session between tests.** The spec is a
sequence — sign in, look around, sign out, sign back in — and Cypress clears
cookies before every test by default. Four of six tests failed on pages that
had correctly redirected to `/login`. `testIsolation: false` on the describe
block is the fix, and the alternative is worse than it looks: vpay's TOTP
replay guard admits only a strictly greater time step, so six sign-ins need six
*different* 30-second windows — three minutes of waiting for clocks in order to
test a payments list.

## What was NOT done

- **A payer column on the payments list.** See above; the detail page has it.
- **`/config/v1`**, the operator verification endpoint `frontends/apps/checkout`
  serves. The dashboard's equivalent would disclose the internal API hostname,
  which is a deployment-layout fact the checkout's version deliberately omits;
  it was not worth designing a redacted version this pass.
- **Anything a staff member can do *to* a payment.** No re-poll, no replay, no
  refund, no annotation — `/dash/v1` refuses every non-`GET` method at the
  boundary — and therefore no `audit_log`, because there is nothing to audit.
- **Slices 2–6** (webhooks, checkout sessions, balances, settings, rail
  health). The nav does not link to them, and the gate fails if it ever does.
- **A `cypress-axe` pass against a real browser.** The axe coverage here is
  jsdom's, so it is structural only — `color-contrast` computes nothing in
  jsdom and is not checked anywhere. Unchanged from exp26's position.
- **Anything in `README.md` or `backends/apps/vpay-server/src/main.rs`**, both
  of which other agents held during this pass.
