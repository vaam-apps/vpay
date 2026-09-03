<!-- Implementation design for one step of the production-readiness plan. A point-in-time working document: once the step lands, docs/status.md and the flow docs are the record and this file is history. -->

# Step 5 — webhooks: implementation-ready design

Decisions taken by the orchestrator under the user's delegation (do not reopen): (1) the outbox
drain is hand-written (the `cratestack-outbox` candidate in docs/roadmap.md is declined for now —
Step 4 already hand-writes `events::insert_in_tx`, and the drain shares the `jobs` table, lease
reaping and drain path; reversible, recorded in the roadmap); (2) endpoints carry an operator-authored
`id`, unique per merchant, refused at boot; (3) the signed body is re-rendered per attempt and only its
SHA-256 stored — a mismatch is `Poisoned`; (4) no runtime SSRF filtering — boot-time `validate_host`
only, stated plainly in docs (a custom connector is the only honest alternative and is out of scope);
(5) build `GET /v1/events` and `GET /v1/events/{id}` now, defer `?type=` and say so; (6) the Node
parity test shells out to `node`, gated by `VPAY_REQUIRE_NODE=1` in CI so a missing `node` fails
rather than skips; endpoints are YAML (`merchant_clients[].webhooks[]`), secrets `${VAR}` and covered
by the livemode literal-secret rule (S3 is required work).

I have what I need. Here is the design.

---

# Step 5 — webhooks: implementation-ready design

## 0. Five things that are not what the ticket implies

**S1 — Phase 6 has an undecided dependency question reserved for the maintainer.** `docs/roadmap.md:742-746`: *"Candidate, not decided (2026-09-02): `cratestack-outbox` implements exactly this shape — `OutboxClient::persist_in_tx` inside the caller's transaction, `drain` in insertion order… Whether to take the dependency or write the ~200-line outbox by hand is for whoever builds this phase."* Step 4 has already written the `persist_in_tx` half by hand (`vpay_db::events::insert_in_tx`). Writing the drain half by hand silently closes this. **Do not start until Q1 is answered.**

**S2 — nothing renders an `Event` envelope, and `events.data` is not one.** `events.data` holds the *inner* wire object: `backends/crates/vpay-db/src/events.rs:78-80`, and `settlement.rs:132-134` ("`event_data` is the wire object as it was at settlement time (`vpay-api`'s shape — this crate does not know it)"). The SDK envelope (`sdks/rust/src/model.rs:215-235`) needs `id`, `object:"event"`, `type`, `created`, `livemode`, `data.object`. `vpay_api::model` has `PaymentIntentObject`, `ListObject`, tags — **no `EventObject`** (grep of `backends/crates/vpay-api/src/model.rs`). It must be written, and `GET /v1/events` and the delivered body must be the *same* renderer or the two surfaces disagree about what an event is.

**S3 — the livemode literal-secret guard will not see a webhook secret.** `RawProviderSecrets` "Walks the merged, **unresolved** document for `providers[].credentials`" (`backends/crates/vpay-config/src/config.rs:585-595`), and `validate_all` (`:497-505`) only iterates `provider.credentials`. A livemode config with `secrets: [whsec_literal]` boots clean today. Extending the walk to `merchant_clients[].webhooks[].secrets` is required work in block C, not a nicety.

**S4 — `vpay-worker` cannot make an HTTP request or an HMAC.** `backends/crates/vpay-worker/Cargo.toml` has no `reqwest`, `hmac`, `sha2` or `hex`; `hex` is not a workspace dependency at all — it exists only at `sdks/rust/Cargo.toml:56`. Block B promotes `hex = "0.4"` into `[workspace.dependencies]` (`Cargo.toml`, beside `sha2`/`hmac`/`subtle` at `:214-216`) and adds `reqwest.workspace = true` to the worker. `subtle` is *not* needed — signing has nothing to compare.

**S5 — the fan-out handler has no way to see endpoints.** Step 4's handler signature takes `&ResourceConfig` (`docs/plans/2026-09-03-step4-worker.md:76`), and `ResourceConfig` (`backends/crates/vpay-api/src/v1/mod.rs:297-302`) keys merchants by **`client_id`**, not `merchant_id`, and carries no endpoints. `events.merchant_id` is the fan-out key. Adding `endpoints_by_merchant_id` to `ResourceConfig` keeps every Step 4 signature intact — but that struct also lives in the server's `AppState`, so its `Debug` must redact secrets (verify it is not `#[derive(Debug)]` before shipping).

