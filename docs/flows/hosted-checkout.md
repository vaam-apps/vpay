# Hosted and embedded checkout (`checkout.session`, `frontends/apps/checkout`)

**What this is.** A page vpay serves, in two modes, so a merchant does not have
to build a payer page at all. It was requested by the maintainer on 2026-09-04,
verbatim: *"We need a hosted page for driving payments on the web: one in-iframe
version, one fully hosted page. We need that before prod."* Designed and built
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

| | Hosted | Embedded |
|---|---|---|
| What the merchant's server gets back | a `url` | a `client_secret` |
| What the merchant does with it | redirects the payer to it | hands it to `@vaam-apps/vpay-stripe-js`'s `initEmbeddedCheckout`, which frames the page |
| The URL a browser loads | `{checkout.public_base_url}/c/{cs_id}?key={pk}#{client_secret}` | `{checkout.public_base_url}/e/{cs_id}?key={pk}#{client_secret}` |
| `Content-Security-Policy` on that page | `frame-ancestors 'none'` | `frame-ancestors <the merchant's `checkout_origins`>` |
| Required on create | `success_url` **and** `cancel_url` | `return_url` |
| Refused on create | `return_url` | `success_url`, `cancel_url` |
| Where the payer ends up | the merchant's `success_url` / `cancel_url`, top-level | the merchant's `return_url`, and a `vpay:complete` message to the framing page |
| Who performs a redirect rail's navigation | the page itself | the **parent**, on a `vpay:redirect` message — a sandboxed frame may not navigate the top level |

Both modes render the same screens from the same state machine. The mode is a
property of the session, decided by the merchant at create, and the page refuses
to run in the wrong one: `/c/{id}` refuses if it *is* framed, `/e/{id}` refuses
if it is *not*.

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
   is a compare-and-swap with a `NOT EXISTS` live-charge guard *in the same
   statement*: a session whose payer is mid-payment refuses with `409`.
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

| Method | Path | What it answers |
|---|---|---|
| POST | `/v1/checkout/sessions` | `201` with the session **and its `client_secret`**, plus `url` when hosted. Refuses an intent that is not `requires_payment_method`, one that already has a charge, and one that already has an open session |
| GET | `/v1/checkout/sessions/{id}` | The session with `client_secret`, like `retrieve` on intents |
| GET | `/v1/checkout/sessions` | A list, no secrets, filterable by `payment_intent` |
| POST | `/v1/checkout/sessions/{id}/expire` | `open` → `expired`; a session with a live charge is `409` |

Browser surface, `/v1/browser` — publishable key plus a session credential,
the same CORS layer and the same uniform 404 as the payment-intent reads, in
`BROWSER_ROUTES`:

| Method | Path | What it answers |
|---|---|---|
| GET | `/v1/browser/checkout/sessions/{id}?key&client_secret` | The session with `payment_intent` **expanded and carrying the intent's own `client_secret`** — while `status = 'open'` |
| GET | `/v1/browser/checkout/sessions/{id}/return?key&t` | The same, with `payment_intent` expanded **without** the intent's secret |
| GET | `/v1/browser/checkout/origins?key` | `{"origins": [...]}` for the key's tenant, with no secret at all |

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

## The page's state machine

`frontends/apps/checkout` keeps its logic in a **pure reducer** — no `fetch`, no
timer, no DOM — with the React layer as wiring. That is what makes every refusal
and every transition a test rather than a branch inside an effect nobody can
reach twice.

```
                    ┌──────────────────────────────────────────┐
   load             │                                          │
   ────► loading ───┤ credentials missing → invalid link       │
                    │ framed when it must not be, or framer    │
                    │   not on the merchant's list → refused   │
                    │ session past its horizon → expired       │
                    │ no rail this page can drive → refused    │
                    └───────────────┬──────────────────────────┘
                                    │ session read ok
                                    ▼
                         select_rail  (only when the intent
                                    │  offers more than one)
                    ┌───────────────┴───────────────┐
                    │ MTN (push)                    │ Orange (redirect)
                    ▼                               ▼
              collect_msisdn                  ready_redirect
                    │ confirm                       │ confirm (redirect: 'if_required')
                    ▼                               ▼
               confirming                      redirecting ──► the rail's own page
                    │                               │            (top-level, or the
                    ▼                               │             parent navigates
                 waiting  ◄─────────────────────────┘             when framed)
                    │ poll  /v1/browser/payment_intents/{id}
                    ▼
                 outcome  (succeeded │ failed │ canceled)
                    │
                    ▼
               forwarding ──► success_url / cancel_url (hosted)
                              return_url + vpay:complete (embedded)
```

The return page is its own document with its own smaller machine: it holds no
intent secret, so it has **no confirm transition at all** — it polls the return
route until the intent is terminal, shows the outcome, and forwards.

