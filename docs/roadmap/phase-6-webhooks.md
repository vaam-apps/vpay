# Roadmap — Phase 6: Webhooks

_Moved out of [docs/roadmap.md](../roadmap.md) on 2026-09-11 by exp57, which split a 1 504-line page into one page per phase. **The text below is the original, unedited** — every struck-through claim, every "Open —" maintainer question and every dated amendment is here as it was written, because on this page those are most of the content; only the relative links gained a `../` because the file moved one directory down._

## Phase 6 — Webhooks

**~~Candidate, not decided (2026-09-02):~~ Decided 2026-09-03 — keep the
in-repo outbox; `cratestack-outbox` is not adopted.** The question this
paragraph reserved was whether to take `cratestack-outbox` (which implements
exactly this shape — `OutboxClient::persist_in_tx` inside the caller's
transaction, `drain` in insertion order, an axum drain/gc handler pair, and no
schema macro) or to write the ~200-line outbox by hand. **The answer is the
hand-written one**, in migrations `0021`/`0022`: the `jobs` table already
provides `FOR UPDATE SKIP LOCKED` claims, leases guarded on `locked_by`, a
reaper, dead-letter parking and a dedupe key, and `vpay_db::TxRepositories::insert_in_tx`
already writes the outbox row in the same transaction as the charge. Adopting
`cratestack-outbox` would duplicate all of that — two drain paths, two lease
mechanisms, two sets of metrics for one queue — and add a dependency in the
money path that has to be justified through `deny.toml`. What is given up is
real: vpay now owns this code forever, and the drain is one more thing to
maintain rather than to upgrade.

**This decision was taken by an agent under the user's standing delegation
("take all decisions yourself"), not by the maintainer who reserved it.** It is
reversible — the drain is `vpay_worker::webhooks::handle_fan_out`, one function
behind one job kind — and the maintainer may reopen it. ADR-0006's "no
in-process fake receiver" rule applied either way and still does.

**Goal.** Merchants receive signed webhook notifications for intent/charge
state changes, delivered at-least-once via a durable outbox.

**Status. 🟡 Delivered 2026-09-03 (Step 5), against a WireMock receiver.** Both
transactions of [`docs/flows/webhooks.md`](../flows/webhooks.md)'s outbox run: the
settlement transaction writes the `events` row (Step 4), and the `fan_out_events`
singleton (`fanout:events`, seeded at every worker boot) turns it into one
`webhook_deliveries` row and one `deliver_webhook` job per configured endpoint,
in one transaction per event. `deliver_webhook` renders the event through the
same `vpay_api::model::EventObject` that `GET /v1/events` serves, signs those
exact bytes with HMAC-SHA256 over `"{t}.{body}"`, and POSTs them under
`Vpay-Signature` and `Stripe-Signature`. ~~`just demo`'s seventh step ends with
a signed `payment_intent.succeeded` read back out of a WireMock receiver's own
request journal and verified with the shipping SDK.~~ **Corrected 2026-09-04
(Step 8, lane A): the walkthrough is four steps, and step 4 reads a signed
webhook back out of the WireMock receiver's own request journal — one per
outcome, six in all — verifying each with the shipping SDK and asserting its
`type` against what that outcome must produce.**

**No merchant endpoint has ever been POSTed to.** Every delivery observed so
far went to a WireMock host on a compose network, on a developer machine and in
CI. That is why this phase is 🟡 and not ✅, and it is the same limit Phases 4
and 5 carry about rails.

