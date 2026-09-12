# Hosted and embedded checkout (`checkout.session`, `frontends/apps/checkout`)

**What this is.** A page vpay serves, in two modes, so a merchant does not have
to build a payer page at all. It was requested by the maintainer on 2026-09-04,
verbatim: _"We need a hosted page for driving payments on the web: one in-iframe
version, one fully hosted page. We need that before prod."_ Designed and built
as Step 9 ([`docs/plans/2026-09-04-step9-hosted-checkout.md`](../plans/2026-09-04-step9-hosted-checkout.md),
decisions D1–D13).

This document is the process. [`browser-checkout.md`](browser-checkout.md) is
the surface underneath it — publishable keys, the intent's `client_secret`, the
uniform 404 — and everything it says still holds: vpay's own page authenticates
exactly the way a merchant's own page does.

## The invariant

**A payer's browser holds credentials for one checkout and nothing else, and
every one of them expires.** There is no bearer token anywhere in this flow, no
cookie, and no session state on the server beyond the row. The strongest
credential a payer can hold buys one confirm of one payment intent; the weakest
buys a read of one session's outcome; and both stop working 24 hours after the
session was created, whatever happened in between.

## The two modes

|                                           | Hosted                                                          | Embedded                                                                                        |
| ----------------------------------------- | --------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| What the merchant's server gets back      | a `url`                                                         | a `client_secret`                                                                               |
| What the merchant does with it            | redirects the payer to it                                       | hands it to `@vaam-apps/vpay-stripe-js`'s `initEmbeddedCheckout`, which frames the page         |
| The URL a browser loads                   | `{checkout.public_base_url}/c/{cs_id}?key={pk}#{client_secret}` | `{checkout.public_base_url}/e/{cs_id}?key={pk}#{client_secret}`                                 |
| `Content-Security-Policy` on that page    | `frame-ancestors 'none'`                                        | `frame-ancestors <the merchant's `checkout_origins`>`                                           |
| Required on create                        | `success_url` **and** `cancel_url`                              | `return_url`                                                                                    |
| Refused on create                         | `return_url`                                                    | `success_url`, `cancel_url`                                                                     |
| Where the payer ends up                   | the merchant's `success_url` / `cancel_url`, top-level          | the merchant's `return_url`, and a `vpay:complete` message to the framing page                  |
| Who performs a redirect rail's navigation | the page itself                                                 | the **parent**, on a `vpay:redirect` message — a sandboxed frame may not navigate the top level |

Both modes render the same screens from the same state machine. The mode is a
property of the session, decided by the merchant at create, and the page refuses
to run in the wrong one: `/c/{id}` refuses if it _is_ framed, `/e/{id}` refuses
if it is _not_.

