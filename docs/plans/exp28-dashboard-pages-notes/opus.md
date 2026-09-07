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
| `pnpm --filter @vpay/dashboard test` | **136 passed, 18 files, 0 skipped** (was 22 in 6) |
| `pnpm --filter @vpay/dashboard typecheck` | clean |
| `pnpm --filter @vpay/dashboard lint` | clean, `--max-warnings 0` |
| `pnpm --filter @vpay/dashboard build` | clean; all six real routes server-rendered on demand, as they must be — every one reads cookies. Next's own generated `/_not-found` is the one static entry, and this row said "all seven routes dynamic" until that was checked against the build output |
| `pnpm --filter @vpay/e2e typecheck` / `lint` | clean |
| `just verify-ui` | exit 0 |
| `git grep className` under `app/`+`src/` (non-test) | **zero matches** |
| `just test-e2e` (isolated project, non-default ports) | **exit 0 — 15 Cypress tests across four specs, 15 passing, 0 failing, 0 skipped**: `checkout.cy.ts` 1, `dashboard.cy.ts` **7**, `shop-hosted.cy.ts` 3 in pass 1; `shop-embedded.cy.ts` 4 in pass 2. Stack torn down by `down -v`, zero containers and zero volumes left |

Run from nothing on the branch's final head. Earlier runs on earlier heads are
in this file's "what went wrong" section, because each of them found something.

## Decisive mutations

Each one applied to the tree, the suite run, the mutation reverted. The
"measured" totals are the suite size **at the time each was run**, which grew
from 128 to 136 across the pass.

| Mutation | Expected | Measured |
|---|---|---|
| `COOKIE_ATTRIBUTES.httpOnly` → `false` | `cookies.test.ts` fails | **1 failed, 127 passed** — "is httpOnly — no script on this origin may read a payments credential" |
| token exchange sends `createPkce().verifier` (a *fresh* pair) instead of the one the challenge came from | `oauth.test.ts` fails | **1 failed, 127 passed** — "carries the verifier the challenge was derived from — THE decisive case" |
| `NAV_LINKS` gains `{ href: '/webhooks' }` | `layout.test.tsx` fails | **2 failed, 126 passed** — the rendered-markup case *and* the constant case |
| the status filter's control is renamed off `status` | `payments-filters.test.tsx` fails | **1 failed, 132 passed** — "submits the status as `status`, which is the parameter vpay reads" |
| `pagerHrefs` reads `has_more` the same way in both directions (the bug as it was) | `payments-query.test.ts` fails | **2 failed, 134 passed** — one per half of the inversion |
| (in a browser) — | `dashboard.cy.ts` asserts the session cookie is `httpOnly`, that `document.cookie` cannot see it, and that no JWT appears in the rendered page | see the Cypress section |

## The last defect, and how it was found

### `has_more` means "in the direction you are paging", and it inverts

The last defect this pass found, and it found it by reading
`vpay_db::PaymentIntents::list_page_filtered`'s SQL rather than assuming what
the flag meant. The query is one statement with a direction: forward it walks
`seq DESC` from `starting_after`; backward it walks `seq ASC` from
`ending_before` and reverses the rows before answering. So `has_more` is "there
was a row past the `limit` **in the direction just walked**" — *older rows
exist* going forward, *newer rows exist* going back.

`pagerHrefs` read it the same way in both directions, which is wrong twice at
once. Paged backwards, `Next` disappeared although the page it came from
certainly still existed, and `Previous` appeared at the newest end pointing at
nothing. Both render an empty list, which is precisely the failure this
module's own doc comment warns about — an empty list reads as data having been
lost.

No test caught it because none paged backwards; two do now, one per half.
Nothing in `dashboard.cy.ts` could have: the demo stack never has more than one
page of payments.

## The screens

Committed under [`screenshots/`](screenshots/), captured by `dashboard.cy.ts`
itself against the real stack rather than by hand — so the images are of the
build the spec passed against, and re-capturing them is a `just test-e2e`
rather than a ritual.

