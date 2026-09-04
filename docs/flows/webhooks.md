# Outbound webhooks

Stripe's scheme, copied exactly, so merchants' existing verification code works.

**Header:** `Vpay-Signature: t=1753401600,v1=<hex hmac>`
**Signed payload:** `"{timestamp}.{raw_body}"`, HMAC-SHA256, hex-encoded.
Constant-time comparison; reject a timestamp older than 5 minutes.

## Only real Stripe event types

`payment_intent.created`, `payment_intent.processing`,
`payment_intent.succeeded`, `payment_intent.payment_failed`,
`payment_intent.canceled`, `charge.refunded`, `charge.refund.updated`,
`checkout.session.expired`.

A custom type is silently dropped by any merchant using `stripe-node`'s typed
event union or an exhaustive `switch`. This is why a late success emits a plain
`payment_intent.succeeded`: an event merchants structurally tend to ignore is
the worst possible carrier for "money actually arrived".

**Three of the eight are written, and only three.** `payment_intent.succeeded`
and `payment_intent.payment_failed` come from the settlement transaction (TX 1
below); `checkout.session.expired` comes from the housekeeping sweep, since
2026-09-04. The other five are documented shapes nothing emits — events are
written for terminal transitions only.

**`checkout.session.expired` is the only one whose `data.object` is not a
`payment_intent` or a `refund`.** It carries a `checkout.session`: the thirteen
keys `docs/flows/hosted-checkout.md` documents, with `status` already
`expired`, `payment_status` whatever the money did, and `url` **always
`null`** — a hosted session's `url` carries its `client_secret` in the
fragment (D6), and a webhook body is stored, signed, delivered at-least-once
and replayed on every rung of the ladder. So `url: null` in an event does
**not** mean the session was embedded; read `ui_mode`. `client_secret` is
absent entirely, as is the `return_token`, which is a column and on no wire
object at all. Both merchant SDKs carry the type in their vocabulary
(`vpay_sdk::KnownEventType::CheckoutSessionExpired`, `@vpay/sdk`'s
`KnownEventType`) with a narrowing accessor for the payload
(`Event::checkout_session`, `isCheckoutSessionEvent`), and both keep working
unchanged for a type they do not know: `type` is a `string` in both, not the
union.

The `object_id` on such a row is the `cs_…`, which made it the fourth prefix
that polymorphic column carries.

## Two-step outbox

```
TX 1 (the business transaction)
  UPDATE payment_intent SET status='succeeded'
  INSERT event (fanout_state='pending')

TX 2 (fan-out)
  scan events WHERE fanout_state='pending'
  INSERT webhook_delivery per matching endpoint
  UPDATE event SET fanout_state='done'

delivery, with retries: 10s → 30s → 2m → 10m → 1h → 6h → 24h
```

Both steps matter. Fan-out inline with the state change would make the business
transaction depend on reading the endpoint table. Fan-out without a
`fanout_state` column would leave no way to *find* events never fanned out.
Either mistake produces a succeeded payment with no webhook.

That ladder is **8 POSTs over about 31 hours** — the first attempt plus seven
retries, 112,360 seconds of waiting in total — and then the delivery is
`exhausted`. Every non-2xx walks the whole of it, `4xx` included: a receiver
answering `410 Gone` is retried for 31 hours exactly as a `500` is. That is
Stripe's behaviour too, and it is deliberate — a `404` from a receiver that is
mid-deploy is indistinguishable from one that means "stop", and stopping early
on the wrong one loses the event.

**Delivery is at-least-once, and its order is not guaranteed.** Merchants must
dedupe by `event.id`, and must **not** assume that two events for one merchant
arrive in the order they happened. The fan-out preserves `seq` order when it
*creates* the jobs, and nothing preserves it afterwards: N claim tasks take N
different jobs concurrently (`FOR UPDATE SKIP LOCKED`), and one delivery that
fails drops to the next rung of the ladder while later ones go out immediately.
A receiver that decides state from arrival order will settle a payment from a
stale event. `event.created` and the object's own `status` are what to reason
from.