Three properties worth stating because each is a test:

- **The embed check runs before the credential is read.** A framer that is not
  on the merchant's list is refused even when the URL carries no key and no
  secret, so the refusal cannot be used to probe which half of a link is wrong.
- **The language switch does not navigate.** A `?lang=fr` link has no fragment,
  and resolving a fragment-less relative URL *drops* the current one — which on
  this page is the session's credential. The server picks the initial locale
  from `Accept-Language` (French by default: Cameroon first, and Orange's own
  page is French by default), and the switch swaps the dictionary in place.
- **No floating point anywhere in the money path.** `5000 XAF` becomes the
  string `"5000"` by moving a decimal point through the integer's digits, and
  `Intl.NumberFormat` formats that string. `minor / 100` would be float
  arithmetic in a money path, which the Rust half of this repository denies
  workspace-wide ([money.md](money.md)).

## The iframe protocol

`postMessage`, with `event.origin` checked on both sides against a pinned value
— the parent against `new URL(baseUrl).origin`, the child against the allowed
origin that framed it. **`'*'` appears nowhere as a target**, and the parent
posts nothing into the frame at all: the child learns its framer's origin from
the CSP vpay served it, not from a message, so the strongest form of "never
`postMessage(…, '*')`" is "never `postMessage`".

| Message | Direction | Payload | Why |
|---|---|---|---|
| `vpay:resize` | child → parent | `{ height }` | The frame is created at `height: 0`; the page owns its height and sends this on first paint and on every `ResizeObserver` callback. A height the SDK guessed would be an iframe silently stuck at the wrong size |
| `vpay:complete` | child → parent | `{ session, status }` | `session` is the `cs_…` **id string**, never the object and never a secret. Read strictly: both members must be strings or the message is ignored and `onComplete` does not fire |
| `vpay:redirect` | child → parent | `{ url }` | The parent performs the top-level navigation, because the frame is sandboxed `allow-scripts allow-same-origin allow-forms` — **`allow-top-navigation` is withheld**, which makes this message a necessity rather than a convention |

The SDK's handler also checks `event.source === frame.contentWindow`: the origin
check alone would let two embedded checkouts on one merchant page resize and
complete each other.

`allow-same-origin` is required rather than a relaxation — without it the page
runs in an opaque origin and its `/v1/browser` requests carry `Origin: null`,
which the CORS layer refuses.

## Where the headers come from

`frame-ancestors` is **configuration, resolved server-side, before any script
runs.** The page's `middleware.ts` calls
`GET {VPAY_API_URL}/v1/browser/checkout/origins?key=…` — by publishable key
alone, because an origin is the merchant's own public website and the key
already names the tenant — and turns the answer into the header on the HTML
response.

It **fails closed four ways**, each of which is a test against the shipping
middleware: a missing key, a missing `VPAY_API_URL`, a failed lookup, and an
empty list all produce `frame-ancestors 'none'`. An empty `checkout_origins` is
the default, so a merchant that has configured nothing cannot be framed at all.

There are **two locks on framing, not one**: the header, which a browser
enforces before a pixel is painted, and the page's own comparison of its framer
against the same list, which it does on every `postMessage` and before it reads
any credential.

An entry must be the **canonical** spelling a browser compares against —
lower-cased host, IDNA-encoded to ASCII, default port elided — and a
non-canonical one is refused at boot rather than normalised, because
`https://Shop.example` and `https://shop.example:443` were silently dropped by
the page's own filter, leaving the merchant unable to embed with nothing to
read.

**The CSP is `frame-ancestors` and nothing else, and that is a stated gap.**
There is no `script-src`, `default-src`, `connect-src` or `form-action`: a
policy worth having needs a per-request nonce threaded through Next's inline
bootstrap scripts, and shipping a permissive `default-src` would read like a
content policy while forbidding nothing.

## What is not built, and what is not proven

- **No real rail.** Every payment a browser has completed through this page
  settled against a `wiremock/wiremock` host, including Orange's "hosted page",
  which is a WireMock mapping serving two links (D7). Nothing here shows Orange
  would accept a `return_url` it had not been told about.
- **No browser has been observed enforcing `frame-ancestors`.** Cypress strips
  `Content-Security-Policy` from every document it proxies, so the header is
  asserted **as the server sends it** (`cy.request`, out of the runner's Node
  process). What a browser *was* observed doing is the second lock: refusing an
  unregistered framer by the page's own origin check — proven origin-driven by
  registering the fixture's origin in the overlay and watching the same page
  render. The two are different mechanisms and this document does not let one
  stand in for the other.