**"Required on create" is about `POST /v1/checkout/sessions`.** There is a
second creator of hosted sessions — `POST /v1/invoices/{id}/pay` — and since
2026-09-10 it may take both URLs from the merchant's registration
(`merchant_clients[].invoices`, issue #91 D2) instead of from the request. The
session that reaches this page is identical either way: `urls_match_ui_mode`
still requires both columns, and nothing here reads where they came from. See
[invoices.md](invoices.md#it-takes-success_url-and-cancel_url-and-stripes-pay-does-not).
`POST /v1/checkout/sessions` deliberately has no such default — a merchant's
process that creates a session already has the payer's context in hand, where
a bill paid from a link in an e-mail does not.

## The object

`checkout.session`, `cs_…`, one row in `checkout_sessions` (migration `0028`).
It **references** a PaymentIntent the merchant already created; it never creates
one. Amount, currency and the rails on offer stay on the intent, where every
existing invariant already guards them.

```json
{
  "id": "cs_…", "object": "checkout.session", "livemode": false,
  "payment_intent": "pi_…", "ui_mode": "hosted",
  "status": "open", "payment_status": "unpaid",
  "success_url": "https://shop/ok?sid={CHECKOUT_SESSION_ID}",
  "cancel_url": "https://shop/cancel", "return_url": null,
  "url": "https://checkout.example/c/cs_…?key=pk_…#cs_…_secret_…",
  "expires_at": 1757000000, "created": 1756913600,
  "client_secret": "cs_…_secret_…"
}
```

**The lifecycle is minimal and vpay's own** (D10). `status` is `open`,
`complete` (the intent reached `succeeded`) or `expired` (24 hours from create,
or the intent reached a terminal non-success state — reported as `expired` with
`payment_status: failed`). `payment_status` is `unpaid` / `paid` / `failed`.
**Exactly one of the four transitions below emits an event**
(`checkout.session.expired`, [webhooks.md](webhooks.md)), and it is the
horizon: the other three either already send a `payment_intent.*` event for
the same thing happening, or are the merchant's own action.
There are no `line_items`, no `mode`, no `amount_total` and no refunds. Field
names mirror Stripe's **only** where the semantics match, and
`sdks/stripe-compat` gets no row for any of it: the compat suite proves claims
rather than making them.

Four things move a session, and only four:

1. **The settlement transaction.** `vpay_db::settlement` flips
   `payment_status`/`status` in the **same commit** as the intent —
   `paid`/`complete` on success, `failed`/`expired` on a terminal decline — so
   the two can never be observed disagreeing.
2. **`POST /v1/checkout/sessions/{id}/expire`**, the merchant's own abandon. It
   is a compare-and-swap with a `NOT EXISTS` live-charge guard _in the same
   statement_: a session whose payer is mid-payment refuses with `409`.
3. **The worker's hourly housekeeping sweep**, which expires `open` sessions
   past `expires_at` that have no live charge — **and, since 2026-09-04, emits
   one `checkout.session.expired` per session in the same transaction as the
   flip.** No new `jobs.kind`: it is still a fourth thing the sweep that
   already retires idempotency keys, client-assertion JTIs and expired leases
   does. It is no longer one statement, though — the event's `data.object` is
   the rendered session, so the sweep reads a page of due sessions, renders
   each, and runs one small transaction per session. See below.
4. **Nothing else.** In particular, a payer's browser cannot move a session.

**A session that is not `open` refuses the confirm** (2026-09-05). The list
above is still exactly four — this reads a session, it never moves one — but
it is what makes two of those four mean anything to a payer. Both
`POST /v1/payment_intents/{id}/confirm` and
`POST /v1/browser/payment_intents/{id}/confirm` ask for the intent's
**newest** session before opening a charge and answer `409
checkout_session_expired` — or `409 checkout_session_complete` — when it is
not `open`, writing no charge, no `provider_requests` row and no job. Until
this landed, nothing retracted the payer's credential: the intent's
`client_secret` is minted once and lives as long as the intent
([browser-checkout.md](browser-checkout.md)), so a payer whose page loaded
before the checkout ended could still pay hours later, and
`checkout.session.expired` was a notification the settlement could contradict
rather than a promise. An `open` session past `expires_at` that the sweep has
not reached is read as expired **without being written** — the same rule the
browser reads carry, for the same reason: a deployment whose worker is down
must not decide whether a payer can pay. The newest row is what decides, so
expiring an abandoned checkout and creating a fresh one leaves the intent
payable. `checkout_session_complete` is defence in depth and is not reachable
through the shipping API today, because `complete` is written only by the
settlement transaction, in the same commit as the intent reaching `succeeded`,
which the confirm refuses one step earlier.

**One open session per intent, enforced by a partial unique index** and not
only by the handler's pre-check — two open sessions on one intent would be two
live payer links to the same payment.

## The routes

Merchant surface, `/v1` — token-authenticated, `Idempotency-Key` required on
POST, tenant-scoped, in `V1_ROUTES`:

| Method | Path                                | What it answers                                                                                                                                                                                              |
| ------ | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| POST   | `/v1/checkout/sessions`             | `201` with the session **and its `client_secret`**, plus `url` when hosted. Refuses an intent that is not `requires_payment_method`, one that already has a charge, and one that already has an open session |
| GET    | `/v1/checkout/sessions/{id}`        | The session with `client_secret`, like `retrieve` on intents                                                                                                                                                 |
| GET    | `/v1/checkout/sessions`             | A list, no secrets, filterable by `payment_intent`                                                                                                                                                           |
| POST   | `/v1/checkout/sessions/{id}/expire` | `open` → `expired`; a session with a live charge is `409`                                                                                                                                                    |

Browser surface, `/v1/browser` — publishable key plus a session credential,
the same CORS layer and the same uniform 404 as the payment-intent reads, in
`BROWSER_ROUTES`:

| Method | Path                                                   | What it answers                                                                                                        |
| ------ | ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| GET    | `/v1/browser/checkout/sessions/{id}?key&client_secret` | The session with `payment_intent` **expanded and carrying the intent's own `client_secret`** — while `status = 'open'` |
| GET    | `/v1/browser/checkout/sessions/{id}/return?key&t`      | The same, with `payment_intent` expanded **without** the intent's secret                                               |
| GET    | `/v1/browser/checkout/origins?key`                     | `{"origins": [...]}` for the key's tenant, with no secret at all                                                       |

Every failure on both session reads is one byte-identical
`ApiError::NotFound { resource: "checkout session" }` — unknown key, session not
found, wrong tenant, wrong credential, missing credential, and past the horizon.

## The two credentials, and why there are two

**D6: secrets ride in URL fragments, never query strings, on vpay-served
pages.** A fragment is never sent to a server, never written to an access log,
and never carried across a redirect. So the session's own `client_secret` — the
strong credential, the one that buys the intent's — lives in the fragment of the
hosted `url` and of the embedded iframe's `src`, and the page reads it in
JavaScript.

A fragment does not survive a rail's redirect, which is precisely what the
return page has to survive. So the return page carries a **separate
`return_token`** (its own column, 160 bits, minted at create, constant-time
compared) in the query string, and it authorises reading the session and polling
its intent — nothing else. The charge already exists by then, so the intent's
`confirm` is a `409` anyway; the token is still not the intent's secret, and the
return read never renders one.

That query string is the reason both reads stop at `expires_at` **whatever the
session's `status`**: a copy of the token is in the rail's storage, in whatever
the rail logs, and in the checkout app's access logs, and the 24-hour horizon is
the bound on how long that copy is worth anything.

Every vpay-served page sends `Referrer-Policy: no-referrer`,
`Cache-Control: no-store` and `X-Content-Type-Options: nosniff`.

## The rest of this flow

Everything between the credentials above and the Status below was 522 lines
until 2026-09-11. It is four pages now, in the order it was written, moved
verbatim:

- [hosted-checkout/state-machine-and-outcomes.md](hosted-checkout/state-machine-and-outcomes.md)
  — the page's state machine, and the screen each outcome lands on
- [hosted-checkout/runtime-configuration.md](hosted-checkout/runtime-configuration.md)
  — `branding.yaml` and `config.yaml`, read at runtime
- [hosted-checkout/page-memory-and-protocols.md](hosted-checkout/page-memory-and-protocols.md)
  — page memory and the PIN vault that is **not** here, the iframe protocol,
  the popup as a third peer, and where the headers come from
- [hosted-checkout/not-built-and-not-proven.md](hosted-checkout/not-built-and-not-proven.md)
  — what is not built, what is not proven, and what the horizon emits

**If you read one, read the last.** It is the page that bounds every claim the
other three make.

## Status

Built and merged 2026-09-04 (Step 9). Proven in a real browser by
`frontends/tests/e2e/cypress/e2e/shop-hosted.cy.ts` (3 tests, both rails,
through `examples/shop`, with every "it was paid" assertion made on the shop's
own database) and `shop-embedded.cy.ts` (4 tests, the frame's exact `src`, MTN
completed inside the frame, Orange breaking out, and an unregistered framer
refused), green from nothing in the `vpay-ci` VM. Proven not to pass with
`vpay-worker` stopped. The page's own suite is 302 vitest cases in 17 files, 0
skipped.

**Updated 2026-09-04: the horizon emits an event.** Six container-backed cases
in `backends/tests/integration/tests/checkout_sessions.rs`, all driving the
shipping `vpay_worker::run_once` over the shipping `seed_singletons`: one
event per sweep with no credential in its serialised body and one delivery and
one job per configured endpoint, no second event on a second sweep, no event
for a session with a live charge, no event for a session the settlement
finished, the event listable and retrievable through `/v1/events` with the
tenant boundary, and the transaction proof above. **No merchant endpoint
outside this repository has received one** — the same limit every other event
type carries.

**Updated 2026-09-05: a session that is no longer `open` refuses the confirm**
(the paragraph after the four transitions above). Seven container-backed cases
in `backends/tests/integration/tests/checkout_sessions.rs` — swept, merchant-
expired, past the horizon and unswept, `complete`, no session at all, the
merchant `/v1` surface with its idempotent replay, and a second session after
an expiry — plus one `postgres_smoke` case for migration `0030`'s index and
four unit cases for the classification and the verdict. **Nothing proves it in
a browser:** the checkout app already paints an expired screen from the
session read's 404, so the refusal sits behind a page that should never reach
it, and no Cypress spec drives a confirm on a dead session.

**Updated 2026-09-06: a hosted session can be rendered in a popup, and it is
the same session.** `@vaam-apps/vpay-stripe-js`'s `openCheckoutPopup` opens
`session.url` in a top-level window the merchant's page owns rather than
navigating the payer's tab. Nothing on this side of the contract changes: the
session is created `ui_mode: 'hosted'` with the same `success_url` and
`cancel_url`, so `examples/shop` sends an identical
`POST /v1/checkout/sessions` for its `hosted` and `popup` modes and gives them
the same `Idempotency-Key` — a payer whose popup is blocked falls back to a
redirect and gets the session they already had rather than a second one.

When that paragraph was written the completion signal did **not** come from
vpay: a popup has no framer, so the child channel in `frontends/apps/checkout`
returned `null` and the page said nothing, and the signal came from the
merchant's own `success_url` running inside the popup. **The entry below
supersedes that half**: the channel now takes a `peer` and a popup is its
third case, so vpay does post `vpay:complete` to an opener it could pin — see
"The popup, and why it is a third peer" above. The rest still holds, and the
design and its limits are in
[browser-checkout.md](browser-checkout.md)'s Status; the short version is that
26 unit cases drive it against stub windows and **no test, and nothing in
any demo run, has opened a real popup**.

**Updated 2026-09-06: the page was restyled and made runtime-configurable.**
daisyUI's `bumblebee` theme and Base UI component defaults replace
`corporate`/`business` and the hand-wired form markup; the outcome screens'
five-second auto-forward is gone and a named "Back to {merchant}" button is
the only way off them; `branding.yaml` and `config.yaml` are read at
container start; and the page can remember a payer's number and last method
on their own device, opt-in and clearable. The popup peer and the return
trip's `soleOrigin` rule landed the same day, at the `examples/shop` track's
request and by the maintainer's decision respectively. What has _not_ changed:
no rail behind this page has ever been anything but a WireMock host, and the
four "what is not built" entries above are joined by five more.

**Updated 2026-09-07: a real browser, and the defect it found.** The entry
above was written with **no Cypress run** — the binary could not be fetched
where it was built — and the specs were not green. The runtime theme override
was emitted inside an explicitly written `<head>` element, and React threw #418
(_hydration failed because the server rendered HTML did not match the client_),
uncaught, on the hosted payment page: **all three of `shop-hosted.cy.ts`'s
tests failed**, with the page stuck on "Loading this payment…". It reproduces
only with a `primary_color` configured, which is why no unit case could have
seen it. Fixed with React 19's own hoisting (`href` + `precedence`), guarded by
`src/layout.test.tsx` on the element tree rather than on the markup. Three more
fixes came with it: a two-second bound on the origins lookup that now holds the
payment page's first byte, a failed outcome that is no longer neutral grey
while a cancelled one is red, and two example values that put a broken image
and a permanent false warning into the demo. **`just test-e2e` on this head:
11 Cypress tests, 11 passing, 0 skipped** — `checkout.cy.ts` (1),
`dashboard.cy.ts` (3), `shop-hosted.cy.ts` (3) and `shop-embedded.cy.ts` (4) —
through a compose stack that mounts both YAML files. The page's own suite is
**448 vitest cases in 23 files, 0 skipped** (was 302 in 17).

**Updated 2026-09-07: the shared UI library moved; this page has not, yet.**
`docs/plans/2026-09-07-ui-revamp.md` Lane A landed `@vpay/ui` on
`@base-ui/react@1.8.0` (the deprecated `@base-ui-components/react@1.0.0-rc.0`
this page still imports was **renamed**, not superseded — a different
package, a real 1.0 release nine months old), Tailwind 4 and daisyUI 5, and
took decision D4 above. **This app is unchanged**: it still imports the old
package, still has its own `tailwind.config.ts`, and its `Field`/`Checkbox`
still write `form-control`/`label-text` — daisyUI 4 classes daisyUI 5 removed,
caught by the new `just verify-ui` gate, which is red on this file until the
migration lands. That migration (Lane B) is not yet started.

**Updated 2026-09-07: the migration landed, and `just verify-ui` is green on
this app.** `screens.tsx`, `checkout-view.tsx`, `return-view.tsx` and
`locale-switch.tsx` compose `@vpay/ui`'s components (`Alert`, `Badge`,
`Button`, `Card`, `Checkbox`, `Field`, `Input`, `Select`, `Spinner`, `Stack`,
`Text`, `PageShell`, `Heading`, `List`) instead of writing daisyUI classes or
importing `@base-ui-components/react` directly; `tailwind.config.ts` is
deleted (Tailwind 4 is CSS-first); `globals.css` is one `@import` plus the
`prefers-reduced-motion` block. `OutcomePanel` now passes
`checkoutOutcomeTone[kind]` straight into `Alert`'s `tone` prop — the
`TONE_CLASS` lookup map and the template literal assembling a class string
that this document's own earlier entries traced the 2026-09-07 outcome-colour
defect to are both gone. `src/config/theme.ts` collapses from 176 to 97
lines: daisyUI 5's `--color-primary` takes any CSS colour directly, so the
sRGB→OKLCh conversion this module used to do by hand is dead code under
daisyUI 5; what remains derives the foreground with the platform's own
`color-mix()`, and `theme.test.ts` verifies that computation holds WCAG AA
contrast (≥4.5:1) for six colours via an independent re-implementation, not by
importing from `theme.ts`. `just test-e2e`'s three specs this app's own
scope covers — `checkout.cy.ts`, `shop-hosted.cy.ts`, `shop-embedded.cy.ts` —
are **8/8, 0 skipped**, including the two lines in the shop specs that used to
select the redirect-continue button by its daisyUI class
(`button.btn-primary`) and now use `data-testid="continue"`.
`dashboard.cy.ts` did not run: `docker compose`'s `dashboard` image build is
broken by an unrelated, pre-existing, out-of-scope defect (`frontends/apps/
dashboard`'s own `next.config.ts` has no `@vpay/ui` in `transpilePackages`,
and its scaffold already imports `@vpay/ui`'s `StatusBadge`) — Lane D's job,
not this migration's. The page's own suite is **459 vitest cases in 23
files, 0 skipped** (was 448). See `docs/status.md`'s "exp26 Lane B" row for
the full gate-by-gate record, the counted `styling_files`/token targets (one
target missed, named there with the reason), and the four regenerated
screenshots.

**Updated 2026-09-07: `examples/shop`, the other side of every payer trip
this document proves, is on Tailwind 4 + daisyUI 5 (Lane C, decision D1).**
The shop adopts the same two public packages `@vpay/ui` now uses, directly —
it imports neither `@vpay/ui` nor `@vpay/tokens`, so the trip this document
describes stays one a merchant can reproduce from public npm packages alone,
not from this workspace's private ones. `src/app/globals.css` — 229 lines of
hand-written CSS — is six lines now: one `@import`, one `@plugin 'daisyui'`
block on `bumblebee`, and a paragraph explaining why daisyUI does not
compromise the file's original "no design system" reasoning. Every one of
the shop's pages and components that had a bespoke class name or a
`style={{…}}` now composes daisyUI classes directly, with zero bespoke CSS
and zero inline styles (were 229 and 24); every `data-testid` this document's
two Cypress specs assert on is unchanged, `#vpay-embedded-checkout` included.
Confirmed against the compiled stylesheet, not assumed: `.btn`,
`.badge-{success,warning,error}`, `.table-zebra`, `.fieldset-legend`,
`.radio`, `.navbar` and `.alert-{warning,error,info}` are all present in the
`@tailwindcss/postcss` build output. **100 vitest cases, 0 skipped** (was 96)
— two new suites render `OrderStatusBadge` and `TestNumbersPanel` and assert
the tone-per-status and caveat-`role="alert"` properties the plan's decisive
mutations name, each confirmed to fail under the named mutation before being
left passing. See `docs/status.md`'s `examples/shop` row for the full count
and for why `styling_files` rose rather than fell under the counting
script's own definition — the shop has no `@vpay/ui`-style layer to absorb
classNames into, by design (D1), so eliminating every inline style could
only ever add files to that count, not remove them.

**Driven end to end in a real browser, through the plan's own recipe.**
Corrected 2026-09-07 in review (`docs/plans/exp26-notes/lane-c-review.md`):
the lane originally proved its two specs by hand, because the `dashboard`
image would not build. That defect is fixed, so `just test-e2e` was run to
completion instead — all four images, all four specs, **11 tests, 11
passing, 0 failing, 0 pending, 0 skipped** (`checkout.cy.ts` 1,
`dashboard.cy.ts` 3, `shop-hosted.cy.ts` 3 — MTN push, Orange redirect, a
declined MTN charge to `cancel_url` — and `VPAY_E2E_FRAMED=1
shop-embedded.cy.ts` 4: the frame's exact `src` and CSP, MTN inside the
frame, Orange breaking out, an unregistered framer refused).

**And the claim about the compiled stylesheet above was true and proved
nothing.** `.badge-{success,warning,error}` were in the output only because
a vitest file contains those strings and Tailwind 4's content detection
scans test files; the components assembled the class at runtime, which
Tailwind's scanner cannot see. Fixed, and gated — see the review's
finding 1, and `docs/status.md`'s Lane C row. One caveat this document must
carry until Lane B lands: `just lint-web` fails on
`frontends/apps/checkout/tailwind.config.ts` with this lane in the tree, for
the dependency-hoisting reason the review's finding 6 sets out.

See [../status.md](../status.md) for the per-feature ledger and the reasons
several of those rows are 🟡 where this document says "built".

**Reviewed 2026-09-07 (`docs/plans/exp26-notes/lane-b-review.md`), and three
things about this page changed as a result.**

_The failure screen is readable again._ `OutcomePanel`'s `failed` branch
renders `.alert-error`, and daisyUI 5's bumblebee paints it at **3.53:1** —
below WCAG AA's 4.5:1 for body text, and down from the **6.82:1** the same
alert had under daisyUI 4. That is the one screen on this page that tells a
payer their money did not move. `@vpay/ui/src/styles.css` now corrects
`--color-error-content` (and `--color-info-content`) as a theme token,
unlayered so it beats daisyUI's own `@layer base` block — the same mechanism
`src/config/theme.ts` uses at runtime for `--color-primary`. Measured in
Chrome from the app's own compiled stylesheet: `.alert-error` **4.62:1**,
`.alert-success` 5.12:1, `.alert-warning` 5.24:1, `.btn-primary` 5.51:1.
`frontends/packages/ui/src/theme-contrast.test.ts` re-measures every tone from
the compiled sheet on every run, so a daisyUI bump that moves a colour is a
failing test rather than an unreadable screen.

_The language switch has a label a payer can see again._ The migration
replaced the visible `<label>` with an `aria-label`, which kept the accessible
name and took the word off the screen. `LocaleSwitch` now names the control
with a visible `<Text>` through `Select`'s `aria-labelledby`.

_The brand-and-language row is a `<header>` again_ on both `CheckoutView` and
`ReturnView`, so a screen-reader user can still skip it by landmark.

Also on this page and unchanged by any of it: the hydration guard
(`layout.test.tsx`'s "renders NO explicit `<head>` element", `href`/
`precedence` hoisting) is untouched, the popup peer and sole-origin rules are
untouched, and the CSP header is untouched — `git diff 08d9b8e..HEAD` over
`*frame*` and `*channel*` is empty. `just test-e2e` is **11/11 across all four
specs**, `shop-hosted.cy.ts` — §6.5's own decisive check for the hydration
fix — among them at 3/3.

Not covered by any test, and named here rather than left implied: **Space or
Enter on the memory opt-in's checkbox**. jsdom does not simulate a native
`<button>`'s keyboard default action, and no Cypress spec touches this
control. The label-click path IS measured (`@vpay/ui`'s `CheckboxLabel`
case clicks the sentence and expects the handler).

**Updated 2026-09-11 (exp53): the page's two live regions are one component.**
`checkout-view.tsx` and `return-view.tsx` each carried the byte-identical
`<div aria-live="polite" aria-atomic="true" data-testid="live-region">` — the
element the whole screen-change announcement depends on, written twice. It is
`@vpay/ui`'s `LiveRegion` now, and `aria-atomic` is **not** a prop on it: a
partial announcement of a payment outcome ("failed" without "your payment") is
worse than none, so no third screen can get it subtly wrong.

Nothing else in this app changed. Its suite is the same **507** cases in 24
files, and the one that defends the tone of a failed outcome was used as the
decisive mutation for exp53's variant maps: setting `Alert`'s `error` variant
to the empty string in `alert.variants.ts` makes
`checkout-view.test.tsx > does NOT render a failed payment in the neutral tone
a cancelled one beats` fail with `expected 'mt-4 alert' to contain
'alert-error'`.

**Updated 2026-09-11 (b1e): axe contrast checks on outcome screens added.**
The bumblebee theme's contrast had been "checked by nobody" (2026-09-07,
above) — @vpay/ui's Storybook runs axe against corporate/business (unused
here), and that Storybook is not in `just ci`. Issue #73 decision: run axe on
the four outcome screens. `frontends/apps/checkout/src/components/outcomes.axe.test.tsx`
is a new vitest suite: 10 tests (succeeded/failed/canceled checkout outcomes +
succeeded/failed return outcomes, each in English and French) running axe's
`color-contrast` rule. **jsdom limitation:** the tests pass (10/10) with zero
violations reported because jsdom computes no CSS colors. A violation that
exists in the rendered page will NOT be caught here. **The real verification is
in a real browser** — `cypress-axe` running against the `compose.e2e.yml`
stack, plan §7 row 6 (still unbuilt). This test documents the requirement and
will catch violations when the real browser implementation is added. The
theme's own tone palette is measured by `@vpay/ui`'s `theme-contrast.test.ts`,
which compiles the stylesheet and verifies every rendered tone clears WCAG AA
(4.5:1), including the error/info overrides at lines 887–899 above.