Documented and *supporting* the YAML default: `docs/flows/configuration.md:277` lists "webhook endpoints" among values *safe to mutate* in config; ADR-0008 puts anything the dashboard cannot administer into YAML. **No document anywhere proposes a `/v1/webhook_endpoints` resource** — searched `docs/`, `docs/api/README.md:174-180`, `sdks/`. The orchestrator's default stands.

## 1. Schema — `0022_create-webhook-deliveries.sql`

```sql
ALTER TABLE jobs DROP CONSTRAINT kind_is_known;
ALTER TABLE jobs ADD CONSTRAINT kind_is_known CHECK (kind IN
  ('poll_charge','resubmit_charge','sweep_expired','scan_live_charges','fan_out_events','deliver_webhook'));

CREATE TABLE webhook_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id TEXT NOT NULL REFERENCES events (id),
    -- The operator-authored `merchant_clients[].webhooks[].id` from YAML.
    -- Not a URL hash: an operator correcting a typo'd URL must not orphan
    -- the delivery history, and a hash is unreadable in a runbook.
    endpoint_id TEXT NOT NULL,
    -- Denormalised for forensics; the endpoint may be re-pointed later.
    url TEXT NOT NULL,
    attempt INT NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'pending',
    status_code INT,
    response_excerpt TEXT,
    -- Proof the bytes did not change between attempts (§3).
    payload_sha256 TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ, responded_at TIMESTAMPTZ,
    next_attempt_at TIMESTAMPTZ,
    CONSTRAINT state_is_known CHECK (state IN ('pending','succeeded','failed','exhausted')),
    CONSTRAINT endpoint_id_length CHECK (char_length(endpoint_id) BETWEEN 1 AND 64),
    CONSTRAINT url_length CHECK (char_length(url) BETWEEN 1 AND 2048),
    CONSTRAINT excerpt_length CHECK (response_excerpt IS NULL OR char_length(response_excerpt) <= 2000),
    CONSTRAINT attempt_is_not_negative CHECK (attempt >= 0)
);
CREATE UNIQUE INDEX webhook_deliveries_event_endpoint ON webhook_deliveries (event_id, endpoint_id);
CREATE INDEX webhook_deliveries_live_idx ON webhook_deliveries (next_attempt_at)
    WHERE state = 'pending';
COMMENT ON TABLE webhook_deliveries IS '…';
```

**No `webhook_endpoints` table.** Endpoints are YAML (ADR-0003); `endpoint_id` is a stable operator-authored string, refused as a duplicate within one merchant at boot. Deliberately **no** `response_is_paired`-style CHECK pairing `status_code`/`responded_at` with `sent_at`: a transport failure has a `sent_at` and neither of the others, which is a real and distinct state.

## 2. Fan-out — the `fan_out_events` job

`dedupe_key = "fanout:events"`, seeded at worker boot `ON CONFLICT DO NOTHING`, rescheduled every 5 s (or immediately when the page came back full).

Per iteration: `events::pending_page(pool, 100)` (exists, `events.rs:160`) → for each row in `seq` order, resolve endpoints for `row.merchant_id` from config → **one transaction per event**: `INSERT … webhook_deliveries ON CONFLICT (event_id, endpoint_id) DO NOTHING` per endpoint, `jobs::enqueue_in_tx('deliver_webhook', "webhook:{delivery_id}", {delivery_id}, now())` per created delivery, `UPDATE events SET fanout_state='done' WHERE id=$1 AND fanout_state='pending'`, commit.

Crash-idempotent by construction: the `fanout_state` flip and the inserts share the transaction, so a crash re-runs the whole event; the unique index and `jobs_dedupe_key` absorb the replay. A merchant with **zero** endpoints still flips to `done` with zero deliveries — otherwise the backlog index grows without bound (partial index `events_pending_idx`, `0018`).

## 3. Signing — the exact bytes

Build the envelope **once per attempt**, sign that exact buffer, send that exact buffer:

```rust
// vpay-api/src/model.rs — the one renderer, shared with GET /v1/events
pub struct EventObject { pub id: String, pub object: EventTag, #[serde(rename="type")] pub kind: String,
                         pub created: i64, pub livemode: bool, pub data: EventDataObject }
impl TryFrom<&vpay_db::EventRow> for EventObject { type Error = ApiError; … }
```

