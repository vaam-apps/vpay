# Runbook: webhook deliveries that fail

A merchant is told about a settled payment by one row in `webhook_deliveries`
(migration `0022`) and one `deliver_webhook` job. This runbook covers the five
things that go wrong with that pair: a delivery that has **exhausted the retry
ladder**, one whose **job was dead-lettered**, an **event that was never fanned
out**, an endpoint that is **not configured or has no signing secret**, and a
**secret rotation**. It ends with the checklist a merchant's own handler has to
satisfy, because most "vpay is not sending webhooks" reports are a receiver
rejecting a delivery vpay recorded as a `4xx`.

Every SQL statement below was run against a `postgres:16-alpine` with every
migration through `0022` applied, **except the two added on 2026-09-03 by the
Step 5 remediation** — the un-park in "A dead-lettered delivery job" and the
re-arm in "An event that was never fanned out" — which are written against
`jobs` (migration `0021`) and `events.fanout_attempts` (migration `0024`) and
have **not** been executed. Each says so where it appears. (`0023` re-opens
`jobs.kind_is_known` for `scan_deliveries` and corrects an index comment;
`0024` adds `events.fanout_attempts` and a third `fanout_state`. Neither
creates a table this runbook reads.) Nothing here is a `just`
recipe, because there is none —
the repository ships no operator CLI, and inventing one in a runbook would be
worse than a `psql` prompt. In the local stack that prompt is:

```bash
docker compose exec postgres psql -U vpay -d vpay
```

**vpay never tells the merchant that a delivery failed.** There is no
`webhook.failed` event, no email and no dashboard view. The `exhausted`
transition is a log line and a row; the merchant's own fallback is
`GET /v1/events`, which they have to poll. Anything a merchant learns about a
missed webhook, a human tells them.

## Exhausted deliveries

### Alert

One `ERROR` line per exhausted delivery, always with `alert = true`
(`vpay_worker::webhooks::record_failure`):

```
alert=true job_id=… delivery_id=… event_id=evt_… endpoint_id=primary
url=https://merchant.example/hooks/vpay attempt=8
"webhook delivery exhausted the retry ladder; the merchant has not been told
about this event"
```

The `WARN` line that precedes it, once per failed attempt that still had a rung
left, is the early signal — a steady stream of it is an endpoint on its way to
this alert:

```
job_id=… delivery_id=… event_id=evt_… endpoint_id=primary attempt=3
status=500 retry_in_seconds=120
"webhook delivery attempt failed; retrying"
```

`status=0` on that line means there was no HTTP answer at all — DNS, connect,
TLS or the 10-second request deadline — because the field carries
`status.unwrap_or_default()`. The row is where the two are properly
distinguished: `status_code IS NULL` is a transport failure, and
`response_excerpt` then reads `no response: …`.

### What it means

The delivery was POSTed **eight times over about 31 hours** — the first attempt
plus the seven rungs of `vpay_worker::delivery_delay` (10 s, 30 s, 2 m, 10 m,
1 h, 6 h, 24 h; 112,360 seconds of waiting) — and every one of them failed. The
row is now `state = 'exhausted'` with `next_attempt_at = NULL`, no
`deliver_webhook` job exists for it any more, and **nothing will retry it**: the
`scan:deliveries` backstop only looks at `pending` rows, so an exhausted one is
invisible to it by design. Replaying is the manual transaction below.

**A `4xx` walks the whole ladder too.** A receiver answering `410 Gone` or `404`
is retried for 31 hours exactly as a `500` is — Stripe behaves the same way, and
the reason is that a `404` from a receiver mid-deploy is indistinguishable from
one that means "stop". If a merchant has genuinely retired an endpoint, remove
it from their `webhooks:` block; do not expect their status code to stop the
retries.

