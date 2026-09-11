# Outbound webhooks — Status: the two transactions, delivery and signing

_Split out of [docs/flows/webhooks.md](../webhooks.md) on 2026-09-11 by exp57, which broke a 811-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

**TX 1 — the business transaction.** `vpay_db::Settlement::apply_succeeded` /
`apply_failed` move the charge, move the intent and insert one `events` row in
a single transaction, with `fanout_state = 'pending'`. Two types from this
document's list — `payment_intent.succeeded` and
`payment_intent.payment_failed` — and the CHECK `type_is_a_documented_event`
(migrations `0018` and `0029`) refuses anything else at the database.

**There is a second TX 1, and it is the same shape.**
`vpay_db::CheckoutSessions::expire_due` moves one checkout session from `open`
to `expired` and inserts its `checkout.session.expired` row in one
transaction, once per session, called by the housekeeping sweep
(`vpay_worker::handlers::sweep_expired`). The argument for it being one
transaction is the settlement's, sharpened: a session that says `expired` with
no event is one **nothing would ever notice** — there is no sweep over
"expired sessions with no event", no fan-out backlog entry naming it, and the
merchant simply never hears. `a_failed_event_insert_leaves_the_session_open`
(`backends/tests/integration/tests/checkout_sessions.rs`) is the proof, and it
is a real refusal from a real CHECK rather than a seam: measured 2026-09-04,
committing the flip before the insert makes it fail with the session
`expired`.

The flip is a compare-and-swap on `status = 'open'` **and** the horizon
**and** a `NOT EXISTS` over the live charge states, so a second sweep emits no
second event, a session a rail is still holding is neither expired nor
evented, and a session the settlement transaction already finished is left
alone — it has had its `payment_intent.*` event for the same thing happening,
and a second one would be a duplicate vpay invented. `POST
/v1/checkout/sessions/{id}/expire`, the merchant's own abandon, emits nothing
either; see "What is not built".

**TX 2 — the fan-out.** `vpay_worker::webhooks::handle_fan_out` is the
`fan_out_events` job: a singleton (`fanout:events`) seeded beside
`sweep:expired`, `scan:live` and `scan:deliveries` by
`vpay_worker::run_loop::seed_singletons`,
rescheduled every 5 s, or immediately when its page came back full. It reads
`vpay_db::Events::pending_page`, and per event, in **one transaction**, inserts
a `webhook_deliveries` row per configured endpoint, enqueues one
`deliver_webhook` job per row and flips `fanout_state` to `done`. Crash
idempotency is the unique index `webhook_deliveries_event_endpoint` plus
`jobs_dedupe_key`, absorbing the replay a crash produces
(`fan_out_creates_one_delivery_and_one_job_per_endpoint_and_is_idempotent`). A
merchant with **zero** endpoints still flips to `done`, or the partial index
`events_pending_idx` grows without bound
(`an_event_for_a_merchant_with_no_endpoints_is_still_fanned_out`). **One bad
event does not stop the page:** a failure on a single event is logged at `WARN`
— naming the event, its merchant, its type, its attempt count and no secret —
and the pass moves on; the page ends with a `WARN` summarising how many drained
and how many failed. The failing event keeps `fanout_state = 'pending'`, so the
next pass retries it, and a pass that drained _nothing_ waits the idle interval
rather than rescheduling immediately — otherwise a page of failures would spin.
Aborting the whole page instead — what an earlier shape did — let one merchant's
unfannable event hold up every other merchant's webhooks behind it
(`one_merchants_unfannable_event_does_not_block_another_merchants`). And
`worker_e2e.rs`'s `wait_for_fanout` proves the loop that settles a charge is
the loop that drains it.