## Status

**Updated 2026-09-04: the housekeeping sweep is a third writer.** A checkout
session passing its 24-hour horizon with nothing driving it now produces one
`checkout.session.expired` — in the same transaction as the status flip — and
that event goes through the identical fan-out, signing, delivery client,
egress guard and retry ladder every other event does, because it is one more
row in the same table. Migration `0029` is what let the database accept the
type; nothing else in this document changed. `docs/flows/hosted-checkout.md`'s
"An expired session notifies nobody" is retired by it.

**Both transactions are real, and a signed webhook has been delivered to a
WireMock receiver and verified with both shipping SDKs — and, since Step 5b,
with the official `stripe` package. No merchant endpoint has ever been POSTed
to.** Updated 2026-09-03 (Step 5, then Step 5b). The receiver is a host
in configuration, reached over HTTP exactly as a merchant's endpoint would be
(ADR-0006) — which is the same limit the rails carry, and the reason
[../status.md](../status.md)'s Webhooks row is 🟡.

**Updated 2026-09-04 (Step 8): since this step every delivery goes through the
egress guard first** (`vpay_worker::ssrf`), including the ones in the compose
stack — the sandbox profile *permits* its private receiver explicitly rather
than the guard being absent. "No merchant endpoint has ever been POSTed to"
stands, and so does its corollary: **no deployment has ever refused one
either.**

**Updated 2026-09-04 (Step 9): the receiver in the demo stack is now a real
merchant handler, and the sentence above is narrowed rather than retired.**
`examples/shop` exposes `POST /api/vpay/webhook`, which verifies the
`Vpay-Signature` header with `@vpay/sdk`, dedupes by event id, marks an order
`paid` on `payment_intent.succeeded` and `failed` on
`payment_intent.payment_failed`, and answers `2xx` only after the write. It is
the first thing in this repository's history to *act* on a delivery rather than
record it in a journal, and lane 6's Cypress specs assert an order reaching
`paid` **only** through it — the payer's return page reads the shop's database
and takes no decision from the return trip. What has still never happened is a
POST to a merchant endpoint **outside this repository**: `vpay-shop` is a
container on the same compose network, permitted by the sandbox profile's
`webhooks.allow_private_targets`, and it is code this repo wrote and tests.

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
next pass retries it, and a pass that drained *nothing* waits the idle interval
rather than rescheduling immediately — otherwise a page of failures would spin.
Aborting the whole page instead — what an earlier shape did — let one merchant's
unfannable event hold up every other merchant's webhooks behind it
(`one_merchants_unfannable_event_does_not_block_another_merchants`). And
`worker_e2e.rs`'s `wait_for_fanout` proves the loop that settles a charge is
the loop that drains it.

**An event that can never be fanned out is abandoned after five passes, and
alerts once.** Isolating the failure is not enough on its own: `pending_page`
orders by `seq`, so a permanently unfannable event heads *every* subsequent
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
([../runbooks/webhook-delivery-failures.md](../runbooks/webhook-delivery-failures.md)).

**Delivery.** `handle_deliver` renders the event through
`vpay_api::model::EventObject` — the *same* renderer `GET /v1/events` returns,
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
`JobError::Poisoned`. Non-2xx and transport failures walk
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
built `@vpay/sdk` in a `node` subprocess
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

**Endpoints are configuration, never a resource.** `merchant_clients[].webhooks[]`
in YAML (ADR-0003), keyed for fan-out on `merchant_id` and not on `client_id`;
each carries an operator-authored `id`, unique within a merchant and refused at
boot, stored verbatim on every delivery row so a URL correction does not orphan
the history. Secrets are covered by **two** livemode rules, and they are a
pair: the literal-secret rule reads the file's *text* and says the value came
from the environment, and a 32-byte floor reads the *resolved* value and says
the environment holds something worth having — an HMAC-SHA256 key shorter than
the hash's own output adds nothing over a 32-byte one and is what makes offline
guessing cheap (`ConfigError::WeakWebhookSecret`,
`a_livemode_webhook_secret_below_the_floor_is_refused`). Only the resolved value
can answer the second, because `${MERCHANT_WEBHOOK_SECRET}` is a placeholder of
fixed length whatever it holds. In sandbox neither applies and the rule is only
"not blank". There is no `/v1/webhook_endpoints` and no `webhook_endpoints`
table.

