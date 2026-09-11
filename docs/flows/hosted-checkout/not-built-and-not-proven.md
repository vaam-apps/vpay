# Hosted checkout — what is not built, what is not proven, and what the horizon emits

_Split out of [docs/flows/hosted-checkout.md](../hosted-checkout.md) on 2026-09-11 by exp57, which broke a 937-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

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
  the same operational requirement [`browser-checkout.md`](../browser-checkout.md)'s
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
  ADR-level change to [ADR-0011](../../adr/0011-error-modelling.md) touching every
  error in the workspace, **left to the maintainer**.
- ~~**The auto-forward countdown is 5 seconds and not configurable.**~~
  **Retired 2026-09-06:** there is no countdown. See "The outcome screens".
- **No POD has run the page with a mounted `branding.yaml` or `config.yaml`,
  and the chart still cannot supply them.** The parsers, the colour conversion
  and the filesystem layer are unit-tested (57 cases across `src/config/`).
  _Corrected 2026-09-07:_ this bullet claimed "the container was run by hand
  with them" and cited
  [`../plans/exp21-checkout-page-notes/opus.md`](../../plans/exp21-checkout-page-notes/opus.md),
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
question in [webhooks.md](../webhooks.md)'s "What is not built" rather than
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