**An event that can never be fanned out is abandoned after five passes, and
alerts once.** Isolating the failure is not enough on its own: `pending_page`
orders by `seq`, so a permanently unfannable event heads _every_ subsequent
page — re-alerting every five seconds and holding one of the page's hundred
slots forever, and a hundred of them stop the drain for everyone. So each
failure increments `events.fanout_attempts` (migration `0024`, in its own
statement — the event's own transaction has rolled back), and the fifth
(`vpay_worker::FANOUT_MAX_ATTEMPTS`) sets `fanout_state = 'failed'`. `failed`
is not `pending`: the event leaves `events_pending_idx`, leaves
`pending_page`, and stops being retried. Exactly **one**
`ERROR … alert = true` is emitted, at the transition — so a page of 99
poisoned events costs 99 alerts in total rather than 99 every pass
(`a_permanently_unfannable_event_is_abandoned_after_five_passes_and_alerts_once`).
The cost is the honest one: a `failed` event is a webhook the merchant will
never receive, and **nothing resurrects it** — re-arming one is a deliberate
`UPDATE` after the cause is fixed
([../runbooks/webhook-delivery-failures.md](../../runbooks/webhook-delivery-failures.md)).

**Delivery.** `handle_deliver` renders the event through
`vpay_api::model::EventObject` — the _same_ renderer `GET /v1/events` returns,
so the delivered body and the API's answer cannot disagree — signs those exact
bytes and POSTs them with `Content-Type`, `Vpay-Signature`, `Stripe-Signature`
and `Vpay-Event-Id`. **`Stripe-Signature` carries the same string as
`Vpay-Signature`, byte for byte, in Stripe's documented `t=…,v1=…` grammar** —
an integration test asserts both, that the two headers are equal and that the
value parses as that grammar. **Since Step 5b it is also verified with the real
`stripe` package**: `sdks/stripe-compat`'s `webhooks.compat.test.ts` makes a
payment against the compose stack, waits for the worker to settle it, pulls the
resulting delivery out of the WireMock receiver's own request journal, and hands
the recorded bytes and `Stripe-Signature` to
`stripe.webhooks.constructEvent` — then flips one byte of the payload, and a
second time uses the wrong secret, and requires
`StripeSignatureVerificationError` for both. So "a Stripe-shaped handler works
unmodified" is an observation now, not an argument from the scheme being
identical. The body is not stored; `payload_sha256` is written on the
first attempt and compared on every later one, and a mismatch is
`JobError::Poisoned`. **There is exactly one place vpay clears that digest on
purpose** (2026-09-11): erasing a customer rewrites `events.data` for every
`customer.*` body of that payer, which changes the bytes a delivery already
mid-ladder would re-render, so
`vpay_db::customers::erase_in_tx` clears `payload_sha256` on the deliveries
of those events that are still `pending` or `failed` — in the same
transaction — and the next attempt signs and sends the redacted body.
Without it the guard dead-letters exactly the delivery that tells the
merchant the erasure happened, blaming a renderer change that did not
happen. `succeeded` and `exhausted` rows keep their digest: nothing
re-renders them, and it is the record of what a merchant was actually sent.
See [customers.md](../customers.md) § "A delivery already in flight" and
`an_erasure_mid_ladder_redelivers_the_redacted_body_instead_of_dead_lettering`.
Non-2xx and transport failures walk
`vpay_worker::delivery_delay` — the seven rungs above, rung by rung — and the
eighth failure is `state = 'exhausted'` with an `alert = true` log line, never
another rung (`the_ladder_walks_delivery_delay_and_then_succeeds`,
`a_delivery_past_the_last_rung_is_exhausted_and_not_rescheduled`).

**A delivery that got no answer records why, not just that.** A transport
failure — DNS, connect, TLS, the request deadline — is stored with
`status_code IS NULL` (the encoding for "the request went out and nothing came
back") and a `response_excerpt` reading `no response: <error>: <source chain>`.
The chain was added in Step 7 for the reason `jobs.last_error` carries one
(ADR-0011's amendment): a `reqwest` error's own `Display` names the URL an
operator already has and keeps "connection refused" in its `source()`. The
excerpt is bounded by migration `0022`'s `excerpt_length` CHECK either way.

**Acknowledge first, then work.** The delivery client is
`vpay_provider::http::client_pinned_to`, built per delivery from
`vpay_worker::ssrf`'s vetted addresses over the same two budgets
(`WEBHOOK_CONNECT_TIMEOUT` = 5 s to connect, `WEBHOOK_REQUEST_TIMEOUT` = **10 s
for the whole request**), with redirects refused and the proxy environment
ignored — and it reads at most 8 KiB of the acknowledgement body,
which nothing parses. A receiver that finishes its own processing before
answering turns a slow database into a failed delivery. Any `2xx` is success;
a `3xx` arrives as an ordinary non-2xx failed attempt, because following it
would replay a signed event body at a host the operator never configured.

**Signing, proven against the SDKs a merchant installs.** The header is fed
straight to `vpay_sdk::webhooks::verify_at`
(`the_delivered_signature_verifies_with_the_shipping_rust_sdk`, which also flips
one byte of the recorded body and requires `SignatureMismatch`) and to the
built `@vaam-apps/vpay-sdk` in a `node` subprocess
(`the_delivered_signature_verifies_with_the_shipping_node_sdk` — it **fails**
rather than skips when `node` is missing; CI sets `VPAY_REQUIRE_NODE=1`). Two
configured secrets produce exactly two `v1=` values and either one verifies
(`a_rotation_signs_with_both_secrets_and_either_one_verifies`).

**And against the SDK vpay does not ship.** Step 5b added
`sdks/stripe-compat`'s `webhooks.compat.test.ts`, which runs out of process
against the compose stack: it makes a payment through the official `stripe`
package, waits for the worker to settle it, reads the delivery out of the
WireMock receiver's request journal (`GET /__admin/requests` — the
merchant-side view, not vpay's own tables) and calls
`stripe.webhooks.constructEvent(body, headers['stripe-signature'], secret)`.
The recorded bytes go in verbatim; a parse-and-reprint would be verifying a
body vpay never sent. Both refusals are asserted too — one flipped byte of the
payload, and the right body with the wrong secret — because a verifier that
accepted everything would have accepted the delivery as well. That is what
retires the "byte-identical by construction, unobserved in practice" caveat
this section used to carry.