`created = row.created_at.unix_timestamp()`; `data.object = row.data` verbatim.

```rust
// vpay-worker/src/signing.rs
pub fn signature_header(body: &[u8], now: OffsetDateTime, secrets: &[String]) -> String;
```

`t = now.unix_timestamp()` rendered as plain decimal (no sign, no padding — matches `^\d+$`, `sdks/nodejs/src/webhooks.ts:35`). For each secret in order: `HMAC-SHA256(secret_bytes, t_text || b"." || body)`, `hex::encode` (lowercase). Result `format!("t={t},v1={s0},v1={s1}")`. `t` is signed as the **literal text written into the header** — pinned by `sdks/rust/src/webhooks.rs:the_hmac_covers_the_literal_t_text_not_a_re_rendered_number`.

The body is **not stored**. `payload_sha256` is written on the first attempt and compared on every later one; a mismatch is `JobError::Poisoned` (a renderer changed under a live delivery, which is exactly the defect a merchant would see as a bad signature). This is cheaper than a stored body column and makes the invariant observable.

Secrets: `merchant_clients[].webhooks[].secrets: [${VAR}, …]`, 1–2 entries. `WebhookEndpoint` gets a hand-written `Debug` redacting `secrets` wholesale, mirroring `MerchantClient`'s at `backends/crates/vpay-config/src/oauth.rs:216-236`.

## 4. Delivery — the `deliver_webhook` job

Client: `vpay_provider::http::client_with_timeouts(Duration::from_secs(5), Duration::from_secs(10))` (`http.rs:200`) — vendored roots, `redirect::Policy::none()`, `no_proxy()` already (`:278-279`). One client built at worker boot and cloned.

`POST url`, headers `Content-Type: application/json`, `Vpay-Signature`, `Vpay-Event-Id: evt_…` (a convenience for merchant dedupe logging; not part of the signature). Body = the signed bytes.

Response read with `vpay_provider::http::bounded_body(resp, 8 * 1024)` — an 8 KiB local cap, not `MAX_RAIL_BODY_BYTES`; a receiver's ack body is not a rail's. `response_excerpt` = first 512 chars of the lossy UTF-8, truncated on a char boundary.

- `2xx` → `state='succeeded'`, `status_code`, `responded_at`, `Outcome::Done` (job deleted).
- anything else, or a transport error → record `status_code`/`excerpt`, `attempt += 1`; `delivery_delay(attempt)` → `Some(d)` = `state` stays `pending`, `next_attempt_at = now + d`, `Outcome::RescheduleAfter(d)`; `None` = `state='exhausted'`, log at `tracing_level(Severity::Error)` with `alert = true`, `Outcome::Done`.

**URL validation is boot-time, not runtime.** Reuse `vpay_config::config::validate_host` on each endpoint URL inside `validate_all`, giving `ConfigError::InsecureHost` (https-only under livemode) and `ConfigError::StubHostInLivemode` for free, in the shape operators already recognise. **No runtime private/link-local IP blocking** — see Q4.

## 5. Retry ladder

```rust
// vpay-worker/src/lib.rs, beside poll_delay
#[must_use]
pub fn delivery_delay(attempt: u32) -> Option<Duration> {
    const LADDER: [u64; 7] = [10, 30, 120, 600, 3_600, 21_600, 86_400];
    LADDER.get(attempt as usize).copied().map(Duration::from_secs)
}
```

One test transcribing `docs/flows/webhooks.md:35` rung by rung, asserting `delivery_delay(7) == None` and monotonicity. `Option` rather than `Duration`, because "the ladder ran out" is the `exhausted` transition and must not be expressible as another rung.

Add to `backends/crates/vpay-worker/src/error.rs`'s module header, and to `docs/flows/reconciler.md`'s Status: **`JobError::decision()` is charge polling only**; webhook delivery uses `delivery_delay` and never consults `Classify::retry`. Delivery has no rail semantics — a merchant's 500 is not a `ProviderError`.

## 6. Merchant surface — build `GET /v1/events` now