**What boot-time URL validation actually checks — and what it does not.** Every
endpoint URL is parsed once and then goes through
`vpay_config::validate_webhook_url`, plus shape checks. The
`id` is 1–64 characters and the `url` 1–2048, **in both modes**, counted in
characters exactly as migration `0022`'s `endpoint_id_length` and `url_length`
CHECKs count them — so a document the database would refuse is refused at boot
instead, and the constants are pinned against the migration by
`the_length_bounds_are_migration_0022s`. The URL must also parse, must not
carry embedded credentials (`https://user:pw@…`) and **must name a host — in
both modes**, so a `file:///var/spool/…` or a `mailto:ops@example` is refused
at the boot that introduced it rather than discovered as a delivery walking the
ladder to `exhausted` in a sandbox. `validate_webhook_url` itself is **two
rules and only two**, both livemode-only: the scheme must be `https`
(compared as a scheme, so `HTTPS://Hooks.Example/x` is fine), and the **host**
must not contain any of four substrings — `wiremock`, `stub`, `mock`,
`localhost`. The host and nothing else: `https://hooks.example/mockups` is a
merchant's own path and is accepted, `https://mock.example/x` is not
(`a_livemode_endpoint_may_be_uppercase_and_may_have_a_stub_word_in_its_path`,
and the refusal table beside it). It is a sibling of the rails'
`validate_host` rather than the same function, because that one's substring
tests are right for a bare origin and wrong for a URL. **Neither ever inspects
the destination address.** Under `livemode: false` neither rule applies.

So `https://127.0.0.1/hook`, `https://10.0.0.5/hook` and
`https://169.254.169.254/latest/meta-data/…` all boot cleanly in livemode.
**This is not SSRF protection and must not be described as any.** It is a guard
against shipping a stub host into production, which is a different problem.
What stops those three being *delivered to* is the next section, and it is a
different mechanism at a different time.

**The egress guard, at delivery time.** `vpay_worker::ssrf` runs on every
attempt, immediately before the socket and after the body has been rendered but
before it is signed. It parses the URL and refuses any scheme but
`http`/`https`; resolves the host **once**, with `tokio::net::lookup_host`;
classifies **every** address that lookup returned — loopback, unspecified,
RFC 1918, IPv6 unique-local `fc00::/7`, link-local `169.254.0.0/16` and
`fe80::/10`, CGNAT `100.64.0.0/10`, multicast, broadcast, `0.0.0.0/8`,
`240.0.0.0/4`, the IANA special-purpose IPv4 blocks (including the 6to4 relay
anycast `192.88.99.0/24`), every IPv6 address outside global unicast
`2000::/3`, the special-purpose prefixes inside it — 6to4 `2002::/16`, Teredo
`2001::/32`, IETF protocol assignments `2001:1::/32`, benchmarking
`2001:2::/48`, ORCHIDv2 `2001:20::/28` and documentation `2001:db8::/32` — and
the **IPv4-mapped (`::ffff:10.0.0.1`) and IPv4-compatible (`::10.0.0.1`)
spellings of all of them** — and refuses the delivery if *any* of them is not
public, because a name answering with one public and one private address is the
shape of a rebind and hyper would try them in order. It then builds the
delivery client with `reqwest::ClientBuilder::resolve_to_addrs` pinned to those
addresses. **That pin is what makes the check mean anything**: reqwest never
resolves the name again, so the address that was classified is the address the
socket connects to. The Step 5 plan's decision 4 concluded that a
resolve-then-connect check is TOCTOU "unless reqwest is given a custom
connector"; `resolve_to_addrs` is the third answer it did not consider, and it
needs no connector. Redirects remain `Policy::none()` — a followed `302` would
resolve the hop's host freshly and be the one way back out of the pin.