> **Addendum, 2026-09-03 (evening) — two follow-ons landed on top of this
> phase, and neither changes anything above.** Step 5b (PR #20) proved a
> delivered webhook verifies through the **official `stripe` package**'s
> `webhooks.constructEvent`, against bytes read back out of the receiver's own
> request journal rather than bytes vpay believes it sent, and refuses both a
> body with one byte flipped and the right body under the wrong secret
> ([flows/stripe-sdk-compat.md](../flows/stripe-sdk-compat.md)). Step 5c (PR #22)
> added the browser surface a payer's page polls while a delivery is in flight
> ([flows/browser-checkout.md](../flows/browser-checkout.md)). Both are rows in
> the table at the top of this page rather than scope here. **The receiver is
> still a WireMock host, delivery is still unordered, and there is still no
> SSRF protection of any kind.** _(**Corrected 2026-09-04, Step 8 lane B:** the
> last clause is retired — `vpay_worker::ssrf` guards every delivery. The other
> two stand.)_

**Scope, and what became of each item.**

- ✅ An outbox row written in the same transaction as the state change it
  reports — `vpay_db::TxRepositories::insert_in_tx`, inside
  `vpay_db::Settlement::apply_succeeded`/`apply_failed` (Step 4).
- ✅ A signing scheme matching Stripe's — `t=…,v1=…` over `"{t}.{body}"`,
  emitted under both header names, verified by the two shipping SDKs against
  bytes this server actually sent.
- ✅ A delivery worker with a retry policy — `vpay_worker::delivery_delay`
  (10 s, 30 s, 2 m, 10 m, 1 h, 6 h, 24 h), then `state = 'exhausted'` with an
  `alert = true` log line.
- ✅ A backstop behind the queue — `scan:deliveries` (migration `0023`), every
  10 minutes over up to 500 rows, re-enqueueing a `deliver_webhook` job for any
  `pending` delivery nothing is driving.
- 🟡 Absorbed and only half-built: **the operator side.** There is no replay
  endpoint and no CLI, and the backstop cannot resurrect an `exhausted`
  delivery — re-sending one is a hand-written transaction, now written down in
  [`docs/runbooks/webhook-delivery-failures.md`](../runbooks/webhook-delivery-failures.md).
- ⛔ Not in scope and still unbuilt: five of the seven documented event types
  (`payment_intent.created`, `.processing`, `.canceled` and the two refund
  types), and the `?type=` filter on `GET /v1/events`.

**Definition of done — met.** `worker_e2e.rs` proves a settlement commits its
`events` row in the same transaction as the charge and that the same loop drains
it; `backends/tests/integration/tests/webhooks.rs` proves signature verification
against a real HMAC — twice, once through the Rust SDK and once through the Node
SDK in a subprocess — driven against a WireMock receiver started by
`vpay_testkit`, never an in-process fake
([ADR-0006](../adr/0006-no-mocks-in-main-processes.md)). The Node case **fails
rather than skips** when `node` is absent; CI sets `VPAY_REQUIRE_NODE=1`.

**Unblocks.** Nothing further downstream on the payments path — this is the
last functional phase before the MVP claim in `docs/status.md` can move.

**Risks carried by this phase.**

- ~~**No SSRF protection at all, and `validate_host` is not any.**~~
  **Closed 2026-09-04 (Step 8, lane B), and what replaces it is narrower.**
  Endpoint URLs are still checked at boot only, and that check is still a
  scheme rule plus four host substrings that never looks at an address — it
  never could, because an address is not a property of a configuration file.
  What changed is that the **address is now checked at delivery**:
  `vpay_worker::ssrf` resolves the host once, classifies every address the
  lookup answered with, refuses the delivery permanently if any of them is
  non-public, and pins the client to exactly those addresses with
  `reqwest::ClientBuilder::resolve_to_addrs` — which is the third answer the
  Step 5 plan's decision 4 did not consider, and it needs no custom connector.
  **Three residuals remain and are risks this phase still carries:** the guard
  is on webhook delivery only; a NAT64 receiver (`64:ff9b::/96`) is refused
  fail-closed; and the pin cost the shared connection pool, one handshake per
  delivery, unmeasured under load. And **no deployment has ever refused a real
  merchant's endpoint.**
- **Delivery is unordered.** Concurrent claims and the retry ladder can reorder
  two of one merchant's events; merchants must dedupe on `event.id` and reason
  from `event.created` and the object's own `status`, never from arrival order.
  Nothing in the design provides ordering and nothing is planned to.
- **A webhook secret is a forgery key.** Whoever holds one can sign a
  `payment_intent.succeeded` a merchant's handler will believe. Rotation exists
  (two secrets, two `v1=` values) but needs a deploy, and there is no
  revocation short of one.
- **The ladder's later rungs have never elapsed.** 1 h, 6 h and 24 h are
  asserted as values by a unit test; no deployment has waited them out.
- Unchanged: the standing no-mocks constraint above.

---
