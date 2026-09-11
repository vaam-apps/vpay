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
                    │ the payer presses "Back to {merchant}"
                    ▼
               forwarding ──► success_url / cancel_url (hosted)
                              return_url + vpay:complete (embedded)
```

**Nothing on that diagram happens on a timer.** The one arrow out of
`outcome` is a button press. See "The outcome screens" below.

The return page is its own document with its own smaller machine: it holds no
intent secret, so it has **no confirm transition at all** — it polls the return
route until the intent is terminal, shows the outcome, and forwards.

Three properties worth stating because each is a test:

- **The embed check runs before the credential is read.** A framer that is not
  on the merchant's list is refused even when the URL carries no key and no
  secret, so the refusal cannot be used to probe which half of a link is wrong.
- **The language switch does not navigate.** A `?lang=fr` link has no fragment,
  and resolving a fragment-less relative URL _drops_ the current one — which on
  this page is the session's credential. The server picks the initial locale
  from `Accept-Language` (French by default: Cameroon first, and Orange's own
  page is French by default), and the switch swaps the dictionary in place.
- **No floating point anywhere in the money path.** `5000 XAF` becomes the
  string `"5000"` by moving a decimal point through the integer's digits, and
  `Intl.NumberFormat` formats that string. `minor / 100` would be float
  arithmetic in a money path, which the Rust half of this repository denies
  workspace-wide ([money.md](money.md)).

## The outcome screens

_Changed 2026-09-06 (the maintainer's requirement of 2026-09-05)._

Every outcome — succeeded, failed, canceled — ends in **one button, named
"Back to {merchant}"**, and nothing else. There is no countdown, no timer,
and no navigation this page performs on its own. Both modes render the same
screen; what differs is what the button does, which is what already differed:

|                          | Hosted                                                                     | Embedded                                                            |
| ------------------------ | -------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| The button               | `location.assign(success_url \| cancel_url)`, top-level                    | posts `vpay:complete` to the framer, then navigates to `return_url` |
| Where the URL comes from | `forwardKindFor` — `success_url` on a paid session, `cancel_url` otherwise | `return_url`, for every outcome                                     |
| Session names nowhere    | no button; "This payment is finished. You can close this page."            | the same                                                            |

The button carries the merchant's name — `Back to Boutique Test` — and falls
back to "Back to the shop" where the session read carried no usable name, the
same `_unnamed` treatment every other merchant sentence on this page gets.

It replaced a **five-second auto-forward** that had been here since Step 9. A
page that navigates on its own takes the outcome away from the payer who is
reading it, and on the failure screen it takes away the only text that says
why — on a handset, in a shop, with someone waiting. That the button was
always there beside the countdown did not fix it: it made the countdown a
race a slow reader loses.

**A failed outcome is red.** _Corrected 2026-09-07._ The screen took its
colour from `@vpay/tokens`' `statusTone` through the intent status each
outcome implies, and a failed attempt leaves the intent at
`requires_payment_method` — which is `neutral`, because on a **dashboard**
that status means "awaiting a payment method" rather than "this failed". The
mapping was accurate and the screen was wrong: a payer whose payment failed
read a grey box while a payer who cancelled read a red one. An operator's
status palette is not a payer's outcome palette, so `@vpay/tokens` now carries
both — `statusTone` unchanged, and `checkoutOutcomeTone` for these three
screens. _Updated 2026-09-07 (decision D4,
[2026-09-07-ui-revamp.md](../plans/2026-09-07-ui-revamp.md) §9):_ `canceled`
now tones `warning`, not `error` — the design call this paragraph used to
defer is taken, on the reasoning that a payer's own cancellation is not the
same event as a payment that failed for a reason outside their control.
`failed` still tones `error`. The token moved in `@vpay/tokens`
(`frontends/packages/tokens/src/index.ts`); `OutcomePanel`
(`frontends/apps/checkout/src/components/screens.tsx`) already carries a
`warning` entry in its `TONE_CLASS` lookup, so the colour renders correctly
with no change to this app — the migration of that lookup onto `@vpay/ui`'s
`Alert` component is separate work, not yet done (`docs/plans/2026-09-07-ui-
revamp.md` §4.1).

**A failed outcome also shows the rail's own words** where the API gave any.
`last_payment_error.message` is rendered as _data_, under the translated
sentence and labelled as the provider's ("What the payment provider said"),
never in place of it: it arrives in whatever language the rail writes in,
it is outside the closed `FailureCode` vocabulary the page translates, and
this page controls not a word of it. `providerReason` strips control
characters, collapses whitespace and bounds it at 300 characters; React
escapes the rest, so it reaches the DOM as a text node and never as markup,
an attribute or a URL.

## Runtime configuration

_New 2026-09-06 (the maintainer's requirement of 2026-09-05)._

**One image serves every operator.** The page's brand and its feature flags
are two YAML files an operator mounts, read at container start — not a
`tailwind.config.ts` edit, not a build argument, and not `NEXT_PUBLIC_*`
inlined by `next build`.

| File            | Default path                       | Environment override          |
| --------------- | ---------------------------------- | ----------------------------- |
| `branding.yaml` | `/etc/vpay/checkout/branding.yaml` | `VPAY_CHECKOUT_BRANDING_FILE` |
| `config.yaml`   | `/etc/vpay/checkout/config.yaml`   | `VPAY_CHECKOUT_CONFIG_FILE`   |

Worked examples with every key commented are checked in at
[`../../config/checkout/branding.example.yaml`](../../config/checkout/branding.example.yaml)
and [`../../config/checkout/config.example.yaml`](../../config/checkout/config.example.yaml),
and `compose.demo.yml` mounts both.

```yaml
# branding.yaml
display_name: Vaam Payments          # the OPERATOR's name, never the merchant's
logo_url: https://cdn.example/logo.svg
primary_color: "#f3c623"             # #rrggbb, six digits
support_contact: support@vaam.example
```

```yaml
# config.yaml
checkout:
  public_base_url: https://checkout.example
  allowed_methods: [mtn_momo, orange_money]
  features:
    page_memory: true