Two `V1Route` entries (`backends/crates/vpay-api/src/v1/mod.rs:126-149`): `/events` (`GET`) and `/events/{id}` (`GET`), read-only, `MerchantScope`-filtered, `ListObject<EventObject>` with the existing `limit`/`starting_after`/`ending_before` cursor machinery (`payment_intents.rs:332-352`, `parse_limit` at `:1333`). New `vpay_db::events::list_page(pool, merchant_id, &ListPage) -> Result<(Vec<EventRow>, bool), DbError>` and `get_by_id(pool, merchant_id, id)` — `events_merchant_seq_idx` (`0018`) already exists for exactly this. `?type=` filter: **defer** (documented in `docs/api/README.md:177` but adding a filter is a second cursor interaction; say so in status.md rather than half-build it). No endpoint CRUD.

## 7. Tests

All under `backends/tests/integration/tests/`, real Postgres + a WireMock container from `vpay_testkit::containers::start_wiremock` (`containers.rs:192`) pointed at a **new** root `backends/tests/webhook-receiver/wiremock/` (a single `any-post-200.json` mapping, plus scenario mappings). Requests are read back from `GET /__admin/requests`.

- **Signature parity, Rust:** deliver one event; pull the journal entry; feed `body` + `Vpay-Signature` to `vpay_sdk::webhooks::verify_at` with the configured secret. Must return `Ok(Event)` with the right `id`. Decisive negative: flip one byte of the recorded body and assert `SignatureMismatch`.
- **Signature parity, Node:** the journal entry's `body`/header are written to a temp file and verified by `node -e` against the built `sdks/nodejs` verifier, asserting exit 0. **This is the honest option** — reusing the Node SDK's own fixture bytes would test the fixture, not the server. Gate it on `node` being on `PATH` and **fail, not skip**, if it is missing when `VPAY_REQUIRE_NODE=1` (CI sets it). Cypress-style skipping is how this suite would go green without proving parity.
- **Rotation:** two secrets configured → exactly two `v1=` values in the header, and `verify_at` succeeds with *each* secret independently.
- **Ladder:** a WireMock scenario returning `500` three times then `200`. Assert `attempt` and successive `next_attempt_at` deltas equal `delivery_delay(0..2)`, and that the delivery ends `succeeded`. **Exhausted:** drive the handler directly with attempt pre-set to 6 and assert `state='exhausted'` with no reschedule.
- **Fan-out idempotency:** run `fan_out_events` twice over one event; assert exactly one delivery row and one job. Assert a `fanout_state='done'` event never produces a delivery.
- **End-to-end:** the Step 4 `worker_e2e` confirm→settle path, extended — assert the WireMock receiver's journal holds one POST whose `Vpay-Event-Id` matches the `events` row.

## 8. Work split

**A — schema + deliveries repo + events read API (db half).** `backends/migrations/0022_create-webhook-deliveries.sql`; `backends/crates/vpay-db/src/webhook_deliveries.rs`; `events::{list_page,get_by_id}`.

```rust
pub struct DeliveryRow { pub id: Uuid, pub event_id: String, pub endpoint_id: String, pub url: String,
    pub attempt: i32, pub state: String, pub status_code: Option<i32>,
    pub response_excerpt: Option<String>, pub payload_sha256: Option<String>,
    pub next_attempt_at: Option<OffsetDateTime> }
pub async fn create_in_tx(tx: &mut PgConnection, event_id: &str, endpoint_id: &str, url: &str) -> Result<Option<Uuid>, DbError>;
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<DeliveryRow>, DbError>;
pub async fn record_success(pool: &PgPool, id: Uuid, status: i32, excerpt: Option<&str>, sha: &str) -> Result<bool, DbError>;
pub async fn record_attempt(pool: &PgPool, id: Uuid, status: Option<i32>, excerpt: Option<&str>, sha: &str, next_attempt_at: Option<OffsetDateTime>, exhausted: bool) -> Result<bool, DbError>;
pub async fn mark_fanned_out_in_tx(tx: &mut PgConnection, event_id: &str) -> Result<bool, DbError>;
// events.rs
pub async fn list_page(pool: &PgPool, merchant_id: &str, page: &ListPage) -> Result<(Vec<EventRow>, bool), DbError>;
pub async fn get_by_id(pool: &PgPool, merchant_id: &str, id: &str) -> Result<Option<EventRow>, DbError>;
```