A non-2xx answer is not an error the worker classifies. Delivery never consults
`JobError::decision` — a merchant's `500` is not a `ProviderError` — so the
`deliver_webhook` job itself succeeded on every one of those attempts and none
of this appears as a dead letter or on `job loop gauge`'s `dead_lettered`.

### Find them

```sql
SELECT d.id, d.endpoint_id, d.url, d.attempt, d.state, d.status_code,
       left(coalesce(d.response_excerpt, ''), 120) AS said,
       d.sent_at, e.merchant_id, e.type, e.object_id
FROM webhook_deliveries d
JOIN events e ON e.id = d.event_id
WHERE d.state = 'exhausted'
ORDER BY d.sent_at DESC
LIMIT 50;
```

`e.object_id` is the `pi_…` the merchant cares about; `d.url` is the URL as it
was **when the delivery was created**, which is not necessarily the one in
today's YAML — that denormalisation is deliberate, so re-pointing an endpoint
does not rewrite where bytes were actually sent.

The shape of the outstanding work, in one query:

```sql
SELECT state, count(*) FROM webhook_deliveries GROUP BY state ORDER BY state;
```

`'failed'` is in the CHECK constraint's vocabulary and **nothing writes it**: a
failure that still has a rung left stays `pending`, because that is what says
another attempt is owed. Seeing a `failed` row means something outside the
worker wrote it.

### Is it the outbox or the delivery?

If the merchant is missing a webhook and there is no delivery row at all, the
fan-out is the suspect, not delivery:

```sql
SELECT count(*) AS undrained FROM events WHERE fanout_state = 'pending';
```

A number that does not fall over a few seconds means the `fanout:events`
singleton is not running — no worker is up, or the seed lost its row. A number
that falls to a *floor* and stops means the events at the head of the page are
failing individually; those count up `fanout_attempts` and land in
`fanout_state = 'failed'` after five passes, which is its own section below. It is
seeded by `vpay_worker::run_loop::seed_singletons` at every worker boot with
`ON CONFLICT DO NOTHING`, so **restarting a worker re-seeds it**. Confirm with:

```sql
SELECT kind, dedupe_key, run_at, locked_by FROM jobs
WHERE dedupe_key IN ('fanout:events', 'sweep:expired', 'scan:live', 'scan:deliveries');
```

A delivery that is `pending` with no job behind it is the other lost case:

```sql
SELECT d.id, d.state, d.next_attempt_at, j.id IS NOT NULL AS has_job
FROM webhook_deliveries d
LEFT JOIN jobs j ON j.dedupe_key = 'webhook:' || d.id::text
WHERE d.state = 'pending';
```

**`has_job = false` repairs itself within ten minutes.** The `scan:deliveries`
singleton (migration `0023`) runs every **10 minutes**, reads up to **500** rows
a pass, and re-enqueues a `deliver_webhook` job for each — the same relationship
`scan:live` has to `poll_charge`. It covers both shapes: a delivery whose
`next_attempt_at` has passed, **and** one that was never attempted
(`next_attempt_at IS NULL`) and is older than `RecoveryPolicy::lease`. The lease
on that second arm is not caution about correctness, it is what stops the scan
racing the queue: the fan-out writes the delivery row and its job in one
transaction, so a row younger than a lease is one whose job simply has not been
claimed yet.

So `has_job = false` on rows **older than a lease** that survives two passes
means the singleton itself is gone — check the singleton query above and restart
a worker, which re-seeds it. Rows younger than that are normal and need nothing.

**What the scan does not cover** is an `exhausted` delivery (that state is not
`pending`, so the scan cannot see it by design — replaying one is the
transaction below) and a delivery whose job was **dead-lettered** (the parked
row still holds the `dedupe_key`, so the scan's insert is a no-op — un-parking
one is the section after that).

## Replaying a delivery

There is **no replay endpoint and no CLI**. Re-sending is two writes in one
transaction: flip the row back to `pending`, and re-enqueue the job that drives
it. Do both — the job alone is refused by `record_*`'s `state = 'pending'`
guard, and the row alone would sit until the next `scan:deliveries` pass, up to
ten minutes away, rather than going out now.