**Updated 2026-09-12, and superseded by the entry after it on the same day.**
The two paragraphs below record a browser-level accessibility gate that
existed for a few hours and the four defects it found on this page. It was
built on `@vpay/ui`, which the next entry deletes, so **the gate and the fix
are both gone**; they are kept here because the defects were real, because
one of them was on the screen a payer meets, and because a measurement is not
made wrong by being overtaken. The full record, including two findings that
transfer to whoever restores Storybook, is in
[../status/verification/2026-09-12-browser-a11y.md](../status/verification/2026-09-12-browser-a11y.md).

**Updated 2026-09-12: the real browser the entry above said was "still
unbuilt" now exists — and it failed this page.** That entry ended by naming
what was missing: "**The real verification is in a real browser** —
`cypress-axe` running against the `compose.e2e.yml` stack, plan §7 row 6
(still unbuilt)." It is built, by a different route than the one named:
`@storybook/addon-vitest` renders every Storybook story in a real Chromium and
`@storybook/addon-a11y` runs axe-core 4.13.0 over each one, as `just
test-storybook` and as a step in CI's `web` job. Storybook rather than Cypress
for a measured reason — `shop-hosted.cy.ts` could never get a colour-contrast
verdict out of this page at all, because daisyUI 5's `:root` scroll-lock rule
carries an unconditional `background-image` and axe-core abandons the
`color-contrast` rule under one, which three rounds of mutation in
`outcome-contrast.test.ts`'s header record. A story renders inside
`#storybook-root` with no such ancestor, and the rule returns a verdict.

