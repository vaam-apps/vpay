# Outbound webhooks — Status: the events API, recovery, replay, and what is not built

_Split out of [docs/flows/webhooks.md](../webhooks.md) on 2026-09-11 by exp57, which broke a 811-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

**`GET /v1/events` and `GET /v1/events/{id}`** are mounted, merchant-scoped and
cursor-paged, for the merchant who missed a delivery
(`events_are_listed_newest_first_scoped_to_the_merchant`,
`reading_events_requires_a_scope`). The filter is **`merchant_id` only** —
`livemode` is not part of the query. One deployment is one `livemode` today
(it is a deployment setting, not a per-request one), so there is nothing to
separate; a deployment that ever served both would leak test events into a live
listing, and this is the sentence that says so before it happens.

**A lost job is recovered; an exhausted delivery is not.** The `scan:deliveries`
singleton (`JobKind::ScanDeliveries`, migration `0023`) walks
`vpay_db::WebhookDeliveries::pending_due` every **10 minutes**, up to **500
rows** a pass — the same interval and batch `scan:live` uses for charges — and
re-enqueues a `deliver_webhook` job for each row it finds. Two arms, and the
second is the one that took thought: a `pending` delivery whose
`next_attempt_at` has passed, **or** one that has never been attempted
(`next_attempt_at IS NULL`) and whose `created_at` is older than
`RecoveryPolicy::lease`. The lease is what keeps the second arm from racing the
queue on every freshly created delivery — the fan-out writes the row and its job
in one transaction, so a row younger than a lease is simply one whose job has
not been claimed yet. So a delivery whose job was **deleted**, or lost to a
`jobs` truncation, is picked up again without anyone noticing
(`the_backstop_re_enqueues_a_delivery_whose_job_vanished`,
`pending_due_returns_the_deliveries_nothing_is_driving`).

**It does not recover a delivery whose job was _dead-lettered_, and that is
deliberate.** `vpay_db::Jobs::dead_letter` parks the job at
`run_at = 'infinity'` and keeps its `dedupe_key`, so the scan's
`ON CONFLICT (dedupe_key) DO NOTHING` insert is a no-op for exactly those
rows: the delivery stays `pending` and no attempt is ever made. A
`deliver_webhook` job is parked only for a `Poisoned` reason — an event that
will not render, a body whose digest no longer matches what was signed — and
retrying fixes none of them, so a scan that un-parked it would re-run the same
failure every ten minutes forever. What the scan does instead is emit one
`WARN` per pass naming those deliveries, so the state has an observer; the
un-park is a manual `UPDATE` in
[../runbooks/webhook-delivery-failures.md](../../runbooks/webhook-delivery-failures.md)
(`a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan`). A pass that
_fails_ logs `ERROR … alert = true` before rescheduling on the backoff — a
backstop nobody notices has stopped is a backstop that is not there.

It also does **not** touch an `exhausted` row — that state is not `pending` —
which is why replaying one is still manual.

**Replaying an exhausted delivery is two writes, by hand.** There is no replay
endpoint and no CLI. An operator flips the row back to `pending` and re-enqueues
the job that drives it, in one transaction — the row alone would wait up to ten
minutes for the next `scan:deliveries` pass, and the job alone is refused by
`record_*`'s `state = 'pending'` guard:

```sql
BEGIN;
UPDATE webhook_deliveries
SET state = 'pending', attempt = 0, next_attempt_at = now()
WHERE id = '<delivery uuid>' AND state = 'exhausted';

INSERT INTO jobs (kind, dedupe_key, payload, run_at)
VALUES ('deliver_webhook', 'webhook:' || '<delivery uuid>',
        jsonb_build_object('delivery_id', '<delivery uuid>'), now())
ON CONFLICT (dedupe_key) DO NOTHING;
COMMIT;
```