Fix whatever the receiver was doing first. Then, for one delivery id:

```sql
BEGIN;

UPDATE webhook_deliveries
SET state           = 'pending',
    attempt         = 0,
    next_attempt_at = now()
WHERE id    = '11111111-1111-1111-1111-111111111111'
  AND state = 'exhausted';

INSERT INTO jobs (kind, dedupe_key, payload, run_at)
VALUES ('deliver_webhook',
        'webhook:' || '11111111-1111-1111-1111-111111111111',
        jsonb_build_object('delivery_id', '11111111-1111-1111-1111-111111111111'),
        now())
ON CONFLICT (dedupe_key) DO NOTHING;

COMMIT;
```

Both statements were run against a real database with every migration through `0022` applied;
the row comes back `attempt = 0, state = 'pending'` with a `next_attempt_at` in
the past, and the job comes back claimable with
`payload = {"delivery_id": "…"}` — the exact shape
`vpay_worker::jobs::DeliverWebhookPayload` deserialises.

Three details, each of which is the reason the statement is written this way:

- **`AND state = 'exhausted'` is the safety.** It refuses to touch a delivery
  that is already `pending` (one a worker may be attempting right now) or
  `succeeded` (re-sending which would be a duplicate the merchant did not ask
  for). Re-running the whole transaction is a no-op: `UPDATE 0`, `INSERT 0`.
- **`attempt = 0` restores the whole ladder**, so a receiver that is still
  broken fails fast on the 10-second rung rather than a day from now. To grant
  exactly *one* more attempt instead, leave `attempt` alone: `delivery_delay(8)`
  is `None`, so the next failure exhausts the row again immediately.
- **`payload_sha256` is deliberately not cleared.** It is the digest of the
  bytes the first signed attempt signed, and the handler compares its freshly rendered body
  against it before sending. If the replay dead-letters with *"re-rendered event
  … to a different body than the one the first signed attempt signed"*, that is the check
  working: the event renderer changed between the original attempt and now, and
  the delivery must not go out under a signature nobody can reproduce. Clearing
  the column would silence the one check that catches it.

`dedupe_key` is unique across `jobs`, so `ON CONFLICT DO NOTHING` is what makes
the transaction safe to run twice; the original job was `DELETE`d when it
finished, which is why the key is free.

To replay every exhausted delivery for one merchant, do the same thing set-wise
rather than by hand — but read the count first, because each row is a POST a
merchant's handler will see:

```sql
BEGIN;
UPDATE webhook_deliveries d
SET state = 'pending', attempt = 0, next_attempt_at = now()
FROM events e
WHERE e.id = d.event_id AND e.merchant_id = 'merchant_a' AND d.state = 'exhausted';

INSERT INTO jobs (kind, dedupe_key, payload, run_at)
SELECT 'deliver_webhook', 'webhook:' || d.id::text,
       jsonb_build_object('delivery_id', d.id::text), now()
FROM webhook_deliveries d
JOIN events e ON e.id = d.event_id
WHERE e.merchant_id = 'merchant_a' AND d.state = 'pending' AND d.attempt = 0
ON CONFLICT (dedupe_key) DO NOTHING;
COMMIT;
```

**Only the single-delivery form above has been executed against a real
database.** The set-wise form is the same two statements with a join and has
not been run; treat it as a sketch to adapt, not a copy-paste.

### Do not

- **Do not `DELETE` the row.** It is the only record that a merchant was owed
  an event and did not get it, and `webhook_deliveries_event_endpoint` means
  the fan-out will not recreate it — the event is already `fanout_state =
  'done'`.