```

**Injected at startup, not fetched.** `src/config/runtime.ts` reads both files
once, on the first import in the server process, and memoises the result; each
server component passes the parts it needs to the client as props. A payer's
request never reads a file, and the operator's colour arrives in the same
response as the markup — there is no frame in which a payer sees the default
and then watches it change. A config change is a pod restart, which is what
ADR-0003 already says about administration.

`GET /config/v1` reports exactly what the container loaded, versioned in the
path. It is a **verification surface for an operator**, not something the page
uses: every value on it is already visible to any payer who opens the page. The
file paths and the problem list are deliberately _not_ on it — those are in the
container log, where the person who can act on them is looking.

**A missing file is a log line and defaults, never a blank page.** Absent,
unreadable, not YAML, a top level that is not a mapping, a key of the wrong
type, a value that fails its rule: each costs exactly that key, prints one
`WARN` naming the file and the key, and leaves the rest standing.

```
[vpay-checkout] configuration: no file at /etc/vpay/checkout/branding.yaml — using defaults
[vpay-checkout] configuration: config.yaml: checkout.allowed_methods is empty — ignored, every rail stays on offer
```

Three rules are worth stating because each is a decision rather than a
default:

- **`display_name` is the operator's, and never substitutes for the
  merchant's.** What a payer is told they are paying comes from the session
  (`merchant: { name }`, from the API's `merchant_clients[].display_name`).
  Where that is absent the page still says the neutral sentence; putting the
  operator's name there would name the wrong party.
- **`allowed_methods` can only narrow, and never silently.** A rail the intent
  offers and the operator excludes is listed as unsupported — the same
  treatment D9 gives a rail this page has no flow for. An **empty** list is
  refused with a `WARN` rather than honoured, because honouring it would
  refuse every payment on the deployment.
- **`primary_color` retints the theme at runtime.** daisyUI compiles a theme
  into `--p: L% C H` custom properties, so one `:root[data-theme="bumblebee"]`
  block in `<head>` is enough. `src/config/theme.ts` converts sRGB to OKLCh
  with Ottosson's matrices and daisyUI's own foreground rule — no colour
  library on a payment page — and its tests assert the output against values
  produced by **daisyUI's own converter** for six colours, so a drift is a
  failing test rather than a page that is quietly the wrong colour. The
  colour that goes _on_ the primary is derived, not configured, so a
  combination nobody can read text on is not one this file can produce.

## Page memory, and the PIN vault that is not here

_New 2026-09-06 (the maintainer's requirement of 2026-09-05)._

The page may remember **two values on the payer's own device**: the canonical
MSISDN they last paid with, and the rail they last chose. One IndexedDB
database, one object store, one key. It never leaves the browser — not to
vpay, not into a URL, not through `postMessage`.

- **Opt-in, and written at exactly one moment**: a payer ticks the box and
  submits an entry screen. Visiting the page, choosing a rail or reading an
  outcome stores nothing. Unticking the box on a later payment clears the
  record, and so does the explicit "forget" control.
- **The remembered rail is marked, never chosen.** It shows a "Last used"
  badge in the selector. Advancing a payer past a screen they have not read
  is the mistake the countdown was.
- **Ninety days**, enforced on read rather than by a sweep.
- **A stored number is re-validated with `normalizeCameroonMsisdn` before it
  reaches the form**, because a payer who does not re-read a prefilled field
  would send a push to whatever was there.
- **`page_memory: false` removes the offer entirely** — no checkbox, no read,
  no write.
- **The cost is stated on the checkbox**, in the payer's own language, not in
  a tooltip: _"Anyone else who uses this device will see it. Do not tick this
  on a shared or borrowed phone."_ A household handset, a borrowed phone, a
  phone shop's demo unit — the copy survives closing the tab, and the person
  who pays that cost is whoever picks the device up next.

**There is no PIN vault, and that is a refusal rather than an omission.** The
maintainer's requirement asked for one: an opt-in local store for a payer's
PIN, recalled into "the same form field the payer would type into". _This page
has no such field, and neither does anything behind it._ MTN's flow is a push
the payer approves on their handset; Orange's is approved on Orange's own
page; `POST /v1/browser/payment_intents/{id}/confirm` accepts
`payment_method_data[mtn_momo][msisdn]` and nothing else, and no
`ProviderAdapter` in this workspace takes a PIN. Building the vault would have
meant **adding a PIN input to vpay's checkout page that goes nowhere** — a
control that looks like it does something and does not, which is the first
item in this repository's own list of ways to damage it, and, on a payment
page, a field indistinguishable from phishing. **Left to the maintainer**: if
a rail is ever integrated that takes a PIN through the API, the store, the
opt-in, the horizon and the clear control are all here and a `pin` member is
the change.

## The iframe protocol

`postMessage`, with `event.origin` checked on both sides against a pinned value
— the parent against `new URL(baseUrl).origin`, the child against the allowed
origin that framed it. **`'*'` appears nowhere as a target**, and the parent
posts nothing into the frame at all: the child learns its framer's origin from
the CSP vpay served it, not from a message, so the strongest form of "never
`postMessage(…, '*')`" is "never `postMessage`".

| Message         | Direction      | Payload               | Why                                                                                                                                                                                                                                |
| --------------- | -------------- | --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `vpay:resize`   | child → parent | `{ height }`          | The frame is created at `height: 0`; the page owns its height and sends this on first paint and on every `ResizeObserver` callback. A height the SDK guessed would be an iframe silently stuck at the wrong size                   |
| `vpay:complete` | child → parent | `{ session, status }` | `session` is the `cs_…` **id string**, never the object and never a secret. Read strictly: both members must be strings or the message is ignored and `onComplete` does not fire                                                   |
| `vpay:redirect` | child → parent | `{ url }`             | The parent performs the top-level navigation, because the frame is sandboxed `allow-scripts allow-same-origin allow-forms` — **`allow-top-navigation` is withheld**, which makes this message a necessity rather than a convention |

The SDK's handler also checks `event.source === frame.contentWindow`: the origin
check alone would let two embedded checkouts on one merchant page resize and
complete each other.

`allow-same-origin` is required rather than a relaxation — without it the page
runs in an opaque origin and its `/v1/browser` requests carry `Origin: null`,
which the CORS layer refuses.

## The popup, and why it is a third peer

_New 2026-09-06, at the request of the `examples/shop` track._

A merchant may open the **hosted** page in a popup — `window.open(session.url)`
— rather than framing it or navigating to it. That is a third shape, and it
broke the protocol silently: **a popup is not a frame.** Inside one,
`window.parent === window`, so `createFrameChannel` returned `null` and vpay
said nothing at all to the merchant's page when the payment finished.

The channel now takes a `peer`, `parent` or `opener`, and the popup case pins
`window.opener` to an origin resolved **exactly the way a framer's is**:
`document.referrer` matched against the merchant's `checkout_origins`, never
used to extend them. `'*'` is still not a target anywhere.

|                           | Framed (`/e/{id}`)                                       | Popup (`/c/{id}`)                                                    | Top-level (`/c/{id}`) |
| ------------------------- | -------------------------------------------------------- | -------------------------------------------------------------------- | --------------------- |
| Peer                      | `window.parent`                                          | `window.opener`                                                      | none                  |
| An unresolvable peer      | **refused** — a page with no parent has no way to finish | renders, with no channel                                             | normal                |
| `vpay:resize`             | yes                                                      | **no** — a popup sizes itself                                        | n/a                   |
| `vpay:redirect`           | yes — a sandboxed frame may not navigate the top level   | **no** — see below                                                   | n/a                   |
| `vpay:complete`           | yes                                                      | yes                                                                  | n/a                   |
| "Back to {merchant}" does | posts `vpay:redirect`                                    | posts `vpay:complete` (if it has not already), then `window.close()` | `location.assign`     |

Three of those rows are deliberate departures from what was asked for, and
each is a decision rather than an omission:

- **No `vpay:redirect` to an opener.** A popup _is_ a top-level browsing
  context and may navigate itself. Asking the opener to navigate would send
  **the merchant's own page** to Orange Money out from under the payer, losing
  the page they expect to come back to. So the popup navigates itself to the
  rail, and the rail redirects back into the popup.
- **`vpay:complete` is posted at most once per page.** Two moments can reach
  it — the outcome first appearing, and the payer pressing the button — and a
  second copy would fire a merchant's `onComplete` twice for one payment. A
  merchant that treats that handler as a cue to create an order would create
  two.
- **An opener that does not resolve is not a refusal.** A hosted page is a
  complete page on its own: it takes the payment and sends the payer to
  `success_url` in this window. What it loses is the ability to _tell_ the
  opener, which is a degraded integration rather than an unsafe one. The
  embedded case still refuses, because a framed page with no parent has no way
  to finish at all. The commonest cause is the merchant's own
  `Referrer-Policy: no-referrer`.

If the opener has gone — the payer closed the merchant's tab while paying —
the button navigates this window to `success_url`/`cancel_url` instead of
closing it, so nobody is left looking at a dead popup. `Window.closed` is
readable cross-origin, so that is a real check.

`middleware.ts` now resolves the origin list for `/c/{cs_id}` as well as
`/e/{cs_id}`, because the popup needs one. **Its CSP does not change**: the
hosted page is `frame-ancestors 'none'` whatever the lookup returned, and the
two uses of the list are separate expressions so that widening one cannot
widen the other. `the hosted page > NEVER lets that list reach its CSP` is the
test, and it was measured failing with the guard removed.

**That lookup is a new cost on the hosted page, and it is worth naming.**
Before this, `/c/{cs_id}` reached vpay's API from the server not at all: it
rendered, and the browser did the rest. It now makes the same one call the
embedded page makes, on every hosted page load that carries a `key`, whether
or not the page turns out to be in a popup — the server cannot know, because
`window.opener` is a fact only the browser has. `fetchCheckoutOrigins` catches
every failure and answers an empty list, so a vpay API that is down or
refusing costs the hosted page **no channel** and nothing else. _Corrected
2026-09-07:_ this paragraph used to end by naming a gap — no timeout on that
fetch, so a hanging API would hold the payment page's first byte. There is one
now. `ORIGINS_TIMEOUT_MS` is two seconds, overridable per call so the budget is
a test rather than a wait, and the abort lands in the same empty list every
other failure does. Two cases in `middleware.test.ts` drive it with a `fetch`
that never answers; removing the signal makes the first hang.

**The popup's return trip is wired by a different rule** (the maintainer's
decision, 2026-09-06). After a redirect rail sends the payer back to
`/c/{id}/return`, that page's referrer is the _rail's_ origin, so
`resolveParentOrigin` has nothing to match and the popup would end with the
merchant's window hearing nothing. So the return page resolves its opener
with `soleOrigin` instead: **when the merchant has registered exactly one
`checkout_origins` entry, that is the target; with none or with several,
there is no channel.**

The reasoning, in the maintainer's own terms: with one registered origin the
`postMessage` target _is_ the merchant's own origin, which is the only party
the message could ever have been for — so the worst case is a message
delivered to its intended reader. With two or more, picking one would be
choosing a target by guess, on a page that has just come back from a third
party, and the page stays silent instead.

Everything else about the popup holds here unchanged: `vpay:complete` at most
once, the button closes the window, and an opener that has gone means the page
navigates itself to `success_url`/`cancel_url` rather than closing. `soleOrigin`
counts **after** normalising, so one malformed registered origin is none rather
than one to pin to. `middleware.ts` resolves the list for `/c/{id}/return` as
well, and — the third path it now does that for — **its CSP is still
`frame-ancestors 'none'`**.

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
  process). What a browser _was_ observed doing is the second lock: refusing an
  unregistered framer by the page's own origin check — proven origin-driven by
  registering the fixture's origin in the overlay and watching the same page
  render. The two are different mechanisms and this document does not let one
  stand in for the other.
- **No browser has been observed refusing to frame the hosted page.** Cypress's
  runner _is_ a frame, and the rewrite that makes the hosted page work under it
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
  addon configured, and nothing runs axe. What _is_ asserted in vitest: every
  control — **native or not** — has an accessible name and is in the tab order,
  the live region is mounted from first render, focus moves to the new screen's
  heading, and the MSISDN error is tied to its field. _Corrected 2026-09-07:_
  this bullet said "every control is a native focusable element", which stopped
  being true when the memory opt-in became Base UI's checkbox — a
  `<span role="checkbox" tabindex="0">` with a visually hidden input beside it.
  The test asserts the property rather than the tag: role, tab index,
  accessible name, `aria-checked`, and the label click and space key both
  measured. **The bumblebee theme's contrast has been checked by nobody**:
  `@vpay/ui`'s Storybook runs axe's `color-contrast` rule against `corporate`
  and `business`, which this app does not use, and that Storybook is not part
  of `just ci` either.
- **`checkout_not_configured` answers `500`, not `503`.** A truthful `503`
  needs either a new `Category` or `Category::Configuration` moving — an
  ADR-level change to [ADR-0011](../adr/0011-error-modelling.md) touching every
  error in the workspace, **left to the maintainer**.
- ~~**The auto-forward countdown is 5 seconds and not configurable.**~~
  **Retired 2026-09-06:** there is no countdown. See "The outcome screens".
- **No POD has run the page with a mounted `branding.yaml` or `config.yaml`,
  and the chart still cannot supply them.** The parsers, the colour conversion
  and the filesystem layer are unit-tested (57 cases across `src/config/`).
  _Corrected 2026-09-07:_ this bullet claimed "the container was run by hand
  with them" and cited
  [`../plans/exp21-checkout-page-notes/opus.md`](../plans/exp21-checkout-page-notes/opus.md),
  which says the opposite — "no container was run with the mounted files, and
  no pod ever". What is true now is stronger than either: `compose.demo.yml`
  mounts the two examples, and `just test-e2e` brought that stack up and drove
  all four Cypress specs green through it, so the mounted path is the path the
  browser tests run on. The **chart** templates no ConfigMap for either file,
  so a Kubernetes deployment has no supported way to supply them yet.
- **Nothing has watched a real browser's IndexedDB.** Page memory's adapter is
  driven in vitest by a hand-written `IDBFactory` stub
  (`src/testing/idb-stub.ts`), which models the object graph the adapter
  walks and nothing else — no transaction lifetime, no quota, no versions
  above 1.
- **`@base-ui-components/react` is pinned at `1.0.0-rc.0`**, which is the
  latest release that package has: there is no 1.0.0. A release candidate on
  a payment page is a maintainer's call and is recorded as one.
- **The Storybook stories are reviewed under the wrong theme.** The shared
  Storybook (`frontends/packages/ui/.storybook`) configures `corporate` and
  `business`; this app ships `bumblebee` alone. The stories show layout and
  copy honestly and colour only approximately.

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
