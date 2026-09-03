//! Outbound webhooks end to end: fan-out, signing, the retry ladder, and
//! the `GET /v1/events` fallback a merchant reads when a delivery never
//! arrives.
//!
//! Every test here runs against a **real Postgres container** and a **real
//! WireMock container** standing in for a merchant's receiver — a host in
//! configuration, reached over HTTP exactly as a real merchant's endpoint
//! would be (ADR-0006 / AGENTS.md rule 1). Nothing is stubbed in-process,
//! and the bytes asserted on are read back out of the receiver's own request
//! journal (`GET /__admin/requests`), not out of the sender.
//!
//! # Why most tests drive the handlers directly, and one drives the loop
//!
//! `vpay_worker::webhooks::{handle_fan_out, handle_deliver}` are called here
//! with a `JobRow` claimed from the real `jobs` table. That is what makes the
//! ladder and the exhaustion cases possible at all: they need to stage a
//! delivery's `attempt` counter and step it one rung at a time, which a loop
//! running on its own schedule cannot be asked to do.
//!
//! It is not the whole of what this file proves, and it must not be. Calling
//! a handler proves the handler; it does not prove that anything in a running
//! process ever calls it — a `run_loop` that stopped seeding `fanout:events`,
//! or a `dispatch` arm that stopped routing `deliver_webhook`, would leave
//! every direct-call test here green and every merchant unnotified. So
//! `the_real_run_loop_delivers_a_backlog_event_to_the_receiver`
//! drives the *shipping* `vpay_worker::run_loop` — the same function
//! `vpay-worker-bin`'s `main` calls, with a real `EndpointRegistry` pointed
//! at the WireMock receiver — from an unfanned-out event all the way to a
//! POST in the receiver's journal, and nothing else in this file covers that
//! seam.
//!
//! # Signature parity is proved twice, in two languages
//!
//! The header vpay emits is fed to `vpay_sdk::webhooks::verify_at` (the Rust
//! SDK a merchant installs) and, in a subprocess, to `@vpay/sdk`'s
//! `verifyWebhook` (the Node one). The two verifiers have different parse
//! paths — Node's `t` is a regex-checked string, Rust's is a checked
//! `i64` — so the Rust test alone cannot prove Node's. The Node case
//! **fails** rather than skips when `node` is missing; see
//! `node_verifier_available`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::Context as _;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use time::OffsetDateTime;
use vpay_api::op::keys::LoadedSigningKey;
use vpay_config::{Config, CurrencyEntry, Deployment};
use vpay_db::{DeliveryRow, EventRow, JobRow, NewEvent};
use vpay_worker::jobs::{FANOUT_DEDUPE_KEY, webhook_dedupe_key};
use vpay_worker::webhooks::{
    Endpoint, EndpointRegistry, event_bytes, handle_deliver, handle_fan_out, payload_sha256,
};
use vpay_worker::{Outcome, delivery_delay};

use support::{
    ensure_crypto_provider_installed, generate_key, merchant_client_with, migrated_postgres, serve,
    webhook_endpoint,
};

/// The tenant every event in this file belongs to, and the credential that
/// may read it. Never the same string: a query filtered by `client_id`
/// instead of `merchant_id` would otherwise pass every tenancy assertion.
const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";
const CLIENT_B: &str = "other-merchant";
const MERCHANT_B: &str = "other-merchant-tenant";

/// The endpoint id and secret the configured receiver is registered under.
const ENDPOINT_ID: &str = "primary";
const SECRET: &str = "whsec_integration_primary";
/// The second secret, present only in the rotation test.
const SECRET_NEXT: &str = "whsec_integration_incoming";

/// The worker identity every `jobs::claim` in this file presents. One
/// string, because `finish`/`reschedule` refuse a lease held by anyone else
/// and a typo would look like a lease-reaping bug.
const WORKER: &str = "webhooks-integration";

/// The `Vpay-Signature` tolerance the SDKs default to
/// (`docs/flows/webhooks.md`). Used as-is: a test that widened it would stop
/// proving that the `t` vpay writes is a *current* timestamp.
const TOLERANCE: Duration = Duration::from_secs(300);

/// The WireMock root this suite bind-mounts — the same directory
/// `compose.e2e.yml` mounts into `wiremock-webhook`, so the mappings a
/// developer's stack answers with are the mappings these tests assert
/// against.
fn receiver_mappings_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../webhook-receiver/wiremock")
}

/// This test binary's Prometheus recorder, installed on first use.
///
/// A near-copy of `worker_e2e.rs`'s static of the same name, and duplicated
/// rather than shared for the same reason that one gives: each file under
/// `tests/` is its own binary, `metrics::set_global_recorder` succeeds
/// exactly once per process, and under `cargo nextest` that is once per
/// test — the property the exact-count assertions below rely on.
static METRICS: LazyLock<PrometheusHandle> = LazyLock::new(|| {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    metrics::set_global_recorder(recorder).expect("this test binary installs exactly one recorder");
    vpay_core::metrics::describe_all();
    handle
});

/// Serves the **shipping** observability router
/// (`vpay_api::observability`, the same function both `main.rs` files call)
/// on an ephemeral port, rendering [`METRICS`] — see `worker_e2e.rs`'s
/// identical helper for why this goes through a real socket rather than
/// `PrometheusHandle::render()` directly.
async fn serve_metrics() -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let handle = METRICS.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding the observability listener")?;
    let addr = listener.local_addr().context("reading the bound address")?;
    let task = tokio::spawn(async move {
        let _ = vpay_api::observability::serve(
            listener,
            move || handle.render(),
            std::future::pending::<()>(),
        )
        .await;
    });
    Ok((addr, task))
}

// ------------------------------------------------------------------ harness

/// Postgres, a receiver, and the endpoint registry pointed at it.
struct Harness {
    _postgres: ContainerAsync<PostgresImage>,
    _receiver: ContainerAsync<GenericImage>,
    pool: PgPool,
    /// `http://127.0.0.1:<mapped>` — the receiver as seen from this process.
    receiver_url: String,
    /// Merchant A has one endpoint at `/webhooks`; merchant B has none.
    endpoints: EndpointRegistry,
}

impl Harness {
    /// A registry in which merchant A's endpoint carries `secrets`.
    ///
    /// Built from the *same* `(merchant_id, endpoints)` pairs the worker
    /// binary will project out of `vpay_api::ResourceConfig`, so a suite
    /// cannot register an endpoint shape the binary could not produce.
    fn registry_with_secrets(&self, secrets: &[&str]) -> EndpointRegistry {
        EndpointRegistry::from_pairs([(
            MERCHANT_A.to_owned(),
            vec![Endpoint {
                id: ENDPOINT_ID.to_owned(),
                url: format!("{}/webhooks", self.receiver_url),
                secrets: secrets.iter().map(|s| (*s).to_owned()).collect(),
            }],
        )])
    }

    /// A registry whose only endpoint is the flaky path.
    fn flaky_registry(&self) -> EndpointRegistry {
        EndpointRegistry::from_pairs([(
            MERCHANT_A.to_owned(),
            vec![Endpoint {
                id: ENDPOINT_ID.to_owned(),
                url: format!("{}/flaky", self.receiver_url),
                secrets: vec![SECRET.to_owned()],
            }],
        )])
    }
}

async fn harness() -> anyhow::Result<Harness> {
    ensure_crypto_provider_installed();

    let (postgres, pool) = migrated_postgres().await?;
    let receiver = vpay_testkit::containers::start_wiremock(&receiver_mappings_dir())
        .await
        .context("the merchant webhook receiver container starts")?;
    let receiver_url = format!(
        "http://127.0.0.1:{}",
        receiver
            .get_host_port_ipv4(8080)
            .await
            .context("the receiver's mapped port")?
    );

    let endpoints = EndpointRegistry::from_pairs([(
        MERCHANT_A.to_owned(),
        vec![Endpoint {
            id: ENDPOINT_ID.to_owned(),
            url: format!("{receiver_url}/webhooks"),
            secrets: vec![SECRET.to_owned()],
        }],
    )]);

    Ok(Harness {
        _postgres: postgres,
        _receiver: receiver,
        pool,
        receiver_url,
        endpoints,
    })
}

/// The outbound client the worker binary builds at boot: the same call, and
/// the same two budgets read from the same constants.
///
/// `vpay_worker::WEBHOOK_{CONNECT,REQUEST}_TIMEOUT`, which is where the
/// handler that spends them lives — not `5` and `10` written out a third
/// time. Nothing used to pin the binary's pair to this one, so a change there
/// would have left this suite proving a client that no longer ships.
///
/// Not `reqwest::Client::new()`: that one panics in the `scratch` runtime
/// image, and a test that used it would not be exercising the client that
/// ships.
fn delivery_client() -> reqwest::Client {
    vpay_provider::http::client_with_timeouts(
        vpay_worker::WEBHOOK_CONNECT_TIMEOUT,
        vpay_worker::WEBHOOK_REQUEST_TIMEOUT,
    )
    .expect("the vendored-roots client builds")
}

// ------------------------------------------------------------- fixtures ---