| Image | What it is |
|---|---|
| `01-login.png` | `/login`, before anything is typed — no credential is in an image |
| `02-enrolment.png` | `/login/totp` on a first sign-in: the QR, the base32 key, and the notice that nothing is stored until a code from it verifies |
| `03-payments.png` | `/payments` — the signed-in bar naming the staff member and the tenant, the filters, and real rows created by `checkout.cy.ts` |
| `04-payment-detail.png` | `/payments/{id}` |

`02-enrolment.png` deliberately contains a real TOTP secret. It belongs to a
staff member the stack creates fresh on every run and destroys with `down -v`
minutes later, and it authenticates nothing that will exist tomorrow.

**Looking at them found a defect nothing else did.** The detail page headed
itself `Payment` and the first section of the view inside it headed itself
`Payment` too — two `<h2>` with one name, which reads as a rendering bug to
anybody who opens the page. No test asserted on heading text, and axe does not
object to it. The section is `Summary` now, and
`payment-detail.test.tsx` pins it. That is the entire argument for "render it
and look at it" in one finding.

## Four things this pass got wrong first, and what they cost

**`just demo-staff` addressed the wrong stack.** `test-e2e` called it as a bare
`just demo-staff`, which inherits the exported environment but **not** the
recipe's variable overrides — so `demo_project` resolved to its default and the
sub-invocation created a one-off container in `vpay-demo`, a stack this run had
never brought up and which belonged to somebody else. It failed instantly
(that stack's image predates the `staff` subcommand) and `--rm` removed the
container, so nothing was left behind. The fix repeats all six overrides on the
sub-invocation, with a comment saying why.

**The pager read `has_more` one way** — see the section above. Not in this
list originally; it belongs here, because it was a defect this pass shipped
into four commits before reading the SQL it depended on.

**The spec asserted an event type nothing writes.** Its first real run failed
on `payment_intent.created` in the timeline of a just-minted intent — and that
type is one of **five of the eight documented event types that nothing emits**
(`docs/status.md`, "Events written by the worker": only `.succeeded`,
`.payment_failed` and `checkout.session.expired` are ever written, by
settlement and by the housekeeping sweep). The spec now asserts what is
actually there: "No events yet." on an unconfirmed intent, and a real
`payment_intent.*` on one that has a charge. This is the good case for running
a thing rather than reasoning about it — the wrong assumption was the author's,
and the only thing that could have caught it is the run.

**Cypress test isolation cleared the session between tests.** The spec is a
sequence — sign in, look around, sign out, sign back in — and Cypress clears
cookies before every test by default. Four of six tests failed on pages that
had correctly redirected to `/login`. `testIsolation: false` on the describe
block is the fix, and the alternative is worse than it looks: vpay's TOTP
replay guard admits only a strictly greater time step, so six sign-ins need six
*different* 30-second windows — three minutes of waiting for clocks in order to
test a payments list.

## What is covered end to end and by no unit test

Worth naming, because a reader counting 136 vitest cases could reasonably
assume otherwise.

`requireStaff` and the four server actions are exercised **only** by
`dashboard.cy.ts`. They are the two files that cannot be unit tested without
mocking `next/headers` and `next/navigation`, and mocking those would test the
mock: `cookies()` and `redirect()` only mean anything inside a request, and a
test that stubbed them would assert that this app calls functions it obviously
calls. What *is* unit tested is everything they decide with — `gateFor`'s three
branches, the cookie attributes, both OAuth legs against a stubbed `fetch`, the
`FormData` reading — and the composition of those is what the browser run
covers.

The brief's fourth decisive mutation, "render a page without a session →
redirected", is therefore a **Cypress** case and not a vitest one: the spec's
first test visits `/payments` signed out and asserts it lands on `/login` and
stays there.

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
- **Unit tests for `requireStaff` and the server actions** — see the section
  above for why, and for what covers them instead.
- **Anything in `README.md` or `backends/apps/vpay-server/src/main.rs`**, both
  of which other agents held during this pass. Nothing here needed a change in
  either: `staff add` already existed and `just demo-staff` drives it through
  the shipped CLI.