**B — signing + ladder + handlers.** `vpay-worker/src/{signing.rs, webhooks.rs}`; `delivery_delay` in `lib.rs`; `JobKind::{FanOutEvents, DeliverWebhook}` in `jobs.rs`. No `sqlx`.

```rust
pub fn signature_header(body: &[u8], now: OffsetDateTime, secrets: &[String]) -> String;
pub fn delivery_delay(attempt: u32) -> Option<Duration>;
pub async fn handle_fan_out(pool: &PgPool, endpoints: &EndpointRegistry, job: JobRow) -> Result<Outcome, JobError>;
pub async fn handle_deliver(pool: &PgPool, http: &reqwest::Client, endpoints: &EndpointRegistry, job: JobRow) -> Result<Outcome, JobError>;
```

**C — config, API routes, tests, demo.** `vpay_config::oauth::WebhookEndpoint` + `MerchantClient::webhooks`; the `validate_all` extension **and** the `RawProviderSecrets` extension (S3); `EventObject` in `vpay-api/src/model.rs`; the two `V1_ROUTES` entries and `v1/events.rs`; `ResourceConfig::endpoints_by_merchant_id`; all of §7; demo step 6.

**Demo receiver: a third WireMock service** in `compose.demo.yml` (`wiremock-webhook`, mounting `backends/tests/webhook-receiver/wiremock`), with `examples/merchant-demo` step 6 polling that container's `/__admin/requests` for a POST naming its intent. `examples/webhook-receiver/index.mjs` is a *documentation* example and is not a compose service; ADR-0006 does not forbid it, but wiring it in would make the demo depend on a hand-rolled verifier the SDK already supersedes.

## 9. Docs and status

`docs/status.md`: row 490 (`Webhooks (signing, outbox, delivery)`) ⛔ → 🟡/✅ with the exact WireMock evidence; row 460 (`JobError`) — drop "nothing calls `decision()`" *only* if Step 4 already did, and add that delivery does **not** use it; row 475 (Poll ladder) — name `delivery_delay` as a separate ladder; row 480 — migration `0022`; the `/v1/events` ⛔ claims at `docs/api/README.md:177` and `v1/mod.rs:124-126` ("Deliberately **not** including `/v1/balance` or `/v1/events`") must both be corrected or they become lies the moment the route mounts. `docs/flows/webhooks.md` Status; `docs/flows/reconciler.md` Status (the `decision()` note); `docs/roadmap.md` Phase 6 Status and the `cratestack-outbox` paragraph. New runbook `docs/runbooks/webhook-delivery-failures.md` (reading `webhook_deliveries`, replaying an exhausted delivery, rotating a secret).

Step 4's `events` claims to re-verify before trusting: that `apply_succeeded`/`apply_failed` actually write `fanout_state='pending'` rows in the settlement transaction, and that `events::pending_page` returns `seq`-ordered rows — both are Step 4 code that has not landed on master.

---

# Decisions needed from a human

1. **`cratestack-outbox`, or hand-write the drain?** *Default: hand-write it — Step 4 already hand-wrote `insert_in_tx`, and this design is ~250 lines against tables that exist.* Gained: no new dependency in the money path, and the fan-out shares the `jobs` table (so lease reaping, drain and metrics are one mechanism, not two). Lost: the maintainer explicitly reserved this at `docs/roadmap.md:742-746` and a hand-rolled outbox is code vpay now owns forever. **This is marked as a maintainer choice in the roadmap; do not let implementation decide it by default.**

2. **Endpoint identity: an operator-authored `id`, or a hash of the URL?** *Default: a required `id` string per endpoint, unique within a merchant, refused at boot.* Gained: an operator can fix a typo'd URL without orphaning the delivery history, and runbooks name something readable. Lost: one more required YAML field, and a duplicated `id` across two merchants is legal (deliberately — the unique index is `(event_id, endpoint_id)` and events are already merchant-scoped).

3. **Store the signed body on the delivery row, or re-render per attempt and store only its SHA-256?** *Default: re-render, store `payload_sha256`, treat a mismatch as `Poisoned`.* Gained: the invariant that matters ("we sent exactly what we signed") holds by construction within an attempt, without duplicating every event body once per endpoint. Lost: if the renderer changes mid-flight, the delivery dead-ends as `Poisoned` instead of continuing with the original bytes; storing the body would let it continue. Storing costs a `TEXT` column of unbounded-ish size times endpoints.