**The verdict on this page was a violation.** `Mtn Number Rejected` — the
screen a payer sees when their MSISDN is refused — paints its
`role="alert"` message in `text-error` at 12px: bumblebee's `--color-error`
`#ff6266` on `#ffffff`, **2.92:1**, against WCAG AA's 4.5:1 for body text. It
is the least readable text on a screen whose entire job is to tell someone
what went wrong. Two `@vpay/ui` stories fail on the identical pair
(`field.stories.tsx > Invalid`, `text.stories.tsx > Tones`), so the cause is
the shared `text-error`, not this app.

This is a **different pair** from the one the 2026-09-07 review fixed, and
that is why nothing caught it earlier: that review corrected
`--color-error-content`, the foreground `.alert-error` paints **on**
`--color-error`, and both `theme-contrast.test.ts` and `outcome-contrast.test.ts`
measure `--color-error` as a background. `--color-error` as a _foreground on
base-100_ had never been measured by anything.

**Not fixed here**, and the reason is the same one `@vpay/ui/src/styles.css`
gives for recording its own `--color-error-content` change as the maintainer's
delegate's decision of 2026-09-07: moving `--color-error` moves the
payment-failure alert that two suites pin. The story carries
`a11y: { test: "todo" }` — axe still runs and still reports it on every run,
the run does not fail — with the measured ratio in a comment beside it, and
`docs/status/frontend.md` carries the row. **A decision is owed; nothing about
this screen has been quietly adjusted.**

