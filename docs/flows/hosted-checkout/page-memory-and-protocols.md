# Hosted checkout — page memory, the iframe protocol, the popup, and the headers

_Split out of [docs/flows/hosted-checkout.md](../hosted-checkout.md) on 2026-09-11 by exp57, which broke a 937-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

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