/// Appends one `payment_intent.succeeded` event for `merchant_id`, in its
/// own transaction, exactly as `vpay_db::settlement` will.
async fn insert_event(
    pool: &PgPool,
    merchant_id: &str,
    object_id: &str,
) -> anyhow::Result<EventRow> {
    let mut tx = pool.begin().await?;
    let row = vpay_db::events::insert_in_tx(
        &mut tx,
        &NewEvent {
            id: vpay_db::events::event_id(),
            merchant_id: merchant_id.to_owned(),
            livemode: false,
            event_type: "payment_intent.succeeded".to_owned(),
            object_id: object_id.to_owned(),
            data: json!({
                "id": object_id,
                "object": "payment_intent",
                "amount": 5000,
                "currency": "eur",
                "status": "succeeded",
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Seeds the singleton fan-out job the worker binary seeds at boot, and
/// claims it — so the `JobRow` handed to `handle_fan_out` is a real row with
/// a real lease, not a struct literal.
async fn claim_fanout_job(pool: &PgPool) -> anyhow::Result<JobRow> {
    let mut tx = pool.begin().await?;
    vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        "fan_out_events",
        FANOUT_DEDUPE_KEY,
        &json!({}),
        OffsetDateTime::now_utc(),
    )
    .await?;
    tx.commit().await?;

    vpay_db::jobs::claim(pool, WORKER)
        .await?
        .context("the fan-out job is claimable")
}

/// Claims the `deliver_webhook` job the fan-out enqueued for `delivery_id`.
///
/// By dedupe key rather than by `jobs::claim`, because `claim` takes
/// *whichever* job is due — including the fan-out job that is still
/// rescheduling itself — and a test that grabbed the wrong one would fail
/// somewhere unrelated to what it is asserting.
async fn claim_delivery_job(pool: &PgPool, delivery_id: uuid::Uuid) -> anyhow::Result<JobRow> {
    let key = webhook_dedupe_key(delivery_id);
    let row: JobRow = sqlx::query_as(
        "UPDATE jobs SET locked_at = now(), locked_by = $1, attempts = attempts + 1 \
         WHERE dedupe_key = $2 \
         RETURNING id, kind, dedupe_key, payload, run_at, attempts, locked_by, last_error",
    )
    .bind(WORKER)
    .bind(&key)
    .fetch_one(pool)
    .await
    .with_context(|| format!("a deliver_webhook job exists under {key}"))?;
    Ok(row)
}

/// Every `jobs` row's `(kind, dedupe_key)`, for counting.
async fn jobs_of_kind(pool: &PgPool, kind: &str) -> anyhow::Result<Vec<String>> {
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT dedupe_key FROM jobs WHERE kind = $1 ORDER BY dedupe_key")
            .bind(kind)
            .fetch_all(pool)
            .await?;
    Ok(keys)
}

async fn fanout_state(pool: &PgPool, event_id: &str) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar("SELECT fanout_state FROM events WHERE id = $1")
            .bind(event_id)
            .fetch_one(pool)
            .await?,
    )
}

/// Forces a delivery's `attempt` to `value` — how the exhaustion case is
/// staged.
///
/// Writing the counter directly is the only honest way to reach the last
/// rung: the alternative is eight real attempts separated by up to 24 hours
/// of `next_attempt_at`, which is not a test. What it stages is exactly the
/// state seven recorded failures leave behind, and nothing else about the
/// row is touched.
async fn force_attempt(pool: &PgPool, delivery_id: uuid::Uuid, value: i32) -> anyhow::Result<()> {
    sqlx::query("UPDATE webhook_deliveries SET attempt = $2 WHERE id = $1")
        .bind(delivery_id)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

// ------------------------------------------------- the receiver's journal --

/// One request the receiver recorded: the exact body bytes and the headers.
#[derive(Debug, Clone)]
struct Recorded {
    body: Vec<u8>,
    headers: BTreeMap<String, String>,
    url: String,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// `^t=\d+,v1=[0-9a-f]{64}(,v1=[0-9a-f]{64})*$` — Stripe's documented
/// `Stripe-Signature` grammar, as a matcher.
///
/// The strictness is the point, and every part of it is a real failure mode
/// of a signer: a `t` with a sign or an offset (`t=+1753401600`) is what a
/// re-rendered timestamp looks like, uppercase hex is what a different
/// `hex::encode` produces, and a truncated digest is what a `[..32]` slice
/// produces. Any of the three verifies fine in vpay's own SDKs and is refused
/// by Stripe's.
fn matches_stripe_signature_grammar(header: &str) -> bool {
    let mut parts = header.split(',');

    let Some(timestamp) = parts.next().and_then(|part| part.strip_prefix("t=")) else {
        return false;
    };
    if timestamp.is_empty() || !timestamp.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }

    let mut signatures = 0_usize;
    for part in parts {
        let Some(digest) = part.strip_prefix("v1=") else {
            return false;
        };
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return false;
        }
        signatures += 1;
    }
    // At least one `v1=`: `t=1753401600` alone matches every other rule here
    // and carries no signature at all.
    signatures >= 1
}

/// Reads WireMock's request journal.
///
/// This is what makes the signature assertions real: the bytes and the
/// header come back from the *receiver*, over HTTP, exactly as a merchant's
/// own server would have seen them. Re-signing in the test and comparing
/// would only prove that one function agrees with itself.
async fn journal(receiver_url: &str) -> anyhow::Result<Vec<Recorded>> {
    let body: Value = reqwest::get(format!("{receiver_url}/__admin/requests"))
        .await?
        .json()
        .await?;
    let entries = body
        .get("requests")
        .and_then(Value::as_array)
        .context("WireMock's journal has a `requests` array")?;

    let mut out = Vec::new();
    for entry in entries {
        let request = entry
            .get("request")
            .context("a journal entry has a request")?;
        if request.get("method").and_then(Value::as_str) != Some("POST") {
            continue;
        }
        let headers = request
            .get("headers")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.to_ascii_lowercase(), v.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        out.push(Recorded {
            body: request
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .as_bytes()
                .to_vec(),
            headers,
            url: request
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
    }
    // WireMock returns newest first; chronological order is what a ladder
    // assertion reads.
    out.reverse();
    Ok(out)
}

// ------------------------------------------------------------- fan-out ----

/// One delivery row and one job per (event, endpoint), the event flipped to
/// `done` — and running the pass a second time changes nothing.
///
/// Idempotency here is not a nicety: the fan-out's transaction *is* the
/// crash-safety mechanism, so a second pass over an already-fanned-out event
/// is what a crashed-and-restarted worker does. Deleting
/// `mark_fanned_out_in_tx` from the handler makes this test fail with two
/// deliveries and two jobs, which is exactly the duplicate webhook a
/// merchant would see.
#[tokio::test]
async fn fan_out_creates_one_delivery_and_one_job_per_endpoint_and_is_idempotent() {
    let h = harness().await.expect("harness");
    let event = insert_event(&h.pool, MERCHANT_A, "pi_fanout")
        .await
        .expect("an event to fan out");
    assert_eq!(
        fanout_state(&h.pool, &event.id).await.expect("state"),
        "pending",
        "an event is born pending or nothing would ever deliver it"
    );

    let job = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    let outcome = handle_fan_out(&h.pool, &h.endpoints, &job)
        .await
        .expect("the first pass succeeds");
    assert!(
        matches!(outcome, Outcome::RescheduleAfter(_)),
        "the fan-out pass reschedules itself rather than finishing: {outcome:?}"
    );

    let deliveries = vpay_db::webhook_deliveries::for_event(&h.pool, &event.id)
        .await
        .expect("deliveries");
    assert_eq!(deliveries.len(), 1, "one endpoint, one delivery");
    let created = deliveries.first().expect("one delivery").clone();
    assert_eq!(created.endpoint_id, ENDPOINT_ID);
    assert_eq!(created.state, "pending");
    assert_eq!(created.attempt, 0);
    assert_eq!(
        created.url,
        format!("{}/webhooks", h.receiver_url),
        "the URL is denormalised onto the row for forensics (migration 0022)"
    );
    assert_eq!(
        fanout_state(&h.pool, &event.id).await.expect("state"),
        "done"
    );

    let jobs = jobs_of_kind(&h.pool, "deliver_webhook")
        .await
        .expect("jobs");
    assert_eq!(jobs, vec![webhook_dedupe_key(created.id)]);

    // The replay. The event is `done`, so `pending_page` no longer returns
    // it and nothing at all should change.
    let outcome = handle_fan_out(&h.pool, &h.endpoints, &job)
        .await
        .expect("the second pass succeeds");
    assert!(matches!(outcome, Outcome::RescheduleAfter(_)));

    let after = vpay_db::webhook_deliveries::for_event(&h.pool, &event.id)
        .await
        .expect("deliveries");
    assert_eq!(after, deliveries, "a second pass created a second delivery");
    assert_eq!(
        jobs_of_kind(&h.pool, "deliver_webhook")
            .await
            .expect("jobs"),
        jobs,
        "a second pass enqueued a second job"
    );
}

/// A merchant with **no** configured endpoints still has their events marked
/// `done`, with zero deliveries.
///
/// Leaving them `pending` so that "someone might configure an endpoint
/// later" would grow `events_pending_idx` (migration 0018) without bound and
/// re-scan the same rows on every pass, forever.
#[tokio::test]
async fn an_event_for_a_merchant_with_no_endpoints_is_still_fanned_out() {
    let h = harness().await.expect("harness");
    let event = insert_event(&h.pool, MERCHANT_B, "pi_no_endpoints")
        .await
        .expect("an event");

    let job = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    handle_fan_out(&h.pool, &h.endpoints, &job)
        .await
        .expect("the pass succeeds");

    assert_eq!(
        fanout_state(&h.pool, &event.id).await.expect("state"),
        "done",
        "an event nobody has an endpoint for must not stay in the backlog"
    );
    assert!(
        vpay_db::webhook_deliveries::for_event(&h.pool, &event.id)
            .await
            .expect("deliveries")
            .is_empty()
    );
    assert!(
        jobs_of_kind(&h.pool, "deliver_webhook")
            .await
            .expect("jobs")
            .is_empty()
    );
}

// ---------------------------------------------------- signature parity ----

/// Fans out and delivers one event, and hands back what the receiver saw.
async fn deliver_one(
    h: &Harness,
    endpoints: &EndpointRegistry,
    object_id: &str,
) -> (EventRow, DeliveryRow, Recorded) {
    let event = insert_event(&h.pool, MERCHANT_A, object_id)
        .await
        .expect("an event");
    let job = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    handle_fan_out(&h.pool, endpoints, &job)
        .await
        .expect("fan-out");

    let delivery = vpay_db::webhook_deliveries::for_event(&h.pool, &event.id)
        .await
        .expect("deliveries")
        .pop()
        .expect("one delivery");
    let job = claim_delivery_job(&h.pool, delivery.id)
        .await
        .expect("the delivery job");

    let outcome = handle_deliver(&h.pool, &delivery_client(), endpoints, &job)
        .await
        .expect("the delivery handler ran");
    assert!(
        matches!(outcome, Outcome::Done),
        "a 2xx receiver ends the job: {outcome:?}"
    );

    let recorded = journal(&h.receiver_url)
        .await
        .expect("the receiver's journal")
        .pop()
        .expect("the receiver recorded a POST");
    let delivery = vpay_db::webhook_deliveries::get(&h.pool, delivery.id)
        .await
        .expect("the delivery row")
        .expect("the delivery still exists");

    (event, delivery, recorded)
}

/// The bytes the receiver got verify against the Rust SDK a merchant
/// installs, and one flipped byte does not.
///
/// The negative half is what makes the positive half mean anything: a
/// verifier that returned `Ok` unconditionally would pass the first
/// assertion.
#[tokio::test]
async fn the_delivered_signature_verifies_with_the_shipping_rust_sdk() {
    let h = harness().await.expect("harness");
    let (event, delivery, recorded) = deliver_one(&h, &h.endpoints, "pi_rust_parity").await;

    assert_eq!(recorded.url, "/webhooks");
    assert_eq!(
        recorded.header("vpay-event-id"),
        Some(event.id.as_str()),
        "the convenience header carries the event id merchants dedupe on"
    );
    assert_eq!(
        recorded.header("content-type").map(|v| v
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()),
        Some("application/json".to_owned())
    );

    let signature = recorded
        .header("vpay-signature")
        .expect("the delivery carried a Vpay-Signature");

    // The Stripe-named header, which is the whole of what a merchant needs to
    // hand this request to `stripe.webhooks.constructEvent`. Asserted on the
    // *receiver's* copy and asserted byte-identical: the deliverer sends one
    // computed value under two names, and a build that recomputed the second
    // one would emit a different `t` and fail every merchant verifying
    // through the Stripe SDK while this suite's Vpay-Signature assertions
    // stayed green.
    let stripe_signature = recorded
        .header("stripe-signature")
        .expect("the delivery carried a Stripe-Signature");
    assert_eq!(
        stripe_signature, signature,
        "Stripe-Signature must be byte-identical to Vpay-Signature"
    );

    // And it matches Stripe's documented grammar. `t=` then the unix seconds,
    // then one or more `v1=` values of exactly 64 lowercase hex characters —
    // a SHA-256 digest. Transcribed from Stripe's "Verify webhook
    // signatures" documentation rather than from our own signer, so a signer
    // that started emitting uppercase hex, a `+HH:MM` offset in `t`, or a
    // `v0=` scheme would fail here instead of at a merchant.
    //
    // Written as a hand-rolled matcher rather than a `regex` dependency this
    // suite does not otherwise have; the grammar is small enough that the
    // matcher is readable and the alternative is a crate in the graph for one
    // assertion.
    assert!(
        matches_stripe_signature_grammar(signature),
        "the signature header does not match ^t=\\d+,v1=[0-9a-f]{{64}}(,v1=[0-9a-f]{{64}})*$: \
         {signature}"
    );

    let verified = vpay_sdk::webhooks::verify(&recorded.body, signature, SECRET, TOLERANCE)
        .expect("the shipping Rust SDK verifies the header vpay emitted");
    assert_eq!(verified.id, event.id);
    assert_eq!(verified.kind, "payment_intent.succeeded");
    assert_eq!(
        verified.data.object.get("id"),
        Some(&json!("pi_rust_parity"))
    );

    // One byte, and the signature no longer covers the body.
    let mut tampered = recorded.body.clone();
    let last = tampered.last_mut().expect("a non-empty body");
    *last ^= 0x20;
    let error = vpay_sdk::webhooks::verify(&tampered, signature, SECRET, TOLERANCE)
        .expect_err("a tampered body must not verify");
    assert!(
        matches!(
            error,
            vpay_sdk::Error::Webhook(vpay_sdk::WebhookError::SignatureMismatch)
        ),
        "{error:?}"
    );

    // And the row says what happened.
    assert_eq!(delivery.state, "succeeded");
    assert_eq!(delivery.status_code, Some(200));
    assert_eq!(
        delivery.attempt, 0,
        "a first-attempt success failed nothing"
    );
    assert_eq!(
        delivery.payload_sha256.as_deref(),
        Some(payload_sha256(&event_bytes(&event).expect("renders")).as_str()),
        "the stored digest is the digest of the bytes that went out"
    );
}

/// The same header, verified by the **Node** SDK a merchant installs, in a
/// subprocess.
///
/// # Why this cannot be a fixture
///
/// Feeding `@vpay/sdk` its own test vectors would prove the Node SDK agrees
/// with itself. What has to be proved is that the bytes *this server* emits
/// are accepted there — the two verifiers parse `t` differently (Node:
/// `/^\d+$/` over the literal text; Rust: a checked `i64`), and only a real
/// delivery exercises both.
///
/// # Why a missing `node` is a failure and never a skip
///
/// A skip is how this suite would go green while proving nothing, which is
/// the failure mode `CLAUDE.md` names first. `VPAY_REQUIRE_NODE=1` (set in
/// CI's `rust` job) makes the message say so explicitly; without it the test
/// still fails, just with a message that also tells a developer how to get
/// `node`.
#[tokio::test]
async fn the_delivered_signature_verifies_with_the_shipping_node_sdk() {
    let h = harness().await.expect("harness");
    let (event, _delivery, recorded) = deliver_one(&h, &h.endpoints, "pi_node_parity").await;
    let signature = recorded
        .header("vpay-signature")
        .expect("the delivery carried a Vpay-Signature")
        .to_owned();

    let verified = verify_with_node(&recorded.body, &signature, SECRET)
        .expect("the shipping Node SDK verifies the header vpay emitted");
    assert_eq!(
        verified.get("id").and_then(Value::as_str),
        Some(event.id.as_str())
    );
    assert_eq!(
        verified.get("type").and_then(Value::as_str),
        Some("payment_intent.succeeded")
    );

    // The decisive negative, run through the same subprocess: the wrong
    // secret must be refused. Without it, a `verifyWebhook` that ignored the
    // signature entirely would pass the assertion above.
    let error = verify_with_node(&recorded.body, &signature, "whsec_the_wrong_secret")
        .expect_err("the Node verifier must refuse a signature it cannot reproduce");
    assert!(
        error.contains("WebhookSignatureError") || error.contains("signature"),
        "the Node failure should be a signature refusal, got: {error}"
    );
}

/// Two configured secrets produce two `v1=` values, and **each one**
/// verifies independently.
///
/// That is the whole of what a rotation needs: while both are configured, a
/// receiver still holding the old secret and one that has taken the new one
/// both succeed, so the cutover has no window in which deliveries fail.
#[tokio::test]
async fn a_rotation_signs_with_both_secrets_and_either_one_verifies() {
    let h = harness().await.expect("harness");
    let endpoints = h.registry_with_secrets(&[SECRET, SECRET_NEXT]);
    let (event, _delivery, recorded) = deliver_one(&h, &endpoints, "pi_rotation").await;

    let signature = recorded
        .header("vpay-signature")
        .expect("a Vpay-Signature")
        .to_owned();
    assert_eq!(
        signature.matches("v1=").count(),
        2,
        "two configured secrets must produce two v1= values: {signature}"
    );
    // The grammar's repeating half. A one-secret header exercises only
    // `t=…,v1=…`; this is the case that pins `(,v1=…)*`, which is the shape a
    // merchant mid-rotation actually receives.
    assert!(
        matches_stripe_signature_grammar(&signature),
        "a rotation's two-signature header must still match Stripe's grammar: {signature}"
    );
    assert_eq!(
        recorded.header("stripe-signature"),
        Some(signature.as_str()),
        "both v1= values must reach the Stripe-named header too"
    );

    for (index, secret) in [SECRET, SECRET_NEXT].into_iter().enumerate() {
        let verified = vpay_sdk::webhooks::verify(&recorded.body, &signature, secret, TOLERANCE)
            .unwrap_or_else(|error| {
                panic!("secrets[{index}] must verify independently: {error:?}")
            });
        assert_eq!(verified.id, event.id);
    }

    // A third party's secret still does not.
    assert!(
        vpay_sdk::webhooks::verify(
            &recorded.body,
            &signature,
            "whsec_not_configured",
            TOLERANCE
        )
        .is_err()
    );
}

// --------------------------------------------------------- the ladder -----

/// Three refusals then a success: `attempt` counts up, each
/// `next_attempt_at` is `now + delivery_delay(attempt_before)`, and the
/// delivery ends `succeeded`.
///
/// The deltas are asserted against `delivery_delay` itself rather than
/// against transcribed numbers, because the ladder in
/// `docs/flows/webhooks.md` is one table and a second copy here would be a
/// second thing to keep in step. What *is* transcribed is the first rung —
/// ten seconds — so a `delivery_delay` that returned `Some(ZERO)` for
/// everything could not pass.
#[tokio::test]
async fn the_ladder_walks_delivery_delay_and_then_succeeds() {
    let h = harness().await.expect("harness");
    let endpoints = h.flaky_registry();
    let http = delivery_client();
    let (metrics_addr, metrics_task) = serve_metrics().await.expect("the metrics listener");

    let event = insert_event(&h.pool, MERCHANT_A, "pi_flaky")
        .await
        .expect("an event");
    let fanout = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    handle_fan_out(&h.pool, &endpoints, &fanout)
        .await
        .expect("fan-out");
    let delivery_id = vpay_db::webhook_deliveries::for_event(&h.pool, &event.id)
        .await
        .expect("deliveries")
        .pop()
        .expect("one delivery")
        .id;

    assert_eq!(
        delivery_delay(0),
        Some(Duration::from_secs(10)),
        "the first rung of docs/flows/webhooks.md's ladder"
    );

    for rung in 0_u32..3 {
        let job = claim_delivery_job(&h.pool, delivery_id)
            .await
            .expect("the delivery job");
        let before = OffsetDateTime::now_utc();
        let outcome = handle_deliver(&h.pool, &http, &endpoints, &job)
            .await
            .expect("the handler ran");

        let expected = delivery_delay(rung).expect("rung {rung} is on the ladder");
        assert!(
            matches!(outcome, Outcome::RescheduleAfter(delay) if delay == expected),
            "rung {rung}: {outcome:?} is not RescheduleAfter({expected:?})"
        );

        let row = vpay_db::webhook_deliveries::get(&h.pool, delivery_id)
            .await
            .expect("the delivery row")
            .expect("it still exists");
        assert_eq!(
            row.attempt,
            i32::try_from(rung).expect("a small rung") + 1,
            "rung {rung}: `attempt` counts failures so far"
        );
        assert_eq!(row.state, "pending", "rung {rung}: still owed an attempt");
        assert_eq!(row.status_code, Some(500), "rung {rung}");
        assert_eq!(
            row.response_excerpt.as_deref(),
            Some("receiver is having a bad day"),
            "rung {rung}: the receiver's own words reach the runbook column"
        );

        // `next_attempt_at` is `now + delivery_delay(rung)`, allowing for
        // the wall-clock the handler itself took.
        let scheduled = row
            .next_attempt_at
            .expect("a pending row names its next attempt");
        let delta = scheduled - before;
        let expected = time::Duration::try_from(expected).expect("the rung fits");
        assert!(
            delta >= expected && delta <= expected + time::Duration::seconds(30),
            "rung {rung}: next_attempt_at is {delta:?} out, expected about {expected:?}"
        );

        // The loop would reschedule the job; this does the same write so the
        // next claim is a real reclaim rather than a second lease.
        // The loop would reschedule the job by the delay the handler asked
        // for; `Duration::ZERO` here so the next claim in this test is
        // immediate. The *row* still carries the real `next_attempt_at`,
        // which is the value asserted above — the job's `run_at` is the
        // loop's bookkeeping, not the delivery's schedule.
        vpay_db::jobs::reschedule(
            &h.pool,
            job.id,
            WORKER,
            Duration::ZERO,
            Some("receiver refused"),
        )
        .await
        .expect("the job reschedules");
    }

    // The fourth attempt: the receiver has recovered.
    let job = claim_delivery_job(&h.pool, delivery_id)
        .await
        .expect("the delivery job");
    let outcome = handle_deliver(&h.pool, &http, &endpoints, &job)
        .await
        .expect("the handler ran");
    assert!(matches!(outcome, Outcome::Done), "{outcome:?}");

    let row = vpay_db::webhook_deliveries::get(&h.pool, delivery_id)
        .await
        .expect("the delivery row")
        .expect("it still exists");
    assert_eq!(row.state, "succeeded");
    assert_eq!(row.status_code, Some(200));
    assert_eq!(row.attempt, 3, "three failures, then a success");

    // Four POSTs really did leave this process.
    let posts = journal(&h.receiver_url).await.expect("journal");
    assert_eq!(posts.len(), 4, "one POST per attempt");
    let first_body = posts.first().expect("at least one POST").body.clone();
    assert!(
        posts.iter().all(|post| post.body == first_body),
        "every attempt must send byte-identical bytes — that is what payload_sha256 asserts"
    );

    // The metrics half: three failed attempts and one success, each counted
    // at the seam in `vpay_worker::webhooks::handle_deliver` /
    // `record_failure` after the row's write commits — the exact counts a
    // single delivery walking three rungs and then succeeding must produce.
    let scrape = reqwest::get(format!("http://{metrics_addr}/metrics"))
        .await
        .expect("scraping /metrics off the observability listener")
        .text()
        .await
        .expect("reading the scrape body");
    assert!(
        scrape.contains(r#"vpay_webhook_deliveries_total{outcome="retry"} 3"#),
        "three refused attempts, three retry increments:\n{scrape}"
    );
    assert!(
        scrape.contains(r#"vpay_webhook_deliveries_total{outcome="succeeded"} 1"#),
        "the fourth attempt succeeded, exactly once:\n{scrape}"
    );
    metrics_task.abort();
}

/// A delivery that has already failed every rung is `exhausted`, and the job
/// is done rather than rescheduled.
///
/// `attempt = 7` is the state seven recorded failures leave behind: the
/// ladder has seven rungs (`delivery_delay(7) == None`), so the eighth
/// failure has nowhere left to go. The merchant is never told about this
/// event by vpay — the `alert = true` log line and this row are the whole of
/// what happens, which is why the row is asserted rather than assumed.
#[tokio::test]
async fn a_delivery_past_the_last_rung_is_exhausted_and_not_rescheduled() {
    let h = harness().await.expect("harness");
    let endpoints = h.flaky_registry();
    let (metrics_addr, metrics_task) = serve_metrics().await.expect("the metrics listener");

    let event = insert_event(&h.pool, MERCHANT_A, "pi_exhausted")
        .await
        .expect("an event");
    let fanout = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    handle_fan_out(&h.pool, &endpoints, &fanout)
        .await
        .expect("fan-out");
    let delivery_id = vpay_db::webhook_deliveries::for_event(&h.pool, &event.id)
        .await
        .expect("deliveries")
        .pop()
        .expect("one delivery")
        .id;

    assert_eq!(
        delivery_delay(7),
        None,
        "the ladder has exactly seven rungs"
    );
    force_attempt(&h.pool, delivery_id, 7)
        .await
        .expect("staging seven recorded failures");

    let job = claim_delivery_job(&h.pool, delivery_id)
        .await
        .expect("the delivery job");
    let outcome = handle_deliver(&h.pool, &delivery_client(), &endpoints, &job)
        .await
        .expect("the handler ran");
    assert!(
        matches!(outcome, Outcome::Done),
        "an exhausted delivery is finished, not retried: {outcome:?}"
    );

    let row = vpay_db::webhook_deliveries::get(&h.pool, delivery_id)
        .await
        .expect("the delivery row")
        .expect("it still exists");
    assert_eq!(row.state, "exhausted");
    assert_eq!(row.attempt, 8);
    assert_eq!(
        row.next_attempt_at, None,
        "an exhausted delivery must not claim a next attempt it will never make"
    );

    let scrape = reqwest::get(format!("http://{metrics_addr}/metrics"))
        .await
        .expect("scraping /metrics off the observability listener")
        .text()
        .await
        .expect("reading the scrape body");
    assert!(
        scrape.contains(r#"vpay_webhook_deliveries_total{outcome="exhausted"} 1"#),
        "the eighth failure exhausts the ladder and must count as exhausted, not retry:\n{scrape}"
    );
    metrics_task.abort();
}

// -------------------------------------------------------- GET /v1/events --

/// The merchant surface, over the real router: newest first, cursors, and
/// another merchant's events invisible in both directions.
///
/// Driven through the shipping Rust SDK for the list (`client.events()`,
/// which had no route to call until this step) and through a raw request for
/// the retrieve, which the SDK does not implement.
#[tokio::test]
async fn events_are_listed_newest_first_scoped_to_the_merchant() {
    ensure_crypto_provider_installed();
    let (_postgres, pool) = migrated_postgres().await.expect("postgres");

    let (server_pem, _) = generate_key();
    let (pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let served = serve(&pool, &server_pem, move |base_url| {
        events_config(
            base_url,
            jwks_a.clone(),
            jwks_b.clone(),
            &[vpay_api::SCOPE_PAYMENTS_READ],
        )
    })
    .await
    .expect("a server");

    // Three of A's, one of B's, in a known order. Named rather than
    // indexed: `newest` and `oldest` are what the assertions are about, and
    // a positional `mine[2]` would also be a `clippy::indexing_slicing`
    // denial (this workspace does not exempt tests from that one).
    let oldest = insert_event(&pool, MERCHANT_A, "pi_1")
        .await
        .expect("an event");
    let middle = insert_event(&pool, MERCHANT_A, "pi_2")
        .await
        .expect("an event");
    let newest = insert_event(&pool, MERCHANT_A, "pi_3")
        .await
        .expect("an event");
    let theirs = insert_event(&pool, MERCHANT_B, "pi_theirs")
        .await
        .expect("an event");

    let client = vpay_sdk::Client::builder(&served.base_url)
        .credentials(vpay_sdk::Credentials::rsa_pem(CLIENT_A, &pem_a).expect("the PEM parses"))
        .build()
        .expect("the SDK client builds");

    let page = client
        .events()
        .list(vpay_sdk::ListEventsParams::default())
        .await
        .expect("GET /v1/events answers");
    assert_eq!(page.object, "list");
    assert_eq!(page.url, "/v1/events");
    assert!(!page.has_more);
    let ids: Vec<&str> = page.data.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![newest.id.as_str(), middle.id.as_str(), oldest.id.as_str()],
        "newest first"
    );
    assert!(
        !ids.contains(&theirs.id.as_str()),
        "another merchant's event must not appear: {ids:?}"
    );

    // Cursors. `limit=1` then `starting_after` walks the same order.
    let first = client
        .events()
        .list(vpay_sdk::ListEventsParams {
            limit: Some(1),
            ..Default::default()
        })
        .await
        .expect("a one-row page");
    assert_eq!(
        first.data.first().map(|event| event.id.as_str()),
        Some(newest.id.as_str())
    );
    assert_eq!(first.data.len(), 1);
    assert!(first.has_more, "two more of this merchant's events exist");

    let second = client
        .events()
        .list(vpay_sdk::ListEventsParams {
            limit: Some(1),
            starting_after: Some(newest.id.clone()),
            ..Default::default()
        })
        .await
        .expect("the next page");
    assert_eq!(second.data.len(), 1);
    assert_eq!(
        second.data.first().map(|event| event.id.as_str()),
        Some(middle.id.as_str())
    );

    let back = client
        .events()
        .list(vpay_sdk::ListEventsParams {
            ending_before: Some(middle.id.clone()),
            ..Default::default()
        })
        .await
        .expect("the previous page");
    assert_eq!(
        back.data.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec![newest.id.as_str()],
        "paging backwards returns the same order"
    );

    // The envelope the deliverer signs is the envelope this serves.
    let delivered =
        String::from_utf8(event_bytes(&newest).expect("the worker's renderer accepts the row"))
            .expect("JSON is UTF-8");
    let served_body: Value = serde_json::to_value(page.data.first().expect("a first event"))
        .expect("the SDK's Event serialises");
    let delivered_value: Value = serde_json::from_str(&delivered).expect("valid JSON");
    assert_eq!(
        served_body.get("id"),
        delivered_value.get("id"),
        "the list and the webhook must describe the same event"
    );
    assert_eq!(served_body.get("data"), delivered_value.get("data"));

    // Retrieve, and a foreign id.
    let token = mint_read_token(&served.signing_key, CLIENT_A);
    let http = reqwest::Client::new();

    let response = http
        .get(format!("{}/v1/events/{}", served.base_url, oldest.id))
        .bearer_auth(&token)
        .send()
        .await
        .expect("the retrieve answers");
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("a JSON body");
    assert_eq!(
        body.get("id").and_then(Value::as_str),
        Some(oldest.id.as_str())
    );
    assert_eq!(body.get("object").and_then(Value::as_str), Some("event"));

    let response = http
        .get(format!("{}/v1/events/{}", served.base_url, theirs.id))
        .bearer_auth(&token)
        .send()
        .await
        .expect("the retrieve answers");
    assert_eq!(
        response.status(),
        404,
        "another merchant's event id must be indistinguishable from one that does not exist"
    );
    let body: Value = response.json().await.expect("a JSON body");
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some("resource_missing")
    );

    served.server.abort();
}

/// A credential registered for no scope cannot read events, and one
/// registered for `payments:read` can.
///
/// The pair is the assertion: a `403` for an unscoped client proves nothing
/// on its own — an endpoint that 403'd everybody would also pass.
#[tokio::test]
async fn reading_events_requires_a_scope() {
    ensure_crypto_provider_installed();
    let (_postgres, pool) = migrated_postgres().await.expect("postgres");

    let (server_pem, _) = generate_key();
    let (pem_a, jwks_a) = generate_key();
    let (_pem_b, jwks_b) = generate_key();

    let served = serve(&pool, &server_pem, move |base_url| {
        events_config(base_url, jwks_a.clone(), jwks_b.clone(), &[])
    })
    .await
    .expect("a server");

    insert_event(&pool, MERCHANT_A, "pi_scope")
        .await
        .expect("an event");

    let client = vpay_sdk::Client::builder(&served.base_url)
        .credentials(vpay_sdk::Credentials::rsa_pem(CLIENT_A, &pem_a).expect("the PEM parses"))
        .build()
        .expect("the SDK client builds");

    let error = client
        .events()
        .list(vpay_sdk::ListEventsParams::default())
        .await
        .expect_err("a client registered for no scope may not read events");
    match error {
        vpay_sdk::Error::Api { status, code, .. } => {
            assert_eq!(status, 403);
            assert_eq!(code.as_deref(), Some("forbidden"));
        }
        other => panic!("expected a 403, got {other:?}"),
    }

    // The same request with a read scope. `mint_read_token` signs
    // `payments:read` with the server's own key, which is what the OP would
    // have minted for a client registered for it — the registration above is
    // deliberately scopeless so the two answers differ only in the scope.
    let token = mint_read_token(&served.signing_key, CLIENT_A);
    let response = reqwest::Client::new()
        .get(format!("{}/v1/events", served.base_url))
        .bearer_auth(token)
        .send()
        .await
        .expect("the list answers");
    assert_eq!(response.status(), 200);

    served.server.abort();
}

// ------------------------------------------------------------- plumbing ---

/// The configuration both `/v1/events` tests boot: two merchants, no rails,
/// one currency.
///
/// No providers, deliberately: nothing here confirms anything, and a rail
/// would only add a WireMock container to a test about a read path.
fn events_config(base_url: &str, jwks_a: Value, jwks_b: Value, scopes: &[&str]) -> Config {
    Config {
        deployment: Deployment {
            name: "webhooks-test".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: Vec::new(),
        currencies: vec![CurrencyEntry {
            code: "EUR".to_owned(),
            exponent: 2,
        }],
        merchant_clients: vec![
            merchant_client_with(
                CLIENT_A,
                MERCHANT_A,
                jwks_a,
                scopes,
                vec![webhook_endpoint(
                    ENDPOINT_ID,
                    "http://receiver.internal/webhooks",
                    &[SECRET],
                )],
            ),
            merchant_client_with(CLIENT_B, MERCHANT_B, jwks_b, scopes, Vec::new()),
        ],
        dashboard_client: None,
    }
}

/// A `/v1` access token carrying `payments:read`, signed with the server's
/// own key.
///
/// The same shape the OP would mint — same issuer, same key, same `vpay:v1`
/// audience — so a request carrying it is indistinguishable to `/v1` from
/// one the SDK obtained. Minted directly rather than obtained because
/// `GET /v1/events/{id}` has no SDK method to reach it with, and because
/// `reading_events_requires_a_scope` needs a token whose scope differs from
/// the registration's on purpose.
fn mint_read_token(signing_key: &LoadedSigningKey, client_id: &str) -> String {
    signing_key
        .token_manager()
        .issue_client_token_with_extra(
            client_id,
            900,
            Some(vpay_api::SCOPE_PAYMENTS_READ.to_owned()),
            Some(vpay_config::MERCHANT_AUDIENCE.to_owned()),
            std::collections::HashMap::new(),
        )
        .expect("the server's own signer mints a merchant token")
}

// --------------------------------------------------------- the Node SDK ---

/// The repository root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// `node` on `PATH`, or an explanation of why the test is about to fail.
///
/// Never `Ok(skip)`. `VPAY_REQUIRE_NODE=1` only changes the *message*: CI
/// sets it so a runner that lost its Node install says so plainly, and a
/// developer without it gets told how to get one. Either way the test fails,
/// because a skipped parity test is a parity claim nobody checked.
fn node_verifier_available() -> Result<(), String> {
    let required = std::env::var("VPAY_REQUIRE_NODE").as_deref() == Ok("1");
    match Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => Ok(()),
        other => Err(if required {
            format!(
                "VPAY_REQUIRE_NODE=1 and `node` is not usable ({other:?}). This job is the \
                 evidence that the header vpay emits is accepted by @vpay/sdk; without node \
                 there is no such evidence."
            )
        } else {
            format!(
                "`node` is not on PATH ({other:?}). This test proves cross-language webhook \
                 signature parity and cannot be skipped: install Node (.nvmrc pins the \
                 version) and run `pnpm --filter @vpay/sdk build`."
            )
        }),
    }
}

/// Verifies `body`/`signature` with the built `@vpay/sdk`, in a subprocess.
///
/// Returns the parsed event on success, or the verifier's own stderr on
/// failure — which is what the negative case asserts against.
///
/// The body and the header go through **temporary files**, not through
/// argv: the body is JSON containing quotes and braces, and a shell-quoting
/// mistake would corrupt exactly the bytes the signature covers, turning a
/// parity failure into a quoting failure nobody could tell apart.
fn verify_with_node(body: &[u8], signature: &str, secret: &str) -> Result<Value, String> {
    node_verifier_available()?;

    let root = repo_root();
    let dist = root.join("sdks/nodejs/dist/webhooks.js");
    if !dist.is_file() {
        // Built on demand rather than assumed: CI builds it in the `rust`
        // job before nextest, and a developer running this suite for the
        // first time should not have to know that.
        let built = Command::new("pnpm")
            .args(["--filter", "@vpay/sdk", "build"])
            .current_dir(&root)
            .output()
            .map_err(|error| {
                format!(
                    "sdks/nodejs/dist is absent and `pnpm --filter @vpay/sdk build` could not \
                     run ({error}). CI builds it in the `rust` job before nextest; run it by \
                     hand, or `just build-sdk-node`."
                )
            })?;
        if !built.status.success() {
            return Err(format!(
                "`pnpm --filter @vpay/sdk build` failed:\n{}",
                String::from_utf8_lossy(&built.stderr)
            ));
        }
    }

    let dir = std::env::temp_dir().join(format!("vpay-webhook-parity-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let body_path = dir.join("body.json");
    let header_path = dir.join("signature.txt");
    let secret_path = dir.join("secret.txt");
    std::fs::write(&body_path, body).map_err(|e| e.to_string())?;
    std::fs::write(&header_path, signature).map_err(|e| e.to_string())?;
    std::fs::write(&secret_path, secret).map_err(|e| e.to_string())?;

    // `verifyWebhook` from the built package, over the exact bytes. `rawBody`
    // is read as a Buffer, never as a re-serialised object: a
    // parse-and-reserialise is the single most common way a merchant breaks
    // their own verification, and a test that did it would hide the case.
    //
    // A dynamic `import()` rather than `require`: `@vpay/sdk` is
    // `"type": "module"`, so its `dist/` is ESM and `require` of it throws
    // `ERR_REQUIRE_ESM` — which would fail this test for a module-format
    // reason and read exactly like a signature failure. `pathToFileURL`
    // because an ESM specifier is a URL, not a path.
    let script = format!(
        "const {{ readFileSync }} = require('node:fs');\
         const {{ pathToFileURL }} = require('node:url');\
         import(pathToFileURL({dist:?}).href).then(({{ verifyWebhook }}) => {{\
           const event = verifyWebhook({{\
             rawBody: readFileSync({body:?}),\
             signatureHeader: readFileSync({header:?}, 'utf8'),\
             secret: readFileSync({secret:?}, 'utf8'),\
           }});\
           process.stdout.write(JSON.stringify(event));\
         }}).catch((error) => {{\
           process.stderr.write(String(error && error.name) + ': ' + String(error && error.message));\
           process.exit(1);\
         }});",
        dist = dist.to_string_lossy(),
        body = body_path.to_string_lossy(),
        header = header_path.to_string_lossy(),
        secret = secret_path.to_string_lossy(),
    );

    let output = Command::new("node")
        .arg("-e")
        .arg(&script)
        .current_dir(&root)
        .output()
        .map_err(|error| format!("running node: {error}"));
    let _ = std::fs::remove_dir_all(&dir);
    let output = output?;

    if !output.status.success() {
        return Err(format!(
            "node exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("node printed something that is not an event: {error}"))
}

// ------------------------------------------------ fan-out isolation -------

/// One merchant's un-insertable event must not stop every other merchant's.
///
/// # Why this is the failure it is
///
/// The drain is a **singleton**: one `fanout:events` job carries every
/// merchant's backlog, ordered by `events.seq` across all of them. Before the
/// fix `handle_fan_out` propagated the first event's error out of the loop, so
/// one merchant's bad row was a total webhook outage for everyone behind it —
/// and permanently, because a failed event stays `pending` and heads the very
/// next page. Nothing recovers from that on its own.
///
/// # How exactly one event is made to fail, without a test double
///
/// `EndpointRegistry::from_pairs` is shipping code and takes the strings it is
/// given. Merchant A's endpoint is registered with a 65-character id — one
/// past `webhook_deliveries`' `endpoint_id_length` CHECK (migration 0022) — so
/// `create_in_tx` is refused by Postgres for A's event and by nothing for B's.
/// That is a real `DbError` from a real constraint, not an injected fault.
///
/// `vpay_config::validate_webhook_endpoints` now refuses that id at boot, so
/// a *deployment* cannot reach this state through configuration any more. The
/// registry can still be built with it, and more to the point the failure
/// being isolated is any per-event failure at all — a transient Postgres
/// error, a serialisation failure, a constraint added later. The 65-character
/// id is simply the one that is deterministic.
///
/// # Revert proof
///
/// Put the `?` back on `fan_out_one` in `handle_fan_out` and this test fails
/// twice over: the pass returns `Err` instead of `Ok`, and merchant B's event
/// is still `pending` with nothing delivered.
///
/// One pass is also the first rung of `fanout_attempts` (migration 0024), so
/// this test additionally pins what a *single* failure does: count one, stay
/// `pending`, and **not** alert.
/// `a_permanently_unfannable_event_is_abandoned_after_five_passes_and_alerts_once`
/// is the other end of the same ladder.
#[tokio::test]
async fn one_merchants_unfannable_event_does_not_block_another_merchants() {
    let logs = CapturedLog::default();
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .finish(),
    );

    let h = harness().await.expect("harness");

    // One past `endpoint_id_length`. Registered through the same constructor
    // the worker binary uses.
    let doomed_endpoint_id = "p".repeat(65);
    let endpoints = EndpointRegistry::from_pairs([
        (
            MERCHANT_A.to_owned(),
            vec![Endpoint {
                id: doomed_endpoint_id.clone(),
                url: format!("{}/webhooks", h.receiver_url),
                secrets: vec![SECRET.to_owned()],
            }],
        ),
        (
            MERCHANT_B.to_owned(),
            vec![Endpoint {
                id: ENDPOINT_ID.to_owned(),
                url: format!("{}/webhooks", h.receiver_url),
                secrets: vec![SECRET.to_owned()],
            }],
        ),
    ]);

    // A's event first, so it is first in `pending_page`'s `seq` order. That
    // ordering is what makes this a test of isolation rather than of luck.
    let blocked = insert_event(&h.pool, MERCHANT_A, "pi_blocked")
        .await
        .expect("merchant A's event");
    let behind_it = insert_event(&h.pool, MERCHANT_B, "pi_behind_it")
        .await
        .expect("merchant B's event");
    assert!(
        blocked.seq < behind_it.seq,
        "the failing event must come first in the page, or this proves nothing"
    );

    let job = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    let outcome = handle_fan_out(&h.pool, &endpoints, &job)
        .await
        .expect("one event's failure must not fail the pass");
    assert!(
        matches!(outcome, Outcome::RescheduleAfter(_)),
        "a page with failures still reschedules: {outcome:?}"
    );

    // The failing event is untouched and will be retried on the next pass.
    assert_eq!(
        fanout_state(&h.pool, &blocked.id).await.expect("state"),
        "pending",
        "the failing event's transaction rolled back whole, so it is still owed a fan-out"
    );
    assert!(
        vpay_db::webhook_deliveries::for_event(&h.pool, &blocked.id)
            .await
            .expect("deliveries")
            .is_empty(),
        "a rolled-back fan-out must leave no half-written delivery"
    );

    // The one behind it went through.
    assert_eq!(
        fanout_state(&h.pool, &behind_it.id).await.expect("state"),
        "done",
        "merchant B's event was blocked by merchant A's"
    );
    let delivery = vpay_db::webhook_deliveries::for_event(&h.pool, &behind_it.id)
        .await
        .expect("deliveries")
        .pop()
        .expect("merchant B's event produced a delivery");
    assert_eq!(
        jobs_of_kind(&h.pool, "deliver_webhook")
            .await
            .expect("jobs"),
        vec![webhook_dedupe_key(delivery.id)]
    );

    // And it really is deliverable: the bytes reach the receiver.
    let job = claim_delivery_job(&h.pool, delivery.id)
        .await
        .expect("the delivery job");
    let outcome = handle_deliver(&h.pool, &delivery_client(), &endpoints, &job)
        .await
        .expect("the delivery handler ran");
    assert!(matches!(outcome, Outcome::Done), "{outcome:?}");
    let recorded = journal(&h.receiver_url)
        .await
        .expect("the receiver's journal")
        .pop()
        .expect("the receiver recorded merchant B's POST");
    assert_eq!(
        recorded.header("vpay-event-id"),
        Some(behind_it.id.as_str())
    );

    // The failure was counted against the event (migration 0024). One pass,
    // one attempt — and the event is still `pending`, because one failure is
    // a long way from the ceiling.
    assert_eq!(
        fanout_attempts(&h.pool, &blocked.id)
            .await
            .expect("fanout_attempts"),
        1,
        "one failed pass must count exactly one attempt against the event"
    );

    // The log line. Nothing else will ever say this happened — the drain
    // returned `Ok` and the job rescheduled normally — so the line itself is
    // part of the contract. It is a `WARN` and carries **no** `alert`: this
    // event is retried in five seconds, and alerting on every pass is what
    // turned one poisoned event into an unbounded page storm.
    let captured = logs.text();
    assert!(
        !captured.contains("alert=true"),
        "a fan-out failure with attempts left must not alert; captured:\n{captured}"
    );
    assert!(
        captured.contains(&blocked.id),
        "the warning must name the event that failed; captured:\n{captured}"
    );
    assert!(
        captured.contains(MERCHANT_A),
        "the warning must name the merchant whose event failed; captured:\n{captured}"
    );
    assert!(
        !captured.contains(SECRET),
        "no log line may carry a signing secret; captured:\n{captured}"
    );
}

/// A permanently unfannable event is abandoned after
/// `FANOUT_MAX_ATTEMPTS` passes, leaves the backlog, and alerts **once**.
///
/// # The failure this closes
///
/// Isolating a per-event failure (the test above) is not enough on its own.
/// `events::pending_page` orders by `seq`, so a `pending` event that can never
/// be fanned out is at the head of *every* subsequent page: it re-raises its
/// alert every five seconds and holds one of `FAN_OUT_PAGE`'s hundred slots
/// forever. A hundred of them stop the drain for every merchant, and an alert
/// that fires forever is one an operator mutes.
///
/// # What is asserted, and why each part is needed
///
/// Five passes over the *same* failing event — the same 65-character endpoint
/// id, the same real `endpoint_id_length` CHECK. Then:
///
/// * `fanout_attempts = 5` and `fanout_state = 'failed'`: the counter reached
///   the ceiling and the state moved;
/// * the event is **gone from `pending_page`**, which is what stops it
///   holding a slot and being retried;
/// * exactly **one** line in the whole five-pass capture carries
///   `alert=true`, and it is the transition. Four warnings preceded it. This
///   is the assertion the whole change exists for: without the counter the
///   same capture holds five alerts, and with a hundred such events it holds
///   five hundred, per pass, forever.
///
/// # Revert proof
///
/// Delete the increment (the `fanout_attempts + 1` in
/// `vpay_db::events::record_fanout_failure`) and this test fails on the very
/// first assertion — the count stays 0, the state stays `pending`, the event
/// is still in `pending_page` and no alert is ever raised. Removing only the
/// `CASE` that flips the state fails it on `fanout_state`.
#[tokio::test]
async fn a_permanently_unfannable_event_is_abandoned_after_five_passes_and_alerts_once() {
    let logs = CapturedLog::default();
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .finish(),
    );

    let h = harness().await.expect("harness");

    // One past `endpoint_id_length` (migration 0022) — a real constraint
    // refusing a real insert, for the reason the test above gives.
    let endpoints = EndpointRegistry::from_pairs([(
        MERCHANT_A.to_owned(),
        vec![Endpoint {
            id: "p".repeat(65),
            url: format!("{}/webhooks", h.receiver_url),
            secrets: vec![SECRET.to_owned()],
        }],
    )]);

    let doomed = insert_event(&h.pool, MERCHANT_A, "pi_doomed")
        .await
        .expect("an event");

    let job = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    for pass in 1..=vpay_worker::FANOUT_MAX_ATTEMPTS {
        handle_fan_out(&h.pool, &endpoints, &job)
            .await
            .unwrap_or_else(|error| panic!("pass {pass} must not fail the job: {error}"));
    }

    assert_eq!(
        fanout_attempts(&h.pool, &doomed.id)
            .await
            .expect("fanout_attempts"),
        vpay_worker::FANOUT_MAX_ATTEMPTS,
        "five failed passes must count five attempts"
    );
    assert_eq!(
        fanout_state(&h.pool, &doomed.id).await.expect("state"),
        "failed",
        "the fifth failure must abandon the event"
    );
    assert!(
        vpay_db::webhook_deliveries::for_event(&h.pool, &doomed.id)
            .await
            .expect("deliveries")
            .is_empty(),
        "every pass rolled back whole, so there is no half-written delivery"
    );

    // Out of the backlog — the property that stops it holding a page slot.
    let backlog = vpay_db::events::pending_page(&h.pool, 100)
        .await
        .expect("the backlog");
    assert!(
        !backlog.iter().any(|event| event.id == doomed.id),
        "an abandoned event must leave pending_page, or it heads every page forever"
    );

    // And a sixth pass is silent: it does not see the event at all.
    let before = logs.text().len();
    handle_fan_out(&h.pool, &endpoints, &job)
        .await
        .expect("a pass over an empty backlog");
    assert_eq!(
        logs.text().len(),
        before,
        "a pass after the abandonment must say nothing about the event"
    );

    let captured = logs.text();
    assert_eq!(
        captured.matches("alert=true").count(),
        1,
        "exactly one alert for the whole life of a poisoned event; captured:\n{captured}"
    );
    assert!(
        captured.contains("has been abandoned"),
        "the alert must say the event was abandoned; captured:\n{captured}"
    );
    assert!(
        captured.contains(&doomed.id),
        "the alert must name the event; captured:\n{captured}"
    );
    // The four that preceded it were warnings, and they are the early signal
    // an operator sees before the alert.
    assert_eq!(
        captured
            .matches("it stays pending and the rest of the page continues")
            .count(),
        4,
        "four warnings then one alert; captured:\n{captured}"
    );
    assert!(
        !captured.contains(SECRET),
        "no log line may carry a signing secret; captured:\n{captured}"
    );
}

/// `events.fanout_attempts` for one event — migration 0024's counter.
///
/// Read with SQL rather than through `EventRow`, which deliberately does not
/// carry the column: no reader of an event branches on it, and the writer
/// gets the new value back from its own `RETURNING`.
async fn fanout_attempts(pool: &PgPool, event_id: &str) -> anyhow::Result<i32> {
    let attempts: i32 = sqlx::query_scalar("SELECT fanout_attempts FROM events WHERE id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .context("reading events.fanout_attempts")?;
    Ok(attempts)
}

/// The worker's `tracing` output, captured for one test.
///
/// `set_default` rather than `set_global_default`: the guard is thread-local
/// and `#[tokio::test]` runs the body on a current-thread runtime, so the
/// handler being awaited emits on this thread — and no other test in this
/// binary is affected however it is run.
#[derive(Clone, Default)]
struct CapturedLog(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl CapturedLog {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("the capture buffer is not poisoned"))
            .into_owned()
    }
}

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("the capture buffer is not poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// --------------------------------------------- the delivery backstop ------

/// A delivery whose `deliver_webhook` job was deleted is found by
/// `scan_deliveries` and delivered — and one whose job still exists is left
/// alone.
///
/// # What this covers that the queue does not
///
/// The fan-out enqueues a delivery's job in the transaction that creates the
/// row, so the two cannot disagree at birth. They can afterwards: an operator
/// **deleting** a stuck row, or a `jobs` truncation during an incident.
/// Before migration 0023 such a delivery was owed an attempt that nothing
/// would ever make, `pending_due` existed with no caller, and the merchant was
/// never told about the payment behind it.
///
/// A **dead-lettered** job is deliberately not in that list, and never was
/// recovered by this scan: parking keeps the `dedupe_key`, so the re-enqueue
/// is a no-op. `a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan`
/// is the case that pins that, and this one's `DELETE` is what the scan
/// actually covers.
///
/// The second delivery is not decoration — it is what stops this passing for
/// the wrong reason. A scan that simply re-enqueued everything `pending`
/// would satisfy the first half and would also drag every rung of the retry
/// ladder back to now.
///
/// # Revert proof
///
/// Two of them. Delete the `JobKind::ScanDeliveries` seed from
/// `run_loop::seed_singletons` and `worker_recovery`'s
/// `the_housekeeping_jobs_are_seeded_once_and_reschedule_themselves` fails on
/// three rows where four are expected. Drop the never-attempted arm from
/// `pending_due` and this test fails: the stranded delivery is never
/// re-enqueued and never reaches the receiver.
#[tokio::test]
async fn the_backstop_re_enqueues_a_delivery_whose_job_vanished() {
    let h = harness().await.expect("harness");
    let lease = vpay_worker::RecoveryPolicy::default().lease;

    let stranded_event = insert_event(&h.pool, MERCHANT_A, "pi_stranded")
        .await
        .expect("an event");
    let attended_event = insert_event(&h.pool, MERCHANT_A, "pi_attended")
        .await
        .expect("an event");

    let fanout = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    handle_fan_out(&h.pool, &h.endpoints, &fanout)
        .await
        .expect("fan-out");

    let stranded = vpay_db::webhook_deliveries::for_event(&h.pool, &stranded_event.id)
        .await
        .expect("deliveries")
        .pop()
        .expect("one delivery");
    let attended = vpay_db::webhook_deliveries::for_event(&h.pool, &attended_event.id)
        .await
        .expect("deliveries")
        .pop()
        .expect("one delivery");

    // Operator error, or a truncated `jobs` table: the delivery survives and
    // its job does not. Deleted directly, because that is exactly what leaves
    // the state this scan exists for — and deliberately **not** dead-lettered,
    // which leaves a different state the scan cannot fix (see the next test).
    sqlx::query("DELETE FROM jobs WHERE dedupe_key = $1")
        .bind(webhook_dedupe_key(stranded.id))
        .execute(&h.pool)
        .await
        .expect("deleting the delivery's job");
    // And it has been sitting there longer than a claim could legitimately be
    // outstanding, which is what tells the scan the job is gone rather than
    // merely unclaimed.
    sqlx::query(
        "UPDATE webhook_deliveries SET created_at = now() - interval '1 hour' WHERE id = $1",
    )
    .bind(stranded.id)
    .execute(&h.pool)
    .await
    .expect("ageing the stranded delivery");

    let scan = claim_scan_deliveries_job(&h.pool)
        .await
        .expect("the backstop job");
    let outcome = vpay_worker::handle_scan_deliveries(&h.pool, lease, &scan)
        .await
        .expect("the backstop ran");
    assert!(
        matches!(outcome, Outcome::RescheduleAfter(_)),
        "the backstop reschedules itself rather than finishing: {outcome:?}"
    );

    let jobs = jobs_of_kind(&h.pool, "deliver_webhook")
        .await
        .expect("jobs");
    assert!(
        jobs.contains(&webhook_dedupe_key(stranded.id)),
        "the stranded delivery must get its job back: {jobs:?}"
    );
    assert_eq!(
        jobs.iter()
            .filter(|key| **key == webhook_dedupe_key(attended.id))
            .count(),
        1,
        "the attended delivery must keep exactly one job — every insert is ON CONFLICT DO \
         NOTHING, so the backstop never produces a second: {jobs:?}"
    );

    // The recovered delivery really is delivered.
    let job = claim_delivery_job(&h.pool, stranded.id)
        .await
        .expect("the re-enqueued delivery job");
    let outcome = handle_deliver(&h.pool, &delivery_client(), &h.endpoints, &job)
        .await
        .expect("the delivery handler ran");
    assert!(matches!(outcome, Outcome::Done), "{outcome:?}");

    let row = vpay_db::webhook_deliveries::get(&h.pool, stranded.id)
        .await
        .expect("the delivery row")
        .expect("it still exists");
    assert_eq!(row.state, "succeeded");
    let delivered_ids: Vec<Option<String>> = journal(&h.receiver_url)
        .await
        .expect("journal")
        .iter()
        .map(|post| post.header("vpay-event-id").map(str::to_owned))
        .collect();
    assert_eq!(
        delivered_ids,
        vec![Some(stranded_event.id.clone())],
        "exactly the recovered event reached the receiver"
    );
}

/// A delivery whose job was **dead-lettered** is *not* resurrected by the
/// scan — and the scan says so.
///
/// # Why this is the behaviour and not a bug
///
/// `vpay_db::jobs::dead_letter` parks a job at `run_at = 'infinity'` and keeps
/// its `dedupe_key`. That is what the backstop's
/// `INSERT … ON CONFLICT (dedupe_key) DO NOTHING` collides with, so the pass
/// is a no-op for exactly these rows. Deliberate: a `deliver_webhook` job is
/// parked for a `Poisoned` reason — an event that will not render, a body
/// whose digest no longer matches what was signed — and none of them is fixed
/// by trying again. A scan that un-parked it would re-run the same failure
/// every ten minutes forever, which is the hot loop `vpay_db::jobs`' own
/// module comment refuses.
///
/// The documentation used to claim the opposite ("a job an operator deleted,
/// one that was dead-lettered…"), in `webhooks.rs`, `jobs.rs`, migration
/// `0023`, `docs/status.md`, `docs/flows/webhooks.md` and the test above's own
/// comment. This test is what stops that claim coming back: it fails the
/// moment the scan starts resurrecting a parked job.
///
/// # What is asserted
///
/// After a pass over an aged, `pending` delivery whose job is parked:
///
/// * the job is **still** at `'infinity'` — nothing un-parked it;
/// * the delivery is **still** `pending`, and nothing was delivered;
/// * one `WARN` names it, so the state has an observer other than an operator
///   running `SELECT * FROM jobs WHERE run_at = 'infinity'` by hand. That line
///   is the whole mitigation, so it is asserted rather than assumed.
#[tokio::test]
async fn a_dead_lettered_delivery_job_is_not_resurrected_by_the_scan() {
    let logs = CapturedLog::default();
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .finish(),
    );

    let h = harness().await.expect("harness");
    let lease = vpay_worker::RecoveryPolicy::default().lease;

    let event = insert_event(&h.pool, MERCHANT_A, "pi_parked")
        .await
        .expect("an event");
    let fanout = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    handle_fan_out(&h.pool, &h.endpoints, &fanout)
        .await
        .expect("fan-out");
    let delivery = vpay_db::webhook_deliveries::for_event(&h.pool, &event.id)
        .await
        .expect("deliveries")
        .pop()
        .expect("one delivery");

    // Parked through the shipping write, not by a hand-rolled UPDATE: what is
    // being tested is the interaction between `dead_letter` and the scan's
    // enqueue, so a test that wrote `run_at = 'infinity'` itself could keep
    // passing after `dead_letter` changed.
    let job = claim_delivery_job(&h.pool, delivery.id)
        .await
        .expect("the delivery job");
    assert!(
        vpay_db::jobs::dead_letter(&h.pool, job.id, WORKER, "event will not render")
            .await
            .expect("parking the delivery job"),
        "the worker holding the lease must be able to park it"
    );
    // Older than any legitimate claim, so `pending_due` returns it — the scan
    // has to *see* the row for "it does not recover it" to mean anything.
    sqlx::query(
        "UPDATE webhook_deliveries SET created_at = now() - interval '1 hour' WHERE id = $1",
    )
    .bind(delivery.id)
    .execute(&h.pool)
    .await
    .expect("ageing the delivery");
    assert!(
        vpay_db::webhook_deliveries::pending_due(&h.pool, lease, 100)
            .await
            .expect("pending_due")
            .iter()
            .any(|row| row.id == delivery.id),
        "the scan must be able to see the row, or this test proves nothing"
    );

    let scan = claim_scan_deliveries_job(&h.pool)
        .await
        .expect("the backstop job");
    let outcome = vpay_worker::handle_scan_deliveries(&h.pool, lease, &scan)
        .await
        .expect("the backstop ran");
    assert!(
        matches!(outcome, Outcome::RescheduleAfter(_)),
        "the backstop reschedules itself: {outcome:?}"
    );

    // The row is untouched: still parked, still owed an attempt nothing will
    // make.
    let parked: bool = sqlx::query_scalar(
        "SELECT run_at = 'infinity'::TIMESTAMPTZ FROM jobs WHERE dedupe_key = $1",
    )
    .bind(webhook_dedupe_key(delivery.id))
    .fetch_one(&h.pool)
    .await
    .expect("the parked job is still there");
    assert!(
        parked,
        "the backstop must not un-park a dead-lettered delivery job"
    );
    assert_eq!(
        vpay_db::webhook_deliveries::get(&h.pool, delivery.id)
            .await
            .expect("the delivery row")
            .expect("it still exists")
            .state,
        "pending",
        "the delivery stays pending; nothing about it was resolved"
    );
    assert!(
        journal(&h.receiver_url).await.expect("journal").is_empty(),
        "nothing may reach the receiver — the job that would have sent it is parked"
    );

    // The one thing the scan *does* do about it.
    let captured = logs.text();
    assert!(
        captured.contains("dead-lettered (parked) delivery job"),
        "the scan must name a pending delivery whose job is parked; captured:\n{captured}"
    );
    assert!(
        captured.contains(&webhook_dedupe_key(delivery.id)),
        "the warning must name the delivery; captured:\n{captured}"
    );
}

/// Seeds and claims the `scan_deliveries` singleton, exactly as
/// `run_loop::seed_singletons` seeds it.
async fn claim_scan_deliveries_job(pool: &PgPool) -> anyhow::Result<JobRow> {
    let mut tx = pool.begin().await?;
    vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        "scan_deliveries",
        vpay_worker::jobs::SCAN_DELIVERIES_DEDUPE_KEY,
        &json!({}),
        OffsetDateTime::now_utc(),
    )
    .await?;
    tx.commit().await?;

    let row: JobRow = sqlx::query_as(
        "UPDATE jobs SET locked_at = now(), locked_by = $1, attempts = attempts + 1 \
         WHERE dedupe_key = $2 \
         RETURNING id, kind, dedupe_key, payload, run_at, attempts, locked_by, last_error",
    )
    .bind(WORKER)
    .bind(vpay_worker::jobs::SCAN_DELIVERIES_DEDUPE_KEY)
    .fetch_one(pool)
    .await
    .context("the scan_deliveries job is claimable")?;
    Ok(row)
}

// ------------------------------------------------ through the real loop ---

/// The shipping `run_loop` takes an event from the backlog all the way to a
/// POST at a merchant's receiver, with nothing driving it by hand.
///
/// # Why this test exists at all
///
/// Every other case in this file calls `handle_fan_out` / `handle_deliver`
/// directly, which proves the handlers and nothing about the process. A
/// `run_loop` that stopped seeding `fanout:events`, or a `dispatch` arm that
/// stopped routing `deliver_webhook`, would leave all of them green and every
/// merchant unnotified — which is precisely the shape of failure this
/// repository exists to refuse. So this one starts the same function
/// `vpay-worker-bin`'s `main` calls, with a real `EndpointRegistry` pointed at
/// the WireMock receiver, and asserts on the receiver's own journal.
///
/// # What it deliberately does not include
///
/// The `confirm → settle` prefix. The event here is written with
/// `vpay_db::events::insert_in_tx` — the exact call `vpay_db::settlement`
/// makes inside the settlement transaction — rather than produced by a real
/// rail settlement, which would need this file to stand up a payment API, an
/// MTN WireMock and an adapter to prove a seam `worker_e2e.rs` already proves
/// (`wait_for_fanout`, on a real confirm) and that `just demo`'s step 7 proves
/// end to end through the shipping binaries. What is unproven anywhere else,
/// and is proven here, is everything from the backlog onwards.
///
/// # Bounded, and it fails rather than hanging
///
/// The wait is a deadline over the receiver's journal, and running out of it
/// is an assertion failure that names the state it saw. The loop is then shut
/// down through its own signal, so the drain path this test rides is the one
/// a `SIGTERM` takes.
#[tokio::test(flavor = "multi_thread")]
async fn the_real_run_loop_delivers_a_backlog_event_to_the_receiver() {
    /// A ceiling, not an expectation: the drain is seeded at `run_at = now()`
    /// and the delivery job is enqueued in the transaction that creates the
    /// row, so a healthy loop finishes this in well under a second. The
    /// margin is for container scheduling.
    const DELIVERY_TIMEOUT: Duration = Duration::from_secs(30);

    let h = harness().await.expect("harness");
    let event = insert_event(&h.pool, MERCHANT_A, "pi_through_the_loop")
        .await
        .expect("an event settlement would have written");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let pool = h.pool.clone();
    let endpoints = std::sync::Arc::new(h.endpoints.clone());
    let http = delivery_client();
    let loop_handle = tokio::spawn(async move {
        vpay_worker::run_loop(
            &pool,
            // No rails: nothing in this test confirms a charge, and a
            // `poll_charge` job never exists. The webhook path reads neither.
            std::sync::Arc::new(vpay_worker::Adapters::new()),
            std::sync::Arc::new(vpay_worker::RailConfigs::new()),
            vpay_worker::RecoveryPolicy::default(),
            endpoints,
            http,
            1,
            Duration::from_secs(5),
            "webhooks-run-loop".to_owned(),
            async move {
                // A dropped sender is also a shutdown: if this test panics
                // before signalling, the loop still stops rather than the
                // task outliving the run.
                let _ = shutdown_rx.await;
            },
        )
        .await
    });

    let deadline = std::time::Instant::now() + DELIVERY_TIMEOUT;
    let delivered = loop {
        let posts = journal(&h.receiver_url).await.expect("the journal reads");
        if let Some(post) = posts.into_iter().next_back() {
            break post;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the shipping run_loop did not deliver event {} within {DELIVERY_TIMEOUT:?}. The \
             usual causes are that `seed_singletons` no longer seeds `fanout:events`, or that \
             `handlers::dispatch` no longer routes `deliver_webhook`. The event's fan-out \
             state was {:?}.",
            event.id,
            fanout_state(&h.pool, &event.id).await,
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    let _ = shutdown_tx.send(());
    let report = loop_handle.await.expect("the loop task did not panic");
    assert!(
        report.claimed > 0,
        "the loop claimed nothing, so whatever delivered was not it: {report:?}"
    );

    assert_eq!(
        delivered.header("vpay-event-id"),
        Some(event.id.as_str()),
        "the receiver got the event the backlog held"
    );
    let signature = delivered
        .header("vpay-signature")
        .expect("the loop's delivery carried a Vpay-Signature");
    let verified = vpay_sdk::webhooks::verify(&delivered.body, signature, SECRET, TOLERANCE)
        .expect("the bytes the shipping loop sent verify with the shipping SDK");
    assert_eq!(verified.id, event.id);

    // And the durable record agrees with the receiver.
    let delivery = vpay_db::webhook_deliveries::for_event(&h.pool, &event.id)
        .await
        .expect("deliveries")
        .pop()
        .expect("the loop's fan-out created a delivery");
    assert_eq!(delivery.state, "succeeded");
    assert_eq!(delivery.status_code, Some(200));
    assert_eq!(
        fanout_state(&h.pool, &event.id).await.expect("state"),
        "done"
    );
}

// -------------------------------------- the branch that sends nothing -----

/// An endpoint configuration no longer describes is a recorded failed
/// attempt — with **no** `payload_sha256`, because nothing was rendered or
/// signed.
///
/// # Why the digest is the assertion
///
/// `webhook_deliveries.payload_sha256` means "the digest of the exact bytes
/// signed by the first attempt that **rendered and signed a body**" —
/// migration 0024's corrected `COMMENT`; 0022's said "the first attempt",
/// which this very branch makes untrue — and the delivery handler compares
/// every later attempt's re-rendered body against it, treating a difference
/// as `Poisoned`. Stamping it on an attempt that produced no signed body
/// makes the column say something untrue about a delivery that has still
/// never been signed, and silently converts the first *real* attempt's
/// mismatch check from "did the renderer change?" into a comparison against
/// a body that never existed.
///
/// Note what this is **not**: a transport failure passes `Some`, because
/// those bytes were rendered and signed before the socket was ever opened.
/// Only "nothing to sign with" stores nothing.
///
/// # Revert proof
///
/// Pass `Some(&sha)` instead of `None` at the "endpoint is not configured"
/// branch in `handle_deliver` and this test fails on the digest.
#[tokio::test]
async fn a_delivery_with_no_configured_endpoint_records_a_failure_and_no_digest() {
    let h = harness().await.expect("harness");
    let event = insert_event(&h.pool, MERCHANT_A, "pi_unconfigured")
        .await
        .expect("an event");

    // Fanned out while the endpoint is configured, so the delivery row is a
    // real one; then the registry stops describing it, which is what a
    // rollout that briefly serves an older configuration looks like.
    let fanout = claim_fanout_job(&h.pool).await.expect("the fan-out job");
    handle_fan_out(&h.pool, &h.endpoints, &fanout)
        .await
        .expect("fan-out");
    let delivery = vpay_db::webhook_deliveries::for_event(&h.pool, &event.id)
        .await
        .expect("deliveries")
        .pop()
        .expect("one delivery");

    let forgotten = EndpointRegistry::from_pairs(std::iter::empty::<(String, Vec<Endpoint>)>());
    let job = claim_delivery_job(&h.pool, delivery.id)
        .await
        .expect("the delivery job");
    let outcome = handle_deliver(&h.pool, &delivery_client(), &forgotten, &job)
        .await
        .expect("the delivery handler ran");
    assert!(
        matches!(outcome, Outcome::RescheduleAfter(delay) if delay == delivery_delay(0)
            .expect("the first rung")),
        "an unconfigured endpoint is an ordinary failed attempt on the ladder, not an \
         exhaustion and not an error: {outcome:?}"
    );

    let row = vpay_db::webhook_deliveries::get(&h.pool, delivery.id)
        .await
        .expect("the delivery row")
        .expect("it still exists");
    assert_eq!(row.attempt, 1, "the attempt is recorded");
    assert_eq!(row.state, "pending", "and another one is still owed");
    assert_eq!(
        row.status_code, None,
        "no request was made, so there is no status to have heard"
    );
    assert_eq!(
        row.payload_sha256, None,
        "nothing was sent, so there is no digest of sent bytes to record"
    );

    assert!(
        journal(&h.receiver_url)
            .await
            .expect("the journal reads")
            .is_empty(),
        "an unsigned webhook must never leave the process — a receiver may not act on one"
    );
}