**Updated 2026-09-12: the rejection message is readable, and the fix is a
theme token.** The entry above ends by saying a decision was owed on the
2.92:1 `Mtn Number Rejected` screen. The maintainer's answer was to fix it,
and it is fixed — **the story now passes axe in a real browser with no
exemption**, and `a11y: { test: "todo" }` appears nowhere in this repository.

The cause was never this app. daisyUI's `--color-<tone>` is a **fill** — the
colour `.alert-error` paints as a background, with `--color-error-content` on
top, which is the pair the 2026-09-07 review corrected and which
`outcome-contrast.test.ts` pins. `text-error` is a different pair: it puts
that same fill colour on `--color-base-100`, and bumblebee's error fill is
2.92:1 on white. Nothing had measured that direction, which is why three
`@vpay/ui` stories failed on the identical value.

`@vpay/ui/src/styles.css` now defines `--color-error-ink`,
`oklch(55% .191 22.216)` = `#c92d3a` — daisyUI's own hue and chroma, darkened
only as far as AA needs plus a margin — at **5.36:1** on `base-100` and
**4.91:1** on `base-200`. `--color-error` itself is **unchanged**, so nothing
about the payment-failure alert this document spends three entries on has
moved: the same background, the same `--color-error-content`, the same 4.62:1
this page already records. The only thing that changed on this page is the
colour of the `role="alert"` line telling a payer their MSISDN was refused,
which went from `#ff6266` to `#c92d3a`.