4. **Runtime SSRF filtering (block private/link-local ranges in livemode)?** *Default: no — boot-time `validate_host` only (https + no stub markers under livemode).* Gained: reuses a tested, familiar mechanism; keeps the WireMock receiver on a private compose address, which ADR-0006 *requires* the proof to run against. Lost: a livemode operator can point an endpoint at `https://169.254.169.254/…` and use vpay as an SSRF relay. Mitigating truth: a resolve-then-connect check is TOCTOU unless reqwest gets a custom connector, so the honest options are "nothing" or "a custom connector", not "a cheap check". If the answer is "block it", that is a separate, larger piece of work and should be scoped as such.

5. **`GET /v1/events` in this step, and with `?type=`?** *Default: build both routes, defer `?type=`.* Gained: the merchant's documented fallback for a missed webhook exists the same day webhooks do, and the renderer is shared with the deliverer so they cannot disagree. Lost: `docs/api/README.md:177` documents `type` and will keep documenting a parameter that 400s or is ignored — which must be stated in status.md, not left implicit. Deciding "ignore unknown query params" vs "400" is itself a choice; the existing handlers ignore them.

6. **Node parity test: `node -e` subprocess, or drop it?** *Default: subprocess, gated on `VPAY_REQUIRE_NODE=1` in CI so a missing `node` fails rather than skips.* Gained: the only real proof that the header the server emits is accepted by the SDK a merchant actually installs — the two verifiers have subtly different parse paths and the Rust one alone cannot prove Node's. Lost: an integration test that shells out and depends on the Node build being present; without the env gate it degrades into a silent skip, which is worse than not having it.
---

# Outcome (2026-09-03)

*Appended after the step landed. From here on `docs/status.md`,
[`docs/flows/webhooks.md`](../flows/webhooks.md) and
[`docs/runbooks/webhook-delivery-failures.md`](../runbooks/webhook-delivery-failures.md)
are the record; everything above this line is history.*

## What landed

- **Migrations `0022`, `0023` and `0024`** — `webhook_deliveries`, and
  `jobs.kind_is_known` re-opened twice: for `fan_out_events`/`deliver_webhook`
  (`0022`, asserted by
  `migration_0022_reopens_the_job_kinds_and_closes_the_delivery_states`) and
  then for `scan_deliveries` (`0023`, the backstop scan added by the security
  remediation). Seven job kinds. `0024` (the second remediation) adds
  `events.fanout_attempts` and a third `fanout_state`, `failed`, and re-issues
  `0022`'s `payload_sha256` comment.
- **`vpay_db::webhook_deliveries`** — `create_in_tx`, `get`, `record_success`,
  `record_attempt`, `mark_fanned_out_in_tx`, `pending_due`, `for_event`; and
  `events::{list_page, get_by_id}` for the read API.
- **`vpay_worker::signing::signature_header`** and
  **`vpay_worker::delivery_delay`** — the header, and the seven-rung ladder that
  ends in `None` rather than an eighth rung.
- **`vpay_worker::webhooks`** — `Endpoint`, `EndpointRegistry` (both with
  hand-written secret-redacting `Debug`), `handle_fan_out`, `handle_deliver`.
- **`vpay_api::model::EventObject`** and **`vpay_api::v1::events`** —
  `GET /v1/events`, `GET /v1/events/{id}`, one renderer shared with the
  deliverer.