Both statements were run against a `postgres:16-alpine` with every migration
through `0022` applied, and run twice: the `state = 'exhausted'` guard and the unique
`jobs_dedupe_key` make the second run `UPDATE 0` / `INSERT 0`. `attempt = 0`
restores the whole ladder; leaving `attempt` alone grants exactly one further
attempt, because `delivery_delay(8)` is `None`. `payload_sha256` is left in
place on purpose — it is the digest of the bytes the first signed attempt
signed, and clearing
it would silence the check that catches a renderer changing under a live
delivery. The full procedure, the diagnosis queries and what _not_ to do are in
[../runbooks/webhook-delivery-failures.md](../../runbooks/webhook-delivery-failures.md).
**The transaction is proven to run and to leave the right rows; no replayed
delivery has been observed reaching a receiver.**

**What is not built.**

- **The egress guard covers webhook delivery and nothing else.** The rail
  adapters are not behind it: `providers[].host` is operator-configured, not
  merchant-supplied, and `validate_host` already refuses a stub host in
  livemode. If a rail host ever becomes merchant-supplied, `vpay_worker::ssrf`
  moves to `vpay-provider` and both callers use it.
- **A receiver behind NAT64 (`64:ff9b::/96`) is refused**, even when the IPv4
  address it embeds is public, because the guard treats everything outside
  IPv6 global unicast as non-public rather than guessing at IANA's
  special-purpose space. That is fail-closed and has never been met in
  practice; it is written down so it is a decision rather than a surprise.
- **Pinning cost the shared connection pool.** One client per delivery, one
  handshake per delivery to the same receiver. Unmeasured under load.
- **No deployment has ever refused a real merchant's endpoint.** The evidence
  for all of the above is nine unit cases, two container-backed cases against a
  real receiver, and a revert proof in which bypassing the classifier makes the
  private delivery `succeed` — not production.
- **No `?type=` filter** on `GET /v1/events`. Unknown query parameters are
  ignored by every handler on this surface, so it is accepted and has no
  effect; [../api/README.md](../../api/README.md) says so where the route is
  documented.
- **An SSRF-refused delivery is exhausted on its first attempt, and there is
  no replay path — F5, found 2026-09-04 by Step 8's correctness review and
  deliberately not fixed.** An egress refusal is permanent by design
  (`state = 'exhausted'` on attempt 1, no next attempt), and replay is the
  hand-written transaction in the runbook, so a transiently poisoned DNS answer
  — or a receiver behind a resolver that briefly returns a private address —
  destroys the event with nothing to re-drive it. "Fail closed" and "destroy
  the event" are the same thing while replay does not exist. The remedy is a
  design decision about a merchant-visible delivery state machine (a replay
  path, or a retryable `ssrf_blocked` state with a bounded ladder) and belongs
  with whoever owns this document; **lane H's recommendation** is to treat the
  _resolution_ half the way an unresolvable host is already treated — an
  ordinary failed attempt on `delivery_delay` — and keep the permanent refusal
  for an address that classifies as private on every attempt of the ladder.
  That distinction is already made once in this code
  (`a_host_that_resolves_to_a_private_address_is_refused_and_an_unresolvable_one_retries`),
  which is why it is worth naming rather than inventing.
- **No replay endpoint and no CLI.** `scan:deliveries` recovers a _deleted or
  lost_ job; it resurrects neither an `exhausted` delivery nor one whose job
  was dead-lettered (its `dedupe_key` is still held by the parked row), and
  nothing re-arms a `failed` event. All three are the manual `UPDATE`s in
  [../runbooks/webhook-delivery-failures.md](../../runbooks/webhook-delivery-failures.md),
  and all three need a `psql` prompt.
- **No ordering guarantee**, and nothing that could provide one. See above.
- **vpay never tells the merchant a delivery failed.** There is no
  `webhook.failed` event, no email and no dashboard view; `exhausted` is a log
  line with `alert = true` and a row, and so is a `failed` fan-out.
  `GET /v1/events` is the merchant's own fallback, and they have to poll it —
  and an event abandoned at `fanout_state = 'failed'` is one they can still
  read there, which is the only reason abandoning it is defensible at all.
- **No deployment has ever produced a `failed` event or a parked delivery
  job.** Both states are proven by integration tests against a real Postgres
  (`a_permanently_unfannable_event_is_abandoned_after_five_passes_and_alerts_once`,
  `a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan`); the runbook
  procedures for re-arming them have not been followed against a running
  system.