`frontends/packages/ui/src/theme-ink-contrast.test.ts` measures both
directions from the compiled stylesheet on every run, so this does not depend
on anybody remembering to run a browser. It also fails if a component writes
`text-error` again, and it asserts `--color-error` is **still** unreadable on
white — so if a daisyUI bump ever fixes it upstream, the extra token gets
deleted rather than carried out of habit.

**Where that leaves this page, stated plainly.** The entry above it announced
a real browser finally checking these screens; the entry below it deleted the
package that ran the check. Both are true and they land the same day. **As of
this commit the checkout has no visual-review surface and no browser-level
accessibility check.** `screens.axe.test.tsx` does not replace one — it runs
in jsdom, which computes no colour, so it can never answer `color-contrast`
either way. That is a named, accepted gap with a filed task, not an
oversight, and it is a step backwards from the paragraph above it.

**Updated 2026-09-12: `@vpay/ui` — the package every entry above from
2026-09-07 onward describes — is deleted.** Both this app and the dashboard
now compose the published `@vaam-apps/ui` instead (the maintainer's scope for
this cutover: "both apps, delete `@vpay/ui` in this PR", not the checkout
alone as some entries above anticipated). What changed for this page,
concretely: `outcomes.axe.test.tsx` (jsdom, zero-violations-because-no-CSS,
named above as a documented limitation) is deleted along with the
`axeContrastViolations` helper only it called — its jsdom limitation was
real and is not missed. `theme-contrast.test.ts` is deleted with the
package; `outcome-contrast.test.ts` (same real-Tailwind, no-DOM method,
already the pattern this page's own b1e entry names) is rewritten against
the app's own compiled `globals.css` and the theme is now `dark`, not
`bumblebee` — `src/config/theme.ts`'s runtime brand-colour retargeting
(decision 3, 2026-09-12) survives unchanged in method, computing contrast
against the new, darker ground. `LiveRegion` (named above, 2026-09-11) has
no `@vaam-apps/ui` counterpart; `checkout-view.tsx` and `return-view.tsx`
each carry a plain `<div aria-live="polite" aria-atomic="true">` again,
faithful to the same markup but without the test `LiveRegion` used to carry
guaranteeing a hostile caller cannot weaken those two attributes.
`LocaleSwitch` moved to a native `<select>` (decision 8): `@vaam-apps/ui`'s
own `Select` cannot be named by a visible `<label>`, and the control's
French accessible name is exactly what `checkout-view.test.tsx` pins. See
`docs/status/verification/2026-09-12.md` for what was run and measured, and
[hosted-checkout/state-machine-and-outcomes.md](hosted-checkout/state-machine-and-outcomes.md)'s
own 2026-09-12 entry for the outcome-colour vocabulary's new name.

**Updated 2026-09-12: Storybook is back, and building it found that this page
had been rendering without its theme.** The entry above closes by saying the
checkout has no visual-review surface and no browser-level accessibility
check. It has both again — in `frontends/apps/checkout/.storybook/`, hosting
the same 22 stories the cutover deleted, verbatim but for their doc comment,
because `CheckoutView`'s and `ReturnView`'s props did not change. `just
test-storybook` renders each in a real Chromium and runs axe over it; CI's
`web` job runs the recipe.

**The defect it found is more important than the gate.** CSS drops an
`@import` that follows another at-rule, and `app/globals.css` placed
`@import "@vaam-apps/ui/styles/theme.css"` **after** `@plugin "daisyui"`.
Tailwind's own parser is lenient about that, so `next build` and
`src/styling-gate.test.ts` — which compiles this very file through
`@tailwindcss/postcss` — both inlined the theme and both passed. A
spec-compliant pipeline did not: Storybook's vite build emitted a stylesheet
with every `var(--color-base-100)` in it and `--color-base-100` defined
nowhere, so the payer's screens rendered on the browser's default white while
the shipped page is `#0a0b0d`. Moving one line took the stylesheet from
136 360 to 145 903 bytes. `frontends/apps/dashboard/app/globals.css` had the
identical ordering and is fixed in the same commit.

**This page's own answer, measured at last.** All 22 checkout screens clear
WCAG AA on `@vaam-apps/ui`'s theme, colour-contrast included, in a real
browser — the question left open when the cutover replaced bumblebee and
nothing could measure the result. The gate is proved live rather than
trusted: a `#3a3a3a` probe fails at **1.73:1 against `#0a0b0d`**, naming the
real background, and that same probe PASSED for six runs while the theme was
missing.

**What it still does not cover.** It renders stories, not the served page, so
nothing here retires `outcome-contrast.test.ts`'s note that axe cannot walk
the real page's DOM for a background. The 22 states are the ones
`checkout-view.test.tsx` asserts against; Lane 3b's unnamed-merchant screens
remain covered by vitest and not by a story. And the dashboard's screens have
no equivalent gate.