- **No browser has been observed refusing to frame the hosted page.** Cypress's
  runner *is* a frame, and the rewrite that makes the hosted page work under it
  is exactly the one that would hide the refusal.
  `frontends/apps/checkout/src/lib/entry.test.ts` covers it in jsdom.
- **No pod has ever run the page.** The chart templates a Deployment, a Service
  and an Ingress under `checkout.enabled` (default false); the container has
  been run under the constraints the chart asks for, and nothing more. The
  path-prefix Ingress shape has been run by nobody.
- **No rate limiting.** This step added a second unauthenticated surface under
  the same operational requirement [`browser-checkout.md`](browser-checkout.md)'s
  D5 already stated: rate limiting belongs at the ingress, and nothing in this
  repository enforces or checks that an operator has done it.
- **No accessibility gate.** The screens have Storybook stories with the a11y
  addon configured, and nothing runs axe. What *is* asserted in vitest: every
  control is a native focusable element with an accessible name, the live region
  is mounted from first render, focus moves to the new screen's heading, and the
  MSISDN error is tied to its field.
- **`checkout_not_configured` answers `500`, not `503`.** A truthful `503`
  needs either a new `Category` or `Category::Configuration` moving — an
  ADR-level change to [ADR-0011](../adr/0011-error-modelling.md) touching every
  error in the workspace, **left to the maintainer**.
- **The auto-forward countdown is 5 seconds and not configurable.**

## What the horizon emits, and what it does not

A session the sweep expires produces one `events` row of type
`checkout.session.expired`, in the **same transaction** as the `open` ->
`expired` compare-and-swap (`vpay_db::CheckoutSessions::expire_due`,
migration `0029`), and from there it is an ordinary event: the fan-out creates
one `webhook_deliveries` row and one `deliver_webhook` job per endpoint the
merchant configured, signed and delivered on the same ladder as
`payment_intent.succeeded`, and readable at `GET /v1/events` and
`GET /v1/events/{id}` scoped to the merchant.

`data.object` is the **thirteen documented keys** — `status` already
`expired`, `payment_status` whatever the money did, and **`url: null`**.
A hosted session's `url` carries its `client_secret` in the fragment (D6),
and an event body is stored, signed, delivered at-least-once and replayed;
`client_secret` is absent entirely and `return_token` is on no wire object at
all. So a `null` `url` in an event does not mean the session was embedded —
read `ui_mode`.

The transaction is what matters here, not the event. A session that says
`expired` with no event is invisible: no sweep looks for one, no backlog names
it, and the merchant simply never hears. `a_failed_event_insert_leaves_the_session_open`
proves the flip rolls back with the insert, and the reverse — committing the
flip first — was measured failing it on 2026-09-04.

**Three transitions emit nothing, deliberately.** A settlement moving a
session to `complete`/`paid` or `expired`/`failed` already emits
`payment_intent.succeeded` / `payment_intent.payment_failed` from the same
commit, and a second event for one payment is a dedupe problem vpay would have
created. `POST /v1/checkout/sessions/{id}/expire` emits nothing because the
caller already knows — a narrower rule than Stripe's, recorded as an open
question in [webhooks.md](webhooks.md)'s "What is not built" rather than
decided here. And a session a rail is still holding is neither expired nor
evented, because the `NOT EXISTS` live-charge guard is a predicate of the
`UPDATE`.

**The sweep is now paged.** It reads at most `vpay_worker::handlers`'
`EXPIRY_PAGE` (100) due sessions a pass and reschedules itself immediately
when a page comes back full and something moved — the device
`vpay_worker::webhooks::handle_fan_out` already used — so a backlog drains
rather than waiting an hour a page. One session's failure is logged at `WARN`
naming the session, its merchant and no credential, and the pass moves on;
that session stays `open` and the next pass retries it. There is no attempt
counter, unlike `events.fanout_attempts` — and a failing session does **not**
get out of the way: it keeps its `status` and its horizon, and `due_for_expiry`
orders by `expires_at`, so it heads every subsequent page. What bounds it is
that the pass moves on: one poisoned session costs one `WARN` an hour and one
of a hundred slots. A hundred of them would fill the page, and because the
immediate reschedule is conditional on something having moved, the healthy
sessions behind them would wait an hour a pass. Nothing today makes a
deterministic per-session failure reachable; if one is ever found, the fix is
`events.fanout_attempts`' shape.

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

The completion signal does **not** come from vpay: a popup has no framer, so
the child channel in `frontends/apps/checkout` returns `null` and the page
says nothing. It comes from the merchant's own `success_url`, running inside
the popup. The design and its limits are in
[browser-checkout.md](browser-checkout.md)'s Status; the short version is that
34 unit cases drive it against stub windows and **no test opens a real
popup**.

See [../status.md](../status.md) for the per-feature ledger and the reasons
several of those rows are 🟡 where this document says "built".