- The event types this document lists that nothing writes at all are
  **two**, plus the two refund types: `payment_intent.created` and
  `payment_intent.processing`. Events are written for terminal transitions
  only (decision 4 of `docs/plans/2026-09-03-step4-worker.md`), and those two
  are progress. _(This bullet named `payment_intent.canceled` as a third
  until 2026-09-10; it has a writer now — see the table above and
  [issue #57](https://github.com/vaam-apps/vpay/issues/57). It was the one
  type in this vocabulary that predated migration `0023`'s lockstep rule and
  never acquired a writer, which is why it sat here for a week of releases.)_
- **`customer.created` and `customer.updated` were not in the vocabulary at
  all** (2026-09-06, S4a) — a different and stronger statement than the four
  above, because the database refuses a type no code writes. **Closed
  2026-09-10** by migration `0039` and
  [issue #66](https://github.com/vaam-apps/vpay/issues/66), in the same change
  that made both routes transactional. A merchant mirroring their customers
  externally no longer has to poll `GET /v1/customers`.
- **No deployment has ever emitted a `customer.created` or a
  `customer.updated` either.** Both are proven against a real Postgres through
  the shipping router (`a_customer_create_and_update_each_emit_one_event_and_a_no_op_emits_none`,
  `two_concurrent_metadata_merges_keep_both_keys_and_the_event_carries_the_committed_state`)
  and neither has been fanned out to a receiver in any suite — the fan-out is
  type-agnostic and has been observed for other types, which is an argument
  and not a measurement.
- **No deployment has ever emitted a `customer.deleted`.** The event, its
  fan-out and its delivery rows are proven against a real Postgres through the
  real worker loop by
  `the_sweep_deletes_an_idle_unreferenced_customer_and_anonymises_a_referenced_one`
  with a horizon that suite controls, and through the shipping router by
  `a_customer_with_payment_history_is_anonymised_rather_than_deleted`; no vpay
  has been up for twelve months, and no merchant endpoint has received one.
- **`DELETE /v1/customers/{id}` emitted nothing at all until 2026-09-10**
  ([issue #96](https://github.com/vaam-apps/vpay/issues/96) item 2). Only the
  retention sweep wrote `customer.deleted`, so a merchant who deleted a
  customer by hand learned about it from the response to their own request and
  from nowhere else — and any other service of theirs subscribed to the event
  learned nothing. The route writes it in the delete's own transaction now.
- **A merchant expiring its own session emits nothing.** `POST
/v1/checkout/sessions/{id}/expire` moves the row and writes no event, so a
  merchant whose own systems are the ones that need telling has to tell them.
  The argument for the current shape is that the caller already knows; the
  argument against is that a merchant with several services does not
  necessarily, and Stripe emits `checkout.session.expired` for both paths.
  Not an oversight — the 2026-09-04 change that added the event was scoped to
  the sweep, which is the path nobody is watching — and **left to whoever owns
  this document**, because "one transition, one event" is a contract merchants
  build dedupe logic on and widening it later is cheaper than narrowing it.
- **No `checkout.session.completed`.** A session reaching `complete` already
  produces `payment_intent.succeeded` from the same commit, and a second event
  for one payment is a dedupe problem vpay would have created. A merchant that
  wants the session object reads it.
- **No deployment has ever delivered a `checkout.session.expired` to a
  merchant endpoint.** The event, its fan-out, its one delivery row per
  configured endpoint and its `deliver_webhook` jobs are proven against a real
  Postgres by `an_expiry_sweep_emits_one_event_and_one_delivery_per_endpoint`;
  the endpoints in that case are URLs nothing resolves, because what it
  asserts is what the fan-out _created_. The delivery half is the same code
  every other event walks, and that has been observed against a WireMock
  receiver — but not for this type.

Why the delivery code is shaped the way it is — the digest invariant, the two
failure recorders, the fan-out's per-event transaction, and what the backstop
may and may not share — is
[../reference/vpay-worker.md](../../reference/vpay-worker.md); the tables' own
reasoning is [../reference/vpay-db.md](../../reference/vpay-db.md).

See [../status.md](../../status.md).