A refused target is a **permanent** delivery failure: `state = 'exhausted'` on
the first attempt, `next_attempt_at` null, `payload_sha256` null (the guard runs
before signing, so no bytes were signed), `response_excerpt` beginning
`ssrf_blocked: ` and naming the address *class* — `loopback`, `link_local`,
`private`, `cgnat`, … — and **never the address**, because the address is
exactly what the request was trying to learn and `response_excerpt` is a column
the merchant's operator reads. Exactly one `ERROR … alert = true` is emitted, at
that transition, naming the endpoint id, the delivery, the event, the merchant
and the class. The ladder is not walked: eight identical refusals over 31 hours
tell nobody anything. A host that merely fails to **resolve** is *not* a refusal
— it is an ordinary failed attempt recorded `delivery_target_unavailable: …`
that walks `delivery_delay` exactly as the transport error it replaces did,
because a resolver outage heals and a merchant must not lose an event to one.

`webhooks.allow_private_targets` (default `false`) is the only value that
changes the verdict, and it changes nothing else: the guard resolves
identically either way, and under `true` it simply does not classify what it
resolved (`ssrf.rs:420-427`). `config/application-sandbox.yml` and the
generated `demo` overlay set it `true` because `wiremock-webhook` is a service
on a compose network, and `deployment.livemode: true` together with it is a
refusal to boot (`ConfigError::PrivateWebhookTargetsInLivemode`) — a profile
selects a file, never a code path (ADR-0003).

**The cost, stated where it is paid.** A pin belongs to a `reqwest::Client`
builder, so each delivery builds its own client and no two deliveries share a
pooled connection: a receiver taking many events pays a TCP and TLS handshake
per event. Building the client itself is 4.0 µs (measured); the handshakes are
the real price. A per-host client cache would keep the pool and would be a cache
of pins — a client held past its DNS answer keeps delivering to the address that
name used to have — so the pool was the thing given up. Nothing has measured
this under load.

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

**It does not recover a delivery whose job was *dead-lettered*, and that is
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
[../runbooks/webhook-delivery-failures.md](../runbooks/webhook-delivery-failures.md)
(`a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan`). A pass that
*fails* logs `ERROR … alert = true` before rescheduling on the backoff — a
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
delivery. The full procedure, the diagnosis queries and what *not* to do are in
[../runbooks/webhook-delivery-failures.md](../runbooks/webhook-delivery-failures.md).
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
  effect; [../api/README.md](../api/README.md) says so where the route is
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
  *resolution* half the way an unresolvable host is already treated — an
  ordinary failed attempt on `delivery_delay` — and keep the permanent refusal
  for an address that classifies as private on every attempt of the ladder.
  That distinction is already made once in this code
  (`a_host_that_resolves_to_a_private_address_is_refused_and_an_unresolvable_one_retries`),
  which is why it is worth naming rather than inventing.
- **No replay endpoint and no CLI.** `scan:deliveries` recovers a *deleted or
  lost* job; it resurrects neither an `exhausted` delivery nor one whose job
  was dead-lettered (its `dedupe_key` is still held by the parked row), and
  nothing re-arms a `failed` event. All three are the manual `UPDATE`s in
  [../runbooks/webhook-delivery-failures.md](../runbooks/webhook-delivery-failures.md),
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
- The three event types this document lists that nothing writes at all —
  `payment_intent.created`, `payment_intent.processing`,
  `payment_intent.canceled` — plus the two refund types, are unchanged: events
  are written for terminal transitions only (decision 4 of
  `docs/plans/2026-09-03-step4-worker.md`).
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
  asserts is what the fan-out *created*. The delivery half is the same code
  every other event walks, and that has been observed against a WireMock
  receiver — but not for this type.

Why the delivery code is shaped the way it is — the digest invariant, the two
failure recorders, the fan-out's per-event transaction, and what the backstop
may and may not share — is
[../reference/vpay-worker.md](../reference/vpay-worker.md); the tables' own
reasoning is [../reference/vpay-db.md](../reference/vpay-db.md).

See [../status.md](../status.md).
