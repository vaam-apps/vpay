# `checkout.session.expired` — build notes (tier: opus)

Branch `claude/exp-session-expired-opus`, base `d0b602e` (Step 9). Worked
2026-09-04.

`docs/status.md` was **not** edited while the change was being built, per the
task brief; the rows it would carry were transcribed verbatim below. **They
have since been applied**, in the landing pass that rebased this branch onto
`master` — with the sweep row additionally naming the current
`due_for_expiry(now, limit)` / `expire_due(id, now, event_id, event_data)`
shape (the row named the pre-change signature and `verify-status` cannot see a
stale symbol) and carrying the confirm-after-expiry limitation and the
unmeasured `expires_at` index as named, dated limitations.

---

## What landed

| # | Change | Where |
|---|---|---|
| 1 | Migration `0029`: `type_is_a_documented_event` reopened for the eighth type; `events.type` and `events.object_id` comments re-issued | `backends/migrations/0029_events-checkout-session-expired.sql` |
| 2 | `CheckoutSessions::due_for_expiry(now, limit)` — the read half of the sweep, same guard, `ORDER BY expires_at`, bounded | `backends/crates/vpay-db/src/checkout_sessions.rs:660` |
| 3 | `CheckoutSessions::expire_due(id, now, event_id, event_data)` — the compare-and-swap **and** the `events` insert, one transaction; `EVENT_SESSION_EXPIRED` is the module's own constant | `backends/crates/vpay-db/src/checkout_sessions.rs:756`, impl at `:987` |
| 4 | `CheckoutSessionObject::expired_snapshot(row)` — the 13 keys, `status: "expired"`, `url: None`, no credential; `EXPIRED` const | `backends/crates/vpay-api/src/model.rs:447`, `:526` |
| 5 | `sweep_expired` gains `expire_due_sessions` / `expire_one_session` / `SweptSessions`, `EXPIRY_PAGE = 100`, and a full-page reschedule | `backends/crates/vpay-worker/src/handlers.rs:122`, `:875`–`:1035` |
| 6 | `KnownEventType` (`#[non_exhaustive]`, `as_wire_str`/`from_wire`) and `Event::checkout_session()` | `sdks/rust/src/model.rs:453`, `:594`; exported at `sdks/rust/src/lib.rs:66`, `:110` |
| 7 | `KnownEventType` union member and `isCheckoutSessionEvent` guard | `sdks/nodejs/src/types.ts:105`, `:150`; exported at `sdks/nodejs/src/index.ts:60` |
| 8 | Two parity rows plus the shape-vs-capability note | `docs/sdks/parity.md:20`, `:112` |
| 9 | Docs: event catalogue, both TX 1s, the lifecycle, the reference pages | `docs/flows/webhooks.md`, `docs/flows/hosted-checkout.md`, `docs/reference/vpay-db.md`, `docs/reference/vpay-worker.md` |
| 10 | Measurement comment; **`expected_suites` and `min_tests` unchanged** | `justfile:585` |

### Shape, and the one place the brief and the codebase disagree

The brief asks for the event **and the delivery jobs** "in the same
transaction as the status flip". The codebase's outbox is deliberately
two-step (`docs/flows/webhooks.md`, "Two-step outbox"): TX 1 writes the
`events` row beside the state change with `fanout_state = 'pending'`, and TX 2
(`fan_out_events`) creates the `webhook_deliveries` rows and their
`deliver_webhook` jobs. Fan-out inline with a business transaction would make
that transaction depend on reading the endpoint table, and the document says
why that is the wrong trade.

So this is built **exactly as `payment_intent.succeeded` is**, which is what
the rest of the brief's sentence asks for: the event row is in the same
commit as the flip, and the deliveries come from the same fan-out, the same
signing and the same ladder. Test 1 asserts the deliveries and their jobs
exist after the drain has run, which is the property that matters — the same
transaction is not what makes them exist, the event row's existence is.

### Why the bulk `UPDATE` became a page of transactions