- **`vpay_config::oauth::WebhookEndpoint`** and `MerchantClient::webhooks`, with
  `validate_webhook_endpoints` (missing/duplicate id, the URL rules, the 1–2
  secret count, empty secrets) **and** the `RawSecrets` extension that S3 named,
  so a livemode literal webhook secret is now a refusal to boot. The security
  remediation added four more boot rules: `id` ≤ 64 and `url` ≤ 2048 (mirroring
  `0022`'s CHECKs, so a document the database would refuse is refused at boot),
  a URL with no host or with embedded credentials, and a livemode secret shorter
  than 32 bytes once resolved.
- **The binary wiring** — `vpay-worker-bin` projects the registry, builds the
  delivery client and runs `run_loop`; `compose.e2e.yml` runs a third WireMock
  service as the receiver; `just gen-demo-keys` writes the `webhooks:` block.
- **Tests** — `backends/tests/integration/tests/webhooks.rs` (**15** of the
  integration suite's 90: fan-out idempotency, the zero-endpoint case, Rust and
  Node signature parity, `Stripe-Signature` byte-identity and grammar, rotation,
  the ladder, exhaustion, the two `GET /v1/events` cases, and — from the
  remediation — page isolation, the backstop, the no-digest branch and
  `the_real_run_loop_delivers_a_backlog_event_to_the_receiver`; and from the
  second — `a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan` and
  `a_permanently_unfannable_event_is_abandoned_after_five_passes_and_alerts_once`),
  plus the unit tests beside each function. The `run_loop` test starts from an inserted
  `events` row rather than a real confirm; `worker_e2e.rs` and `just demo`'s
  step 7 are what join settlement to it.
- **Docs** — this Outcome, the flow doc's Status, the new runbook, `docs/api/README.md`,
  the roadmap's Phase 6, and `docs/status.md`.

## Deviations from the design above

1. **The handler signature is threaded through `run_loop`, not
   `handle(..., webhooks, job)` as §8 wrote it.** `vpay_worker::handlers` gained
   a `WebhookContext<'a> { endpoints, http }` borrowed struct, which `run_loop`
   builds once per claim task and passes into `handle`/`dispatch`; the two
   handlers are then called as `handle_fan_out(pool, webhooks.endpoints, job)`
   and `handle_deliver(pool, webhooks.http, webhooks.endpoints, job)`. The
   design's `ResourceConfig::endpoints_by_merchant_id` idea (S5) was **not**
   taken for the worker: the registry is keyed on `merchant_id` and
   `ResourceConfig` is keyed on `client_id`, so the binary projects one from the
   other at boot instead. `ResourceConfig::webhook_endpoints()` is that
   projection's source.
2. **`fanout:events` is seeded with the other singletons**, in
   `run_loop::seed_singletons`, in one transaction with `sweep:expired` and
   `scan:live` — not by a separate seeder. A partial seed is worse than none: a
   deployment without `fan_out_events` settles payments and tells no merchant,
   which looks exactly like a healthy deployment until somebody reads
   `events.fanout_state`.
3. **The demo step is 7, not 6.** §8 said "demo step 6", written before Step 4
   inserted its own settlement-wait step. The receiver step is the seventh and
   last, and it now **fails** on an absent or unverifiable delivery where an
   earlier version reported the absence and passed.
4. **`backends/apps/vpay-server/tests/cli.rs` needed `MERCHANT_WEBHOOK_SECRET`.**
   `config/application.yml` now declares a webhook endpoint whose `secrets:` is a
   `${MERCHANT_WEBHOOK_SECRET}` placeholder, and an unresolved placeholder is a
   refusal to boot (exit 78) — so the server's subprocess CLI test had to set the
   variable. `compose.e2e.yml` sets it on **both** the server and the worker for
   the same reason: both processes load and validate the same document.
5. **A second `deliver_webhook` failure mode was added that the design did not
   name:** an endpoint the registry no longer holds, or holds with no secret, is
   recorded as an ordinary failed attempt (with a `WARN`) rather than sent
   unsigned or exhausted on the spot.
6. **The security remediation added a job kind the design did not have.**
   `scan:deliveries` (`JobKind::ScanDeliveries`, migration `0023`) gives
   `webhook_deliveries::pending_due` the shipping caller §8 wrote it without —
   every 10 minutes, 500 rows a pass, over both a passed `next_attempt_at` and a
   never-attempted row older than `RecoveryPolicy::lease`. `0023` also corrects
   `0022`'s `webhook_deliveries_live_idx` comment, which had described a scan
   that did not exist. The remediation further made `handle_fan_out` continue
   past a failing event (alert, count, move on, and wait the idle interval if
   the pass drained nothing) rather than abandoning the rest of the page; added
   the boot bounds and the livemode secret floor; made `record_attempt` take an
   `Option` digest so the unconfigured-endpoint branch stores none; hoisted the
   client budgets into shared `pub const`s; and added
   `the_real_run_loop_delivers_a_backlog_event_to_the_receiver`, which is what
   retired `webhooks.rs`'s module-doc claim that `run_loop` does not exist in
   this build.
7. **A second remediation (2026-09-03) closed four review findings, and the
   heaviest of them was a documentation claim rather than a code defect.**

   - **The backstop cannot recover a *dead-lettered* delivery job**, and six
     places said it could (`webhooks.rs`, `jobs.rs`, migration `0023`,
     `docs/status.md`, `docs/flows/webhooks.md`, and the backstop test's own
     doc comment). `jobs::dead_letter` parks the row and keeps its
     `dedupe_key`, so the scan's `ON CONFLICT DO NOTHING` insert is a no-op.
     The **behaviour was kept** — un-parking a poisoned job on a timer is the
     hot loop parking exists to prevent — and every claim was corrected. The
     scan now emits one `WARN` per pass naming such deliveries
     (`vpay_db::jobs::parked_dedupe_keys`), and
     `a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan` pins the
     negative. The un-park procedure is in the runbook, and is **unrun**.
   - **A permanently unfannable event was an unbounded alert storm and held a
     page slot forever.** Migration `0024` adds `events.fanout_attempts` and
     `fanout_state = 'failed'`; `FANOUT_MAX_ATTEMPTS` (5) failures abandon the
     event, which then leaves `pending_page`. Before the ceiling each failure
     is a `WARN` with no `alert`; the transition is exactly one
     `ERROR … alert = true`. So 99 poisoned events cost 99 alerts in total
     rather than 99 per pass. Re-arming a `failed` event is a manual `UPDATE`
     (runbook, also unrun).
   - **`handle_scan_deliveries`' doc comment argued the inverse of the fan-out
     lesson** — that sharing one transaction across the page was safe. That is
     not why the pass is safe; it is safe because the schema gives it no
     per-row failure mode (no length CHECK on `dedupe_key`, a fixed payload
     shape, no operator-authored value anywhere in the write). The comment now
     says that and names what would invalidate it. A failing pass also logs
     `alert = true` before returning, keeping the `Retry::AfterBackoff`
     reschedule.
   - **Webhook URL validation moved off `validate_host` onto a sibling**,
     `vpay_config::validate_webhook_url`: `starts_with("https://")` refused an
     uppercase scheme, and a stub-marker search over the whole URL refused a
     merchant's own `/mockups` path. A URL must now name a host in **both**
     deployments (a sandbox `mailto:x` used to boot). `validate_host` is
     unchanged, so the rail path is untouched.
   - Smaller: `payload_sha256`'s "first attempt" is really "the first attempt
     that rendered and signed a body" (`0024` re-issues the `COMMENT`, on
     `0023`'s precedent), and `handlers.rs`' prose names
     `WEBHOOK_{CONNECT,REQUEST}_TIMEOUT` instead of spelling 5 s and 10 s.

## What is not done

- **No merchant endpoint has ever been POSTed to.** Every delivery was to a
  WireMock host on a compose network.
- **No runtime SSRF filtering** (decision 4, taken deliberately). Boot-time
  `validate_webhook_url` is the only URL check there is.
- **No `?type=` filter** on `GET /v1/events` (decision 5, deferred deliberately).
  It is ignored, not refused.
- **No replay path beyond a hand-written transaction** and no operator CLI. The
  `scan:deliveries` backstop recovers a *deleted or lost* job within ten
  minutes; it cannot resurrect an `exhausted` delivery (not `pending`), and it
  deliberately does not resurrect one whose job was **dead-lettered** (the
  parked row still holds the `dedupe_key`, so the re-enqueue is a no-op — see
  the second remediation below). Nor does anything re-arm a `failed` event.
  All three are manual `UPDATE`s in the runbook, and none of the three has
  been run against a deployment.
- **No ordering guarantee.** Concurrent claims and the retry ladder can reorder
  two of one merchant's events; nothing in the design provides ordering.
- **No SSRF protection.** `validate_webhook_url` checks the scheme and four
  host substrings and never inspects the destination address (decision 4).
- **No webhook for five of the seven documented event types** — events are
  written for terminal transitions only.
- **The 1 h, 6 h and 24 h rungs have never elapsed** anywhere.
- ~~The 5 s / 10 s webhook client budgets are written twice.~~ **Retracted:
  fixed by the remediation.** `WEBHOOK_CONNECT_TIMEOUT` and
  `WEBHOOK_REQUEST_TIMEOUT` are single `pub const`s in
  `vpay_worker::webhooks`, and both `vpay-worker-bin`'s `main.rs` and the
  integration suite's `delivery_client()` read them.