- **Do not `UPDATE events SET fanout_state = 'pending'` to "re-fan-out".** The
  drain's insert is `ON CONFLICT (event_id, endpoint_id) DO NOTHING`, so the
  delivery row is not recreated and no job is enqueued; the event just flips
  back to `done` having done nothing. Replay the delivery, not the event.
- **Do not clear `payload_sha256`** — see above.

## A dead-lettered delivery job

### Alert

Two lines, from two different places. The one that fires when it happens is
`run_loop`'s disposition line (`docs/runbooks/worker-queue.md` covers it in
general):

```
alert=true job_id=… kind=deliver_webhook disposition=DeadLettered
"job failed: job … cannot be run: …"
```

The one that keeps saying so, once per backstop pass, is:

```
job_id=… parked=1 deliveries=["webhook:1111…"]
"webhook deliveries are pending with a dead-lettered (parked) delivery job;
this backstop cannot recover them, and it will not try"
```

### What it means

A `deliver_webhook` job was **parked**, not deleted: `run_at = 'infinity'`,
lease cleared, reason in `last_error` (`vpay_db::Jobs::dead_letter`, and that
module's own comment explains why parking rather than deleting). The delivery
row is still `pending` and **nothing will ever attempt it**. The
`scan:deliveries` backstop cannot help: parking keeps the `dedupe_key`, so its
`INSERT … ON CONFLICT (dedupe_key) DO NOTHING` does nothing for exactly these
rows. That is deliberate — the reasons a delivery job is parked
(`JobError::Poisoned`: an event that will not render, a body whose digest no
longer matches what the first signed attempt signed) are not fixed by retrying, and a scan
that un-parked it would re-run the same failure every ten minutes forever
(`a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan`).

### Find them

```sql
SELECT j.dedupe_key, j.attempts, left(j.last_error, 200) AS why,
       d.id AS delivery_id, d.state, d.attempt, e.merchant_id, e.type
FROM jobs j
JOIN webhook_deliveries d ON j.dedupe_key = 'webhook:' || d.id::text
JOIN events e ON e.id = d.event_id
WHERE j.run_at = 'infinity'::timestamptz
ORDER BY j.created_at;
```

`last_error` is the whole diagnosis: it is the `JobError::Poisoned` reason,
truncated to 2000 characters.

### Fix

**Fix the cause first.** A parked delivery job is parked because something
about the event or the renderer is broken; un-parking without fixing it walks
straight back into the same failure and parks it again, and the second park
overwrites `last_error`.

Then un-park it. The columns are migration `0021`'s:

```sql
UPDATE jobs
SET run_at   = now(),
    attempts = 0
WHERE dedupe_key = 'webhook:11111111-1111-1111-1111-111111111111'
  AND run_at = 'infinity'::timestamptz;
```

- **`attempts = 0` restores the poll ladder**, exactly as `attempt = 0` does
  for a delivery replay: the job is claimed at rung zero rather than wherever
  its failures left it. `dead_letter` already cleared `locked_at`/`locked_by`,
  so there is no lease to release.
- **`AND run_at = 'infinity'` is the safety.** It refuses to touch a job that
  is already runnable — one a worker may be executing right now — and makes
  re-running the statement a no-op (`UPDATE 0`).
- **Do not `DELETE` the parked row instead.** The `dedupe_key` would then be
  free, the next `scan:deliveries` pass would re-create the job from scratch,
  and the failure would repeat every ten minutes with a fresh `attempts = 1` —
  which reads as a flapping receiver rather than as a permanently broken row.
  That is the exact hot loop parking exists to prevent.

If the delivery should *not* go out at all, leave the job parked and record
why; the `pending` delivery row and the parked job together are the durable
statement that a merchant was owed an event and did not get it.

**Not executed.** Unlike the replay transaction above, this `UPDATE` has not
been run against a database — it is written from migration `0021`'s columns
and `vpay_db::Jobs::dead_letter`'s own write. Read it before running it.

## An event that was never fanned out

### Alert

One `ERROR` with `alert = true`, once in the event's life:

```
alert=true job_id=… event_id=evt_… merchant_id=merchant_a
event_type=payment_intent.succeeded attempts=5 error=…
"an event failed fan-out for the last time and has been abandoned
(fanout_state = 'failed'); the merchant will not receive this event and
nothing will retry it"
```

Preceded by four `WARN`s carrying no `alert` — one per failed pass, with the
same `event_id` and a rising `attempts`. That WARN stream is the early signal;
the single `ERROR` is the abandonment.

### What it means

The drain failed on this one event `vpay_worker::FANOUT_MAX_ATTEMPTS` (**5**)
times. Each failure rolled its own transaction back whole, so there are no
half-written delivery rows; the fifth set `fanout_state = 'failed'` (migration
`0024`). A `failed` event is **not** `pending`, which is the point: it has left
`events_pending_idx` and `events::pending_page`, so it no longer heads every
page and no longer alerts. Nothing retries it.

The alternative — leaving it `pending` forever — was worse in two ways an
operator feels immediately: the alert re-fired every five seconds, and the row
held one of the drain's hundred page slots, so a hundred such events stopped
webhooks for **every** merchant.

The merchant is not told. `GET /v1/events` still serves the event, which is
their only fallback.

### Find them

```sql
SELECT id, merchant_id, type, object_id, fanout_attempts, created_at
FROM events
WHERE fanout_state = 'failed'
ORDER BY seq;
```

And the health of the backlog as a whole:

```sql
SELECT fanout_state, count(*) FROM events GROUP BY fanout_state ORDER BY 1;
```

A `pending` count that does not fall over a few seconds is the fan-out not
running (above). A `failed` count above zero is this section.

### Fix

**Fix the cause first**, then re-arm. The cause is in the `WARN`/`ERROR`
lines' `error=` field — historically a configuration value the database
refused (an over-long `endpoint_id`, since refused at boot).

```sql
UPDATE events
SET fanout_state    = 'pending',
    fanout_attempts = 0
WHERE id = 'evt_11111111111111111111111111'
  AND fanout_state = 'failed';
```

The next `fan_out_events` pass picks it up within five seconds. Note what this
does *not* need: no job to enqueue, unlike a delivery replay — the drain is a
singleton that reads the backlog, so putting the event back in the backlog is
the whole of it.

- **`AND fanout_state = 'failed'` is the safety**, and makes re-running the
  statement a no-op. In particular it refuses to touch a `done` event: flipping
  one of those back to `pending` does nothing useful (the delivery rows already
  exist and `create_in_tx` is `ON CONFLICT DO NOTHING`, so the pass just marks
  it `done` again) — see "Do not" above.
- **`fanout_attempts = 0` gives it the full five passes again.** Leaving the
  count at 5 means the very next failure re-abandons it immediately, which is
  what you want if you are testing whether the cause is really fixed and do not
  want five more alerts' worth of noise.

**Not executed.** This `UPDATE` is written from migration `0024`'s columns and
has not been run against a database. The state it repairs *is* produced against
a real Postgres by
`a_permanently_unfannable_event_is_abandoned_after_five_passes_and_alerts_once`.

## An endpoint with no signing secret

### Alert

```
job_id=… delivery_id=… event_id=evt_… endpoint_id=primary merchant_id=merchant_a
"webhook endpoint is not configured, or has no signing secret; the delivery
cannot be signed and will retry"
```

### What it means

The delivery row names an `endpoint_id` that the worker's `EndpointRegistry`
does not hold for that `merchant_id`, or holds with an empty `secrets` list.
The endpoint was removed from — or renamed in — `merchant_clients[].webhooks[]`
after the delivery row was created. `webhook_deliveries.endpoint_id` references
no table (endpoints are YAML, ADR-0003), so this is a real state and not a
broken join.

vpay records it as an ordinary failed attempt and retries, rather than
exhausting on the spot: a rollout that briefly serves an older configuration
heals by itself, and a removal that is permanent exhausts through the ladder
anyway. **It never sends the event unsigned** — a receiver may not act on an
unsigned webhook.

**`payload_sha256` stays `NULL` on this branch**, because nothing was rendered
or signed and there is therefore no "the bytes we signed" for it to be the
digest of
(`a_delivery_with_no_configured_endpoint_records_a_failure_and_no_digest`). That
matters when the endpoint comes back: the first attempt that actually **renders
and signs a body** is the one that stamps the column — including one whose
socket never opened, since those bytes were signed either way — so the mismatch
check on later attempts is against a body vpay really produced. A row with
`attempt > 0` and a `NULL` digest is the signature of this failure mode, and a
useful thing to `SELECT` for.

### Fix

Put the endpoint back under the same `id`, with its secret, and restart the
worker (the registry is built once at boot from configuration — there is no
reload). If the endpoint is genuinely gone, let the ladder exhaust and leave the
rows as the record.

Note that `merchant_id` and `client_id` are different strings and the registry
is keyed on `merchant_id`: an endpoint configured under the wrong merchant is
invisible to the fan-out and produces no delivery row at all, not this warning.

## Rotating `MERCHANT_WEBHOOK_SECRET`

An endpoint may declare **one or two** secrets (`ConfigError::WebhookSecretCount`
refuses zero and three or more). Each produces one `v1=` in `Vpay-Signature`,
and both SDK verifiers accept a header if *any* `v1=` matches, so a rotation has
no window in which deliveries fail. The order is:

1. Add the new secret **beside** the old one, keeping the old one first:

   ```yaml
   merchant_clients:
     - client_id: demo-merchant
       webhooks:
         - id: primary
           url: https://merchant.example/hooks/vpay
           secrets: ["${MERCHANT_WEBHOOK_SECRET}", "${MERCHANT_WEBHOOK_SECRET_NEXT}"]
   ```

   Set `MERCHANT_WEBHOOK_SECRET_NEXT` on **both** `vpay-server` and
   `vpay-worker` before deploying: the server loads and validates the same
   document, and an unresolved `${VAR}` is a refusal to boot (exit 78), not a
   warning. Restart both.

2. Confirm deliveries now carry two signatures. The receiver sees
   `t=…,v1=<old>,v1=<new>`; from the database, `state = 'succeeded'` on new
   rows is the operational signal.

3. Move the merchant's handler to the new secret. Nothing on vpay's side
   changes; their verification succeeds under either.

4. Remove the old secret, leaving one entry, and restart both processes again.

**Three boot rules will refuse a badly-made rotation, all at step 1 rather than
at step 4.** A literal secret in a livemode file is
`ConfigError::LiteralSecret`, checked against the file's *text* before
placeholders are resolved, so pasting the value inline fails. A livemode secret
shorter than **32 bytes once resolved** is `ConfigError::WeakWebhookSecret` — an
HMAC-SHA256 key below the hash's own 32-byte output adds nothing over one at it
and makes offline guessing cheap, and whoever guesses it can sign a
`payment_intent.succeeded` the merchant's handler will believe. The two rules
are a pair: the first says the secret came from the environment, the second says
the environment holds something worth having. In sandbox neither applies and the
rule is only "not blank" — so a rotation rehearsed in sandbox with a short value
will be refused in livemode. And a third entry in `secrets:` is refused
(`ConfigError::WebhookSecretCount`, `1..=2`), because an endpoint that never
finished a rotation is a secret nobody revoked.

**What livemode URL validation does not do:** it checks the scheme is `https`
and that the **host** carries none of four stub substrings (`wiremock`, `stub`,
`mock`, `localhost`), plus the shape rules that hold in both deployments (the
URL parses, names a host, has no embedded credentials, ≤ 2048 characters). The
host and not the whole URL, so `https://hooks.example/mockups` is a legitimate
livemode endpoint. It **never looks at the destination address**, so a rotation
is not the moment anyone is stopped from pointing an endpoint at
`https://169.254.169.254/…`.
See [../flows/webhooks.md](../flows/webhooks.md).

**There is no way to rotate without a deploy.** Endpoints and secrets are
configuration (ADR-0003, ADR-0008); the dashboard cannot administer them and
there is no `/v1/webhook_endpoints`.

## Receiver-side checklist

Before treating a `4xx`/`5xx` in `response_excerpt` as a vpay defect, walk this
with the merchant. Each item is something a receiver gets wrong that looks
identical from vpay's side.

- **Verify the raw bytes.** The signature covers `"{t}.{body}"` over the body
  exactly as received. A framework that parses JSON and re-serialises it before
  verification breaks every delivery; take the raw body first.
- **Use the SDK's verifier.** `vpay_sdk::webhooks::verify` (Rust) or
  `verifyWebhook` from `@vaam-apps/vpay-sdk` (Node). Both are exercised against bytes this
  server emitted, in `backends/tests/integration/tests/webhooks.rs`.
- **A Stripe-shaped handler works unmodified.** `Stripe-Signature` carries the
  same value as `Vpay-Signature`, byte for byte, so
  `stripe.webhooks.constructEvent` verifies it with the vpay secret.
- **Try every `v1=`.** During a rotation there are two. A handler that reads
  only the first fails for the whole overlap.
- **Check the clock.** Both verifiers reject a `t` more than 5 minutes from the
  receiver's own clock. A receiver whose clock has drifted rejects perfectly
  good deliveries, and vpay records that as an ordinary `4xx`.
- **Acknowledge first, work later.** The request deadline is **10 seconds**
  end to end (5 to connect). A handler that does its work before answering
  turns a slow database into a failed delivery. Answer `2xx`, then process.
- **Any `2xx` is success**; anything else, including a `3xx`, is a failed
  attempt. The client refuses redirects on purpose — following one would replay
  a signed event body at a host the operator never configured.
- **Dedupe on `event.id`, and do not assume order.** Delivery is at-least-once
  and **unordered**: concurrent claim tasks and the retry ladder can deliver two
  of one merchant's events out of the order they happened, so a handler that
  decides state from arrival order will settle a payment from a stale event.
  Reason from `event.created` and the object's own `status`.
- **`Vpay-Event-Id` is a convenience, not evidence.** It is not covered by the
  signature. Read the id out of the verified body.

## Status

**Written against code that exists; most of the SQL was run, the procedure was
not.** Every statement in this document was executed against a
`postgres:16-alpine` with every migration through `0022` applied, including the
replay transaction (which was run twice, to confirm the second run is a no-op)
— **except three, each marked as unrun where it appears**: the set-wise replay,
the un-park in "A dead-lettered delivery job", and the re-arm in "An event that
was never fanned out". The `scan:deliveries` row of the singleton query names a
job kind migration `0023` adds. The states this runbook looks for are
produced by the worker's own integration suite
(`backends/tests/integration/tests/webhooks.rs` —
`a_delivery_past_the_last_rung_is_exhausted_and_not_rescheduled` makes an
exhausted row, `the_ladder_walks_delivery_delay_and_then_succeeds` makes failing
ones, `a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan` makes a
parked delivery job, and
`a_permanently_unfannable_event_is_abandoned_after_five_passes_and_alerts_once`
makes a `failed` event).

**No part of this runbook has been followed against a running deployment**, no
real merchant endpoint has ever been POSTed to, and a replayed delivery has
never been observed reaching a receiver — the replay transaction is proven to
run and to leave the right rows, not to result in a delivery. There is no
dashboard view for any of it: `psql` is the whole toolkit today. See
[../status.md](../status.md) and [../flows/webhooks.md](../flows/webhooks.md).