`events.data` is the *rendered* wire object, and only `vpay-api` can render
it, so the row has to be read before the write that describes it — the order
`Settlement::apply_succeeded` and `handlers::intent_snapshot` have had since
Step 4. That makes the old single unbounded `UPDATE … RETURNING`-less
statement impossible. Consequences, all documented in place:

- `due_for_expiry` takes a `limit` (`EXPIRY_PAGE = 100`) where the statement
  had none. The old one cost one number however many rows it touched; this one
  materialises rows carrying two live payer credentials each.
- The sweep reschedules itself immediately when a page came back full **and**
  something moved — `handle_fan_out`'s exact device and condition. The other
  three housekeeping statements run again with it; three bounded deletes is
  cheaper than a fifth `jobs.kind` and its migration.
- The live-charge guard is evaluated **twice** — once in the read so a session
  a rail is holding is never rendered, once in the write because the read's
  answer is stale the moment it returns.

---

## Measured

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo nextest run -p vpay-db -p vpay-api -p vpay-worker -p vpay-core -p vpay-sdk` | see the report |
| `cargo nextest run -p vpay-tests-integration -E 'binary(checkout_sessions) \| binary(webhooks) \| binary(worker_recovery)' --retries 2 -j 1` | see the report |
| `pnpm --filter @vpay/sdk test` | 172 passed, 0 skipped (was 168) |
| `pnpm --filter @vpay/sdk typecheck` | clean — two of the Node assertions are type-level and only this command makes them |
| `just verify` | all four gates pass |
| `just verify-ignored` | **1146 total, 42 test binaries, 0 ignored** (was 1137) |
| `just test-doc` | **86 passed, 1 ignored** (was 84; the ignored one is `sdks/rust`'s README block, pre-existing) |

Nine new Rust cases, all in files that already existed, so `expected_suites`
stays 42. `min_tests` stays 1080 on the justfile's own stated terms (lane E
and lane H both declined to move it for eight and five cases respectively);
the measurement is recorded in the comment above it.

---

## Guard-failure proofs

Both run against a real Postgres container, both restored afterwards, and
`git status` is clean of them. Both were re-run and reproduced by the review
on 2026-09-04, along with six more mutations — see
`docs/plans/step9-notes/session-expired-review.md`, which also records the two the tests
did **not** catch and the commits that fixed them.

| # | Sabotage | Case | Result |
|---|---|---|---|
| 1 | Delete the `events::insert_in_tx` call from `CheckoutSessions::expire_due` (wrapped in `if false`) | `an_expiry_sweep_emits_one_event_and_one_delivery_per_endpoint` | **FAIL** — `Error: exactly one checkout.session.expired was expected: []`. The session still expires; nothing else changes. That is the silent regression the case exists for |
| 2 | Move the flip outside the transaction — run the `UPDATE` on `self.pool`, open the transaction afterwards for the event alone | `a_failed_event_insert_leaves_the_session_open` | **FAIL** — `left: ("expired", "unpaid")`, `right: ("open", "unpaid")`, "the flip must have rolled back with the event: a session that says `expired` with no event is one no merchant will ever be told about" |

After restoring, the whole `checkout_sessions` binary is **23 passed, 0
skipped**.

---

## The `docs/status.md` rows, verbatim

These are the rows as drafted. They were applied in the landing pass, with the
additions named at the top of this file.

### Replace the existing row at `docs/status.md:1380`

```
| Checkout session expiry sweep (`vpay_db::CheckoutSessions::{due_for_expiry,expire_due}`, `vpay_worker::handlers::sweep_expired`) | ✅ | **New 2026-09-04 (Step 9, lane 1b).** `checkout_sessions.expires_at` was written at create (24 h, D10) and read by nothing; a session past its horizon reported `status: open` until a merchant expired it by hand or the intent settled. The worker's existing hourly housekeeping job now expires them — `open` and past `expires_at` and **no live charge** → `expired`, `payment_status` untouched — and logs its count as `checkout_sessions` beside `idempotency_keys`, `client_assertion_jtis` and `expired_leases`. **No new `jobs.kind`**: it is the same schedule as the other three, and a fifth kind would have said nothing this one does not. The live-charge guard is a `NOT EXISTS` inside the `UPDATE`, over the same `LIVE_CHARGE_STATES` `expire` and `cancel` use, because a session whose payer confirmed seconds before the horizon has a rail holding a live payment and a background job that expired it would be contradicted by the settlement transaction minutes later. Proven by `the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one` (`backends/tests/integration/tests/checkout_sessions.rs`): two sessions created through `POST /v1/checkout/sessions`, one with a live charge opened through the browser confirm the page uses, both moved past their horizon, swept by the shipping `vpay_worker::run_once` over the shipping `seed_singletons` job. Measured 2026-09-04: deleting the `NOT EXISTS` clause expires the paying session. **Changed 2026-09-04: it emits an event, and it is no longer one statement** — see the `checkout.session.expired` row below. Because the event's `data` is the rendered wire object, which only `vpay-api` can shape, the sweep now reads a page (`due_for_expiry`, `EXPIRY_PAGE` = 100, ordered by `expires_at`), renders each session, and runs one transaction per session; a full page that moved something reschedules the sweep immediately rather than in an hour, the device `webhooks::handle_fan_out` already used. One session's failure is a `WARN` naming it and its merchant and no credential, and the pass moves on — that session stays `open` and the next pass retries it; only a failure to read the page is a `JobError`. **What it is not:** the sweep is not what refuses an expired session's payer credential — the browser reads check `expires_at` themselves, because the sweep leaves live-charge sessions `open` on purpose and a deployment whose worker is down must not keep answering |
```

### Insert a new row beside it

```
| `checkout.session.expired` (migration `0029`, `vpay_db::CheckoutSessions::expire_due`) | 🟡 | **New 2026-09-04.** The eighth documented event type, and the first whose `data.object` is neither a `payment_intent` nor a `refund`. Migration `0029` reopens `type_is_a_documented_event` for it; it is Stripe's own spelling, so [flows/webhooks.md](flows/webhooks.md)'s "only real Stripe event types" rule holds and a Stripe-shaped handler already has a branch. **Written inside the same transaction as the `open` → `expired` compare-and-swap** — the settlement's shape, and the argument is sharper here: a session that says `expired` with no event is invisible, because no sweep looks for one, no fan-out backlog names it, and the merchant simply never hears. `a_failed_event_insert_leaves_the_session_open` proves the rollback against a real `data_is_object` CHECK violation, and the reverse was measured on 2026-09-04: committing the flip before the insert makes it fail with the session `expired`. `data.object` is the **thirteen documented keys** rendered by `vpay_api::model::CheckoutSessionObject::expired_snapshot` — `status` already `expired`, `payment_status` untouched, and **`url: null`**, because a hosted session's `url` carries its `client_secret` in the fragment (D6) and an event body is stored, signed, delivered at-least-once and replayed; `client_secret` is absent entirely and `return_token` is on no wire object at all. Both the unit case and the integration case assert that on the **serialised string**, not the parsed object. From there it is an ordinary event: one `webhook_deliveries` row and one `deliver_webhook` job per configured endpoint, the same signing, the same egress guard, the same seven-rung ladder, and readable at `GET /v1/events` and `GET /v1/events/{id}` merchant-scoped. Seven container-backed cases in `backends/tests/integration/tests/checkout_sessions.rs`: `an_expiry_sweep_emits_one_event_and_one_delivery_per_endpoint`, `a_second_sweep_writes_no_second_expiry_event`, `a_session_with_a_live_charge_is_neither_expired_nor_evented`, `a_session_finished_by_settlement_emits_no_expiry_event`, `an_expiry_event_is_listable_and_retrievable_within_its_tenant`, `a_failed_event_insert_leaves_the_session_open` — all six driving the shipping `vpay_worker::run_once` over the shipping `seed_singletons` — and, added by the 2026-09-04 review, `a_payer_confirming_between_the_read_and_the_write_keeps_the_session`, which stages the window between `due_for_expiry` and `expire_due` and proves the live-charge `NOT EXISTS` in the *write* is load-bearing: without it all 23 cases in the file were green. Measured 2026-09-04: deleting the event insert makes the first fail while the session still expires. Both merchant SDKs carry the type — `vpay_sdk::KnownEventType::CheckoutSessionExpired` with `Event::checkout_session()`, `@vpay/sdk`'s `KnownEventType` with `isCheckoutSessionEvent` — and both keep `type` a plain string, so an event type either predates is still deliverable ([sdks/parity.md](sdks/parity.md)). **🟡, for three reasons.** (1) **No merchant endpoint has ever received one.** The endpoints in the proving case are URLs nothing resolves, because what it asserts is what the fan-out *created*; the delivery half is the same code every other event walks and has been observed against a WireMock receiver, but not for this type. (2) **A merchant expiring its own session through `POST /v1/checkout/sessions/{id}/expire` emits nothing.** The caller already knows — but a merchant with several services does not necessarily, and Stripe emits it for both paths. Deliberate and scoped to the sweep, which is the path nobody is watching; **left to the maintainer**, because "one transition, one event" is a contract merchants build dedupe logic on and widening it later is cheaper than narrowing it. (3) **No `checkout.session.completed`**, and there should not be one: a session reaching `complete` already produces `payment_intent.succeeded` from the same commit |
```

### Amend the row at `docs/status.md:1411` ("Events written by the worker")

Its opening reads "Two types, both from …". It becomes three, and the closing
"**Still 🟡:** `payment_intent.created`, `.processing` and `.canceled`, and
both refund types, are written by nothing" is unchanged and still true — five
of the eight are written by nothing. Verbatim replacement for the two
sentences that change:

```
Three types, all from [flows/webhooks.md](flows/webhooks.md)'s Stripe list — `payment_intent.succeeded` and `payment_intent.payment_failed` written **inside** the settlement transaction, and (2026-09-04, migration `0029`) `checkout.session.expired` written inside the housekeeping sweep's per-session transaction — with `fanout_state = 'pending'`, one per terminal transition and never two (decision 4 of the Step 4 plan: terminal transitions only).
```

---

## What I did **not** do

- **Did not edit `docs/status.md`** while building. The brief forbade it; the
  rows are above and were applied in the landing pass.
- **Did not emit an event for `POST /v1/checkout/sessions/{id}/expire`.** The
  brief scopes the emission to the sweep ("when the expiry sweep moves a
  session … and only then"). Stripe emits for both paths, so this is a real
  narrowing; it is recorded as an open question for the maintainer in
  `docs/flows/webhooks.md`'s "What is not built" and in the status row above,
  not left implicit.
- **Did not deliver one to a receiver.** No case POSTs a
  `checkout.session.expired` to a WireMock endpoint and reads it back out of
  the journal, the way `webhooks.rs` does for `payment_intent.succeeded`. The
  delivery path is byte-identical code — the same `EventObject` renderer, the
  same signer, the same client — but "byte-identical by construction" is the
  argument `docs/flows/webhooks.md` itself retired for the Stripe signature,
  and I have not made the observation for this type. It is the honest reason
  the new status row is 🟡 rather than ✅.
- **Did not add an attempt counter for an unexpirable session**, the way
  `events.fanout_attempts` bounds an unfannable event. `due_for_expiry` pages
  a hundred wide ordered by `expires_at`, so a poisoned session does not head
  every subsequent page; it costs one `WARN` an hour. If that turns out to be
  wrong the fix is `events.fanout_attempts`' shape, and the reasoning is
  written down in `docs/reference/vpay-worker.md` so the decision is visible.
- **Did not touch `sdks/stripe-compat`.** It is evidence, not an SDK, and
  ADR-0015 gives it no rows; proving `stripe.webhooks.constructEvent` accepts
  a `checkout.session.expired` delivery would need one to have been delivered,
  which is the gap above.
- **Did not add a `?type=` filter** to `GET /v1/events`. Still ignored, still
  documented as such.
- **Did not run the Cypress e2e suites or `just demo`.** Neither is in the
  brief's gate list and neither exercises this path.
