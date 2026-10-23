//! `docs/flows/crash-safety.md`'s recovery table, executed.
//!
//! Every case here drives `vpay_worker::run_once` — the real loop body, the
//! one `vpay-worker-bin` calls — against a real Postgres container and a real
//! WireMock rail container, and asserts what was *committed*.
//!
//! # How a crash is staged, and why it is not a `SIGKILL`
//!
//! It is not possible to kill a handler mid-transaction from a test and have
//! the result be reproducible; crash-safety.md's own Status section
//! disclaims a `SIGKILL` test for exactly that reason, and this suite does
//! not pretend to be one. What it does instead is write **the state each kill
//! point leaves**, which is the thing the recovery table actually branches
//! on:
//!
//! | kill point | rows a crash leaves | evidence `recovery_step` reads |
//! |---|---|---|
//! | 1 — before the POST | charge `submitting`, no `provider_requests` row | `SubmitAttempt::Never` |
//! | 2 — POST issued, answer lost | + a row with `status_code IS NULL` | `SubmitAttempt::Unanswered` |
//! | 3 — rail answered, write lost | + `status_code` recorded | `SubmitAttempt::Answered` |
//!
//! Migration 0016's `response_is_paired` CHECK is what makes row 2 and row 3
//! genuinely different states on disk rather than a convention, so writing
//! them by hand is writing the same thing a crash would.
//!
//! # The assertion every case shares
//!
//! Exactly **one** distinct `provider_reference_id` across every
//! `provider_requests` row for the charge, however the recovery table
//! resolved it. "A fresh reference on retry is how you double-charge a
//! customer" (crash-safety.md); a second reference would mean vpay asked the
//! rail for a second payment.
//!
//! # No test doubles
//!
//! The rails are WireMock hosts in configuration (ADR-0006), mounted from the
//! same `backends/tests/conformance/wiremock/{mtn,orange}` trees the
//! conformance suite and `compose.yml` use. The adapters, the pool, the
//! handlers and the loop are the shipping ones. The only thing this file
//! constructs that a deployment would not is `RecoveryPolicy`, and it
//! constructs the *same* struct `main` does — a policy with a 50 ms window
//! runs the identical code path a deployment runs at 60 s, which is the whole
//! reason that type has no `#[cfg(test)]` seam.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use serde_json::json;
use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use uuid::Uuid;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, ProviderHost};
use vpay_db::{Jobs, Repositories, TxOutcome, UnitOfWork as _};
use vpay_worker::{Adapters, Disposition, RailConfigs, RecoveryPolicy, Settled};

mod support;

use support::{
    RECOVERABLE_CRASH_AGE, age_the_crash, attempted_references, confirmed_intent, crashed_charge,
    make_every_job_runnable, migrated_postgres, rail_configs, reconcile_from_config,
};

const MERCHANT: &str = "acme-cameroon-tenant";
const PUSH_RAIL: &str = "mtn_momo";
const REDIRECT_RAIL: &str = "orange_money";
const CURRENCY: &str = "XAF";
const AMOUNT: i64 = 5000;

/// A documentation MSISDN nothing stubs specifically, so a submit from here
/// falls through to `requesttopay.json`'s catch-all 202.
const MSISDN: &str = "237670000000";

/// The reference `requesttopay-status.json` answers `404 RESOURCE_NOT_FOUND`
/// to — `ChargeStatus::NotFound`, the input to the `not_found` ladder.
const NOT_FOUND_REF: Uuid = Uuid::from_u128(0x0404);

/// The reference `requesttopay-scenario.json` walks `PENDING` → `SUCCESSFUL`
/// at priority 1. Used where a case needs a *pending* answer it can rely on
/// rather than the catch-all `SUCCESSFUL`.
const PENDING_THEN_SUCCESS_REF: Uuid = Uuid::from_u128(0x0ce0);

/// The reference `requesttopay-status.json` answers `503` to on every status
/// query — `ProviderError::Unavailable`, which is `Severity::Warn` and rides
/// the retry ladder quietly. The input to the "a failing rail must still be
/// escalated at the horizon" case.
const UNAVAILABLE_REF: Uuid = Uuid::from_u128(0x05aa);

/// The reference `requesttopay-status.json` answers `FAILED /
/// NOT_ENOUGH_FUNDS` to — a terminal *decline*, mapped to
/// `FailureCode::InsufficientFunds`. The input to the "a terminal answer
/// settles a charge that is already `unresolved`" case.
const DECLINED_REF: Uuid = Uuid::from_u128(0x0f01);

/// `vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT`, the `0`
/// sentinel migration 0020 introduced for "the rail answered and the port does
/// not carry its status line". Transcribed rather than imported so this file
/// says out loud which value it is staging kill point 3 with.
const ANSWERED_SENTINEL: i32 = 0;

/// The reference `requesttopay-status.json` answers a 300 KB body to, past
/// `vpay_provider::http::MAX_RAIL_BODY_BYTES`: the rail **answered**, and the
/// adapter refuses what it answered as `ProviderError::Malformed`. The input to
/// "a status read the rail answered and vpay would not act on". The body is a
/// valid `SUCCESSFUL` on purpose, so the case cannot pass by the adapter
/// reading it.
const HUGE_ANSWER_REF: Uuid = Uuid::from_u128(0x0b16);

/// The `X-Reference-Id` `requesttopay.json` answers a `307` to on a **submit**.
/// The adapter does not follow it and answers `Malformed`, "never a decline:
/// the charge's fate is unknown" (`vpay-adapter-mtn-momo`'s `submit_outcome`).
const REDIRECTED_SUBMIT_REF: Uuid = Uuid::from_u128(0x0302);

/// The `X-Reference-Id` `requesttopay.json` answers `400 PAYER_NOT_FOUND` to on
/// a **submit** — a rail decision, `ProviderError::Rejected`.
const DECLINED_SUBMIT_REF: Uuid = Uuid::from_u128(0x0400);

/// The rail of the vaam-apps/vpay#313 cases: the only one whose adapter
/// raises `ProviderError::NeedsReconciliation`, and the only two-step rail
/// (ADR-0032).
const NOTCHPAY: &str = "notchpay";

/// The reference `wiremock/notchpay`'s `payments-retrieve.json` answers, by
/// `trx.test_…0f10`, with Notch Pay's sandbox "insufficient funds" read:
/// `complete` for 5000 XAF with `amounts.total` 4980 — a short collection,
/// `NeedsReconciliation` since 2026-10-23.
const SHORT_COLLECTION_REF: Uuid = Uuid::from_u128(0x0f10);

/// The reference `payments-initialize.json` answers `409` "Reference already
/// existing" to. What a `prepare` makes of it turns on whether a submit was
/// ever recorded for the charge (ADR-0032): none, and the charge fails
/// cleanly; one, and it is `NeedsReconciliation`.
const NOTCHPAY_DUPLICATE_REF: Uuid = Uuid::from_u128(0x00dd);

/// The payer `requesttopay.json` declines on **any** reference
/// (`400 PAYER_NOT_FOUND`, selected by `$.payer.partyId`). Paired with
/// [`NOT_FOUND_REF`] it is a rail that declines every submit and has no record
/// of every read — which is what a real decline looks like from the poll's side.
const DECLINED_PAYER_MSISDN: &str = "237670000400";

// ------------------------------------------------------------------ harness

/// Postgres, both rail stubs, and everything `run_once` needs.
///
/// The two maps are behind `Arc` because the lease-reaping cases hand them to
/// `vpay_worker::run_loop`, which owns them for the life of the loop — the
/// same shape `vpay-worker-bin`'s `main` builds. `run_once` borrows through
/// the same `Arc`, so both entry points are driven from one set of adapters
/// rather than from two that could be configured differently.
struct Harness {
    _postgres: ContainerAsync<PostgresImage>,
    /// Every rail stub this harness started: MTN and Orange for [`harness`],
    /// Notch Pay alone for [`notchpay_harness`]. Held only to keep them up.
    _rails: Vec<ContainerAsync<GenericImage>>,
    /// The Notch Pay stub's origin, for the one case that adds a mapping
    /// through WireMock's admin API. `None` on the MTN and Orange harness.
    notchpay_origin: Option<String>,
    repositories: Arc<dyn Repositories>,
    /// The plain `sqlx` pool, for the fixtures that read or force schema
    /// state no repository method owns.
    pool: PgPool,
    adapters: Arc<Adapters>,
    rails: Arc<RailConfigs>,
}

impl Harness {
    /// Claims and runs exactly one job, and asserts there was one to run.
    ///
    /// `run_once` is the loop's own body — the same function
    /// `vpay_worker::run_loop` calls N times per task — so a case that drives
    /// it is driving the shipping claim/settle protocol, `SKIP LOCKED` and
    /// `locked_by` guard included, one step at a time instead of racing a
    /// background loop.
    async fn step(&self, policy: &RecoveryPolicy) -> anyhow::Result<Settled> {
        let endpoints = support::no_webhook_endpoints();
        let egress = support::default_egress_policy();
        vpay_worker::run_once(
            self.repositories.as_ref(),
            &self.adapters,
            &self.rails,
            policy,
            &vpay_worker::WebhookContext {
                endpoints: &endpoints,
                egress,
            },
            &vpay_worker::CallbackPlan::none(),
            "worker-recovery-suite",
        )
        .await
        .context("running one job")?
        .context("the queue had no runnable job; the fixture did not enqueue one")
    }
}

fn mappings_dir(rail: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/wiremock")
        .join(rail)
}

/// Both rails on XAF, `livemode: false` — the same shape
/// `config/application.yml` has, so a configuration this suite accepts is one
/// that would load from a file.
fn config_with(mtn_url: &str, orange_url: &str) -> Config {
    Config {
        deployment: Deployment {
            name: "worker-recovery".to_owned(),
            livemode: false,
            public_base_url: "http://127.0.0.1:8080".to_owned(),
            surfaces: None,
        },
        providers: vec![
            ProviderHost {
                code: PUSH_RAIL.to_owned(),
                enabled: true,
                host: HostEntry {
                    url: mtn_url.to_owned(),
                    label: "mtn-wiremock".to_owned(),
                },
                settings: BTreeMap::from([
                    ("target_environment".to_owned(), "sandbox".to_owned()),
                    (
                        "api_user".to_owned(),
                        "11111111-2222-3333-4444-555555555555".to_owned(),
                    ),
                ]),
                callback_url: None,
                currency: CURRENCY.to_owned(),
                credentials: BTreeMap::from([
                    (
                        "subscription_key".to_owned(),
                        "stub-subscription-key".to_owned(),
                    ),
                    ("api_key".to_owned(), "stub-api-key".to_owned()),
                ]),
                display_name: None,
                callbacks: None,
            },
            ProviderHost {
                code: REDIRECT_RAIL.to_owned(),
                enabled: true,
                host: HostEntry {
                    url: orange_url.to_owned(),
                    label: "orange-wiremock".to_owned(),
                },
                settings: BTreeMap::from([
                    ("env".to_owned(), "dev".to_owned()),
                    ("lang".to_owned(), "en".to_owned()),
                ]),
                callback_url: None,
                currency: CURRENCY.to_owned(),
                credentials: BTreeMap::from([
                    ("merchant_key".to_owned(), "stub-merchant-key".to_owned()),
                    ("client_id".to_owned(), "stub-client-id".to_owned()),
                    ("client_secret".to_owned(), "stub-client-secret".to_owned()),
                ]),
                display_name: None,
                callbacks: None,
            },
        ],
        currencies: vec![CurrencyEntry {
            code: CURRENCY.to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![],
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: None,
        staff_auth: vpay_config::StaffAuth::default(),
        routing: Default::default(),
    }
}

async fn harness() -> anyhow::Result<Harness> {
    let (postgres, repositories, pool) = migrated_postgres().await?;

    let mtn = vpay_testkit::containers::start_wiremock(&mappings_dir("mtn"))
        .await
        .context("the MTN stub container starts")?;
    let orange = vpay_testkit::containers::start_wiremock(&mappings_dir("orange"))
        .await
        .context("the Orange stub container starts")?;

    let mtn_url = format!(
        "http://127.0.0.1:{}",
        mtn.get_host_port_ipv4(8080)
            .await
            .context("the MTN stub's mapped port")?
    );
    // The `/orange-money-webpay/{env}` prefix is part of the configured base
    // URL, exactly as `config/application.yml` writes it.
    let orange_url = format!(
        "http://127.0.0.1:{}/orange-money-webpay/dev",
        orange
            .get_host_port_ipv4(8080)
            .await
            .context("the Orange stub's mapped port")?
    );

    let config = config_with(&mtn_url, &orange_url);
    reconcile_from_config(repositories.as_ref(), &config).await?;

    Ok(Harness {
        _postgres: postgres,
        _rails: vec![mtn, orange],
        notchpay_origin: None,
        repositories,
        pool,
        adapters: Arc::new(support::adapters_by_code()),
        rails: Arc::new(rail_configs(&config)),
    })
}

/// Postgres and the Notch Pay stub alone, with the real `notchpay` adapter,
/// measured, as `vpay-server` links it.
///
/// A harness of its own rather than a third container in [`harness`]: only
/// the vaam-apps/vpay#313 cases need Notch Pay, and every other case in this
/// file would pay for a container it never calls. The stub is the
/// conformance suite's `wiremock/notchpay` tree, and the keys are its
/// `test_`-named stand-ins (`adapter_conformance.rs`'s Notch Pay row).
async fn notchpay_harness() -> anyhow::Result<Harness> {
    let (postgres, repositories, pool) = migrated_postgres().await?;
    let notchpay = vpay_testkit::containers::start_wiremock(&mappings_dir(NOTCHPAY))
        .await
        .context("the Notch Pay stub container starts")?;
    let origin = format!(
        "http://127.0.0.1:{}",
        notchpay
            .get_host_port_ipv4(8080)
            .await
            .context("the Notch Pay stub's mapped port")?
    );

    let mut config = config_with("http://127.0.0.1:1", "http://127.0.0.1:1");
    config.providers = vec![ProviderHost {
        code: NOTCHPAY.to_owned(),
        enabled: true,
        host: HostEntry {
            url: origin.clone(),
            label: "notchpay-wiremock".to_owned(),
        },
        settings: BTreeMap::from([("channel".to_owned(), "cm.mobile".to_owned())]),
        callback_url: None,
        currency: CURRENCY.to_owned(),
        credentials: BTreeMap::from([
            (
                "public_key".to_owned(),
                "stub-public-key-test_0001".to_owned(),
            ),
            (
                "private_key".to_owned(),
                "stub-private-key-test_0002".to_owned(),
            ),
        ]),
        display_name: None,
        callbacks: None,
    }];

    let http = vpay_provider::http::client_with_timeouts(
        vpay_provider::DEFAULT_CONNECT_TIMEOUT,
        vpay_provider::DEFAULT_REQUEST_TIMEOUT,
    )
    .context("the vendored-roots client builds")?;
    let adapters = vpay_api::v1::boot::adapters_by_code(vec![Box::new(
        vpay_adapter_notchpay::Adapter::new(http),
    )
        as Box<dyn vpay_provider::ProviderAdapter>]);
    support::reconcile_from_config_with(repositories.as_ref(), &config, &adapters).await?;

    Ok(Harness {
        _postgres: postgres,
        _rails: vec![notchpay],
        notchpay_origin: Some(origin),
        repositories,
        pool,
        rails: Arc::new(rail_configs(&config)),
        adapters: Arc::new(adapters),
    })
}

// ------------------------------------------------------------------ reading

#[derive(Debug, sqlx::FromRow)]
struct StoredCharge {
    state: String,
    provider_reference_id: Uuid,
    provider_txn_id: Option<String>,
    failure_code: Option<String>,
}

async fn charge(pool: &PgPool, id: &str) -> anyhow::Result<StoredCharge> {
    sqlx::query_as::<_, StoredCharge>(
        "SELECT state::TEXT AS state, provider_reference_id, provider_txn_id, \
         failure_code::TEXT AS failure_code FROM charges WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .context("reading the charge")
}

async fn intent_status(pool: &PgPool, id: &str) -> anyhow::Result<(String, Option<String>)> {
    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT status::TEXT, last_payment_error_code::TEXT FROM payment_intents WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .context("reading the intent")?;
    Ok(row)
}

/// `payment_intents.amount_received`, the column a settlement fills in.
///
/// Read separately from [`intent_status`] rather than widening it: only the
/// success case has anything to say about it, and a tuple that grew a third
/// member would make every other case carry a value it ignores.
async fn amount_received(pool: &PgPool, id: &str) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT amount_received FROM payment_intents WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .context("reading amount_received")
}

/// Every job row, read straight from Postgres rather than through
/// `vpay_db::jobs`: what these cases assert is what is *committed*, and a read
/// through the same repository as the write would prove only that the two
/// agree with each other.
#[derive(Debug, sqlx::FromRow)]
struct StoredJob {
    kind: String,
    dedupe_key: String,
    attempts: i32,
    locked_by: Option<String>,
    last_error: Option<String>,
    /// Seconds from now until this job is claimable. Negative means it is
    /// claimable already. `NULL` for a parked job (`run_at = 'infinity'`),
    /// which has no finite offset — the encoding is exactly what distinguishes
    /// "rescheduled far out" from "dead-lettered".
    seconds_until_runnable: Option<f64>,
}

/// A deferred poll came back **once, when the charge is old enough** — not on
/// the ladder's first rung.
///
/// `RecoveryAction::Wait` carries `not_found_window - charge_age` and the
/// handler adds a one-second margin, so for a charge staged milliseconds ago
/// the delay lands just above the window. The band is wide enough for a
/// loaded machine's few hundred milliseconds and still excludes the number
/// this is a regression test for: `poll_delay(0)`, ten seconds.
///
/// Why it matters that it is not ten: every reschedule is re-claimed,
/// `vpay_db::Jobs::claim` increments `jobs.attempts`, and `poll_delay` is
/// indexed by that count — so waiting the window out in ten-second rungs
/// spent six attempts doing nothing and left a genuinely crashed charge
/// starting its real recovery at `poll_delay(6)`, two minutes a rung.
fn assert_waited_out_the_rest_of_the_window(deferred: &Settled, policy: &RecoveryPolicy) {
    let Disposition::Rescheduled(delay) = deferred.disposition else {
        panic!(
            "a charge too young to recover must be rescheduled, not {:?}; error: {:?}",
            deferred.disposition, deferred.error
        );
    };
    assert!(
        delay > vpay_worker::poll_delay(0),
        "the deferral is back on the ladder's first rung ({delay:?}): six of those cross \
         the window, each one spending an attempt"
    );
    assert!(
        delay > policy.not_found_window - Duration::from_secs(2)
            && delay <= policy.not_found_window + Duration::from_secs(1),
        "a charge staged a moment ago must wait about the whole window; got {delay:?} \
         against a window of {:?}",
        policy.not_found_window
    );
}

async fn jobs(pool: &PgPool) -> anyhow::Result<Vec<StoredJob>> {
    sqlx::query_as::<_, StoredJob>(
        "SELECT kind, dedupe_key, attempts, locked_by, last_error, \
                CASE WHEN run_at = 'infinity' THEN NULL \
                     ELSE EXTRACT(EPOCH FROM (run_at - now()))::DOUBLE PRECISION END \
                AS seconds_until_runnable \
         FROM jobs ORDER BY dedupe_key",
    )
    .fetch_all(pool)
    .await
    .context("reading the jobs table")
}

/// How many times the rail was asked about this charge's *status*.
///
/// Distinct from [`attempted_references`], which counts every attempt of any
/// kind: what the horizon cases assert is that the rail **was** asked, exactly
/// once, and a submit attempt staged by the fixture would hide that. A charge
/// past the horizon is polled hourly, not abandoned — a count of zero there is
/// the bug, not the invariant (`docs/flows/reconciler.md`).
async fn status_queries(pool: &PgPool, charge_id: &str) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM provider_requests \
         WHERE charge_id = $1 AND operation = 'query_status'",
    )
    .bind(charge_id)
    .fetch_one(pool)
    .await
    .context("counting the status queries")
}

/// One `provider_requests` row, as the table holds it.
///
/// `answered` is `responded_at IS NOT NULL`, read separately from
/// `status_code` so a row that carried one without the other would show here
/// — `response_is_paired` makes that unwritable, and an assertion that read
/// only one column would not notice if it ever were.
#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct StoredAttempt {
    status_code: Option<i32>,
    answered: bool,
    error_kind: Option<String>,
}

/// Every attempt of `operation` against a charge, oldest first.
async fn attempts_of(
    pool: &PgPool,
    charge_id: &str,
    operation: &str,
) -> anyhow::Result<Vec<StoredAttempt>> {
    sqlx::query_as::<_, StoredAttempt>(
        "SELECT status_code, (responded_at IS NOT NULL) AS answered, error_kind \
         FROM provider_requests WHERE charge_id = $1 AND operation = $2 ORDER BY id",
    )
    .bind(charge_id)
    .bind(operation)
    .fetch_all(pool)
    .await
    .context("reading the attempt rows")
}

/// The payment intent a charge belongs to.
async fn intent_of(pool: &PgPool, charge_id: &str) -> anyhow::Result<String> {
    sqlx::query_scalar::<_, String>("SELECT payment_intent_id FROM charges WHERE id = $1")
        .bind(charge_id)
        .fetch_one(pool)
        .await
        .context("reading the charge's intent")
}

/// The `type` of every event an intent has, in order.
async fn event_types(pool: &PgPool, object_id: &str) -> anyhow::Result<Vec<String>> {
    Ok(events(pool, object_id)
        .await?
        .into_iter()
        .map(|(event_type, _)| event_type)
        .collect())
}

async fn events(pool: &PgPool, object_id: &str) -> anyhow::Result<Vec<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT type::TEXT, fanout_state::TEXT FROM events WHERE object_id = $1 ORDER BY seq",
    )
    .bind(object_id)
    .fetch_all(pool)
    .await
    .context("reading the events table")
}

/// Stages kill point 2 or 3: an attempt row for a submit that either got no
/// answer (`status_code = None`) or got one (`Some`).
async fn record_submit_attempt(
    repositories: &dyn Repositories,
    charge_id: &str,
    rail: &str,
    reference: Uuid,
    status_code: Option<i32>,
) -> anyhow::Result<()> {
    record_attempt(
        repositories,
        charge_id,
        rail,
        "submit",
        reference,
        status_code,
    )
    .await
}

/// [`record_submit_attempt`] for any operation: a two-step rail's case
/// stages its `prepare` row with it (ADR-0032).
async fn record_attempt(
    repositories: &dyn Repositories,
    charge_id: &str,
    rail: &str,
    operation: &str,
    reference: Uuid,
    status_code: Option<i32>,
) -> anyhow::Result<()> {
    let attempt_id = repositories
        .insert_pending(charge_id, rail, operation, reference, 1)
        .await
        .with_context(|| format!("writing the {operation} attempt row"))?;
    if let Some(code) = status_code {
        repositories
            .record_response(attempt_id, Some(code), None)
            .await
            .with_context(|| format!("recording the {operation} answer"))?;
    }
    Ok(())
}

/// Ages a charge so the 24-hour horizon has genuinely passed for it, under
/// the **documented** `unresolved_after` rather than a policy of zero.
///
/// The other horizon cases set `unresolved_after: Duration::ZERO`, which puts
/// the horizon behind a charge created a moment ago and is enough when every
/// poll in the case is on the same side of it. It is not enough for a case
/// that has to cross the horizon *between* polls — the ladder needs several
/// polls on the near side to build a `NotFound` streak, and only the last one
/// may be past it. Moving `charges.created_at`, which is the column
/// `past_the_horizon` measures from, is how a charge gets to be a day old
/// without the suite waiting a day, and it leaves `RecoveryPolicy` exactly as
/// a deployment has it.
async fn age_past_the_horizon(pool: &PgPool, charge_id: &str) -> anyhow::Result<()> {
    let aged =
        sqlx::query("UPDATE charges SET created_at = now() - INTERVAL '25 hours' WHERE id = $1")
            .bind(charge_id)
            .execute(pool)
            .await
            .context("ageing the charge past the horizon")?
            .rows_affected();
    anyhow::ensure!(aged == 1, "the charge was not there to age");
    Ok(())
}

/// `charges.updated_at`, the column every write in `vpay-db` touches.
///
/// Read for one reason only: an escalation that has already happened must not
/// write again. There is no transition log in this schema (nothing in
/// `backends/migrations` records charge state changes as rows), so
/// `updated_at` is what records "something changed", and an unchanged value
/// across a second hourly poll is the evidence that the re-escalation was
/// idempotent.
async fn charge_updated_at(pool: &PgPool, id: &str) -> anyhow::Result<time::OffsetDateTime> {
    sqlx::query_scalar::<_, time::OffsetDateTime>("SELECT updated_at FROM charges WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .context("reading updated_at")
}

/// Writes the state an escalation leaves: the charge `unresolved`.
///
/// The same write `handlers::escalate_to_unresolved` makes, and
/// `a_charge_past_the_horizon_is_unresolved_polled_hourly_and_alerted_never_parked`
/// is what proves the worker makes it. It is staged here rather than reached
/// through the loop because reaching it needs a non-terminal answer and the
/// case under test needs a terminal one, and a WireMock reference answers the
/// same way every time.
async fn escalated_charge(pool: &PgPool, charge_id: &str) -> anyhow::Result<()> {
    let moved = sqlx::query(
        "UPDATE charges SET state = 'unresolved', updated_at = now() \
         WHERE id = $1 AND state = 'submitting'",
    )
    .bind(charge_id)
    .execute(pool)
    .await
    .context("escalating the charge")?
    .rows_affected();
    anyhow::ensure!(moved == 1, "the charge was not there to escalate");
    Ok(())
}

/// Points a charge at a currency the database accepts and this build does not
/// know, which is `JobError::Poisoned` and nothing else.
///
/// `currencies.code` is a plain `TEXT` primary key with a shape CHECK
/// (`^[A-Z]{3}$`, migration 0001), not a Postgres enum, so a row the schema
/// accepts and `vpay_core::Currency::from_code` rejects is writable — that
/// mismatch *is* the poisoning, and it is reached from `handlers::charge_ref`
/// inside `query_status`, i.e. exactly where the horizon's error arm sees it.
/// No test double is involved: the row is wrong, in the way a bad migration
/// or a downgraded binary would leave it wrong.
async fn write_a_currency_this_build_cannot_parse(
    pool: &PgPool,
    charge_id: &str,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO currencies (code, exponent) VALUES ('USD', 2) ON CONFLICT DO NOTHING")
        .execute(pool)
        .await
        .context("adding a currency the build does not know")?;
    let moved = sqlx::query("UPDATE charges SET currency_code = 'USD' WHERE id = $1")
        .bind(charge_id)
        .execute(pool)
        .await
        .context("pointing the charge at it")?
        .rows_affected();
    anyhow::ensure!(moved == 1, "the charge was not there to poison");
    Ok(())
}

/// The shared invariant. Called by every case, whatever it otherwise asserts.
async fn assert_one_reference(pool: &PgPool, charge_id: &str, expected: Uuid) {
    let references = attempted_references(pool, charge_id)
        .await
        .expect("reading the attempted references");
    assert_eq!(
        references,
        vec![expected],
        "every attempt against a charge must carry the charge's one reference; a second \
         reference is a second payment (docs/flows/crash-safety.md)"
    );
    let stored = charge(pool, charge_id).await.expect("reading the charge");
    assert_eq!(
        stored.provider_reference_id, expected,
        "the charge's own reference was rewritten"
    );
}

// -------------------------------------------------------------------- cases

/// Kill point 1: the charge committed and the process died before the POST.
///
/// `provider_requests` is empty, so the rail cannot have received anything —
/// `SubmitAttempt::Never`. The recovery table's answer is to submit, under the
/// **same** reference, and it must be reached without asking the rail about a
/// charge it was never told about.
///
/// Two steps, because the resolution is two jobs: the poll job enqueues a
/// `resubmit_charge` and reschedules itself; the resubmit is what talks to the
/// rail.
#[tokio::test]
async fn a_charge_whose_submit_never_left_is_resubmitted_under_the_same_reference()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;

    // Nothing was ever sent. This is the fact the whole branch turns on.
    assert!(
        attempted_references(&h.pool, &charge_id).await?.is_empty(),
        "kill point 1 must be staged with no attempt row at all"
    );

    let first = h.step(&policy).await?;
    assert_eq!(first.kind, "poll_charge");
    assert!(
        matches!(first.disposition, Disposition::Rescheduled(_)),
        "the poll job schedules the resubmit and stays on the ladder, it does not finish; \
         got {:?}",
        first.disposition
    );

    let queued = jobs(&h.pool).await?;
    assert!(
        queued.iter().any(|job| job.kind == "resubmit_charge"
            && job.dedupe_key == format!("resubmit:{charge_id}")),
        "no resubmit was enqueued for a charge whose submit never left; jobs: {queued:?}"
    );
    // The rail has still not been asked anything: recovery concluded from the
    // absence of an attempt row alone.
    assert!(attempted_references(&h.pool, &charge_id).await?.is_empty());

    make_every_job_runnable(&h.pool).await?;
    let second = h.step(&policy).await?;
    assert_eq!(second.kind, "resubmit_charge");
    assert_eq!(
        second.disposition,
        Disposition::Finished,
        "a resubmit that the rail accepted is done; error: {:?}",
        second.error
    );

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(
        stored.state, "submitted",
        "the rail accepted the resubmit, so the charge must have left `submitting`"
    );
    assert_one_reference(&h.pool, &charge_id, reference).await;
    Ok(())
}

/// Kill point 2: the POST went out and the answer was lost.
///
/// `status_code IS NULL` is the encoding for "no response was received", and
/// it is the one row of the table where the honest answer is *ask the rail*
/// rather than assume either way. The rail here answers `SUCCESSFUL`, so the
/// charge settles — a payment that was already made and that vpay had no
/// record of.
#[tokio::test]
async fn a_submit_whose_answer_was_lost_is_resolved_by_asking_the_rail() -> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        reference,
        None,
    )
    .await?;

    let settled = h.step(&policy).await?;
    assert_eq!(settled.kind, "poll_charge");
    assert_eq!(
        settled.disposition,
        Disposition::Finished,
        "a charge the rail reports as paid is settled and its job deleted; error: {:?}",
        settled.error
    );

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(stored.state, "succeeded");
    assert_eq!(
        intent_status(&h.pool, &intent).await?.0,
        "succeeded",
        "the intent must move with the charge, in the same transaction"
    );
    assert_one_reference(&h.pool, &charge_id, reference).await;
    Ok(())
}

/// Kill point 3: the rail answered the submit and our own write was lost.
///
/// The attempt row carries a status, so the charge is further along than
/// `charges.state` says. Recovery catches the bookkeeping up
/// (`submitting → submitted`) and then polls as normal; it must **not**
/// resubmit, because the rail already has this charge.
#[tokio::test]
async fn an_answered_submit_advances_the_bookkeeping_rather_than_submitting_again()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        reference,
        Some(ANSWERED_SENTINEL),
    )
    .await?;

    let settled = h.step(&policy).await?;
    assert_eq!(settled.disposition, Disposition::Finished);

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(stored.state, "succeeded");
    assert_eq!(
        stored.provider_txn_id.as_deref(),
        Some("1234567890"),
        "the rail's own transaction identifier must be recorded (migration 0021), and it \
         must be the one this stub returned"
    );

    let queued = jobs(&h.pool).await?;
    assert!(
        !queued.iter().any(|job| job.kind == "resubmit_charge"),
        "a charge the rail had already answered was submitted a second time; jobs: {queued:?}"
    );
    assert_one_reference(&h.pool, &charge_id, reference).await;
    Ok(())
}

/// The confirm's own window: a `submitting` charge younger than
/// `not_found_window` is not recovered, and the same charge is once it is
/// older.
///
/// `submitting` is not only the state a crash leaves. It is also the ordinary
/// state of a **confirm that is still running** — `vpay-api` commits the
/// charge and this very poll job in one transaction, calls the rail, and only
/// then compare-and-swaps `submitting → submitted`. (Since 2026-09-16 that
/// job is committed at `now() + POLL_AFTER_CONFIRM_GRACE` and pulled forward
/// by the compare-and-swap, so a live confirm is no longer the ordinary way
/// this branch is reached. It is still the branch a crashed one reaches, and
/// the fixture below stages that directly, as it always did.)
/// The evidence staged here is kill point 3's, byte for byte
/// (`an_answered_submit_advances_the_bookkeeping_rather_than_submitting_again`
/// is the same fixture aged past the window), because that is precisely the
/// point: nothing on disk distinguishes a crashed confirm from a live one, so
/// the only thing that can is how long the row has been sitting there.
///
/// Found by Step 8's demo, four runs in six on a loaded machine: the worker
/// advanced the charge, the confirm's compare-and-swap then matched no row and
/// the merchant was answered `500 write_matched_no_row` — while a
/// `payment_intent.succeeded` webhook was delivered for the payment it had
/// just been told failed (`docs/plans/step8-notes/lane-a.md` §3).
///
/// The second half is what stops the guard from being a way to never recover
/// anything: the identical charge, ninety seconds old, is advanced and settled.
#[tokio::test]
async fn a_young_push_charge_is_not_advanced_until_it_is_older_than_the_window()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let reference = Uuid::new_v4();
    // Deliberately **not** aged: this is a charge whose confirm may still be
    // inside `submit`, which is what the fixture is staging.
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        reference,
        Some(ANSWERED_SENTINEL),
    )
    .await?;

    let deferred = h.step(&policy).await?;
    assert_eq!(deferred.kind, "poll_charge");
    assert_waited_out_the_rest_of_the_window(&deferred, &policy);
    assert_eq!(deferred.error, None, "waiting is not a failure");

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(
        stored.state, "submitting",
        "the worker advanced a charge whose confirm may still be running; the confirm's \
         own compare-and-swap would now match no row"
    );
    assert_eq!(
        intent_status(&h.pool, &intent).await?.0,
        "requires_payment_method",
        "the intent moved under a confirm that has not answered yet"
    );
    assert_eq!(
        status_queries(&h.pool, &charge_id).await?,
        0,
        "the rail was asked about a submission that may be in flight at that moment"
    );
    assert!(
        events(&h.pool, &intent).await?.is_empty(),
        "a payment nobody has settled produced an event"
    );
    let queued = jobs(&h.pool).await?;
    assert!(
        !queued.iter().any(|job| job.kind == "resubmit_charge"),
        "a resubmit was enqueued for a charge that is still being confirmed; jobs: {queued:?}"
    );

    // What was *committed*, not what the handler returned: one wait, priced
    // at one attempt. `run_at` is Postgres' own `now() + delay`, and
    // `attempts` was incremented by the claim that ran this poll — a second
    // increment here would mean the wait had been split into rungs again.
    let poll = queued
        .iter()
        .find(|job| job.kind == "poll_charge")
        .expect("the poll job the confirm enqueued is still queued");
    assert_eq!(
        poll.attempts, 1,
        "waiting for the window must cost exactly the one claim that decided to wait; \
         jobs: {queued:?}"
    );
    let seconds = poll
        .seconds_until_runnable
        .expect("a waiting poll job is scheduled, not parked");
    assert!(
        seconds > 55.0 && seconds <= 61.0,
        "the committed `run_at` must be about the rest of the window away, not the \
         ladder's ten-second first rung; got {seconds}s"
    );

    // The same charge, a minute and a half old: now it really is a crash, and
    // the recovery table applies exactly as it always did.
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    make_every_job_runnable(&h.pool).await?;

    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::Finished,
        "the aged charge was not recovered at all; error: {:?}",
        settled.error
    );
    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(
        stored.state, "succeeded",
        "the guard delays recovery by the window, it does not cancel it"
    );
    assert_one_reference(&h.pool, &charge_id, reference).await;
    Ok(())
}

/// The same window on a redirect rail, where the branch that must not fire is
/// `RecoveryAction::FailDeadOrder`.
///
/// The worse half of the same race. `FailDeadOrder` is correct for a charge
/// whose submit response was genuinely lost — the payer was never handed a URL
/// — and catastrophic for one whose confirm is about to hand over exactly that
/// URL: Step 8's demo watched a live order be killed as
/// `provider_unavailable`, with `failure_raw` saying the payer "was never
/// handed a redirect URL" while the confirm held it
/// (`docs/plans/step8-notes/lane-a.md` §3).
///
/// The evidence is an **answered** submit, which on a redirect rail changes
/// nothing (the flow shape decides first) and here says the thing the fixture
/// is about: the rail had answered, so the response the recovery reasons about
/// was not lost at all.
#[tokio::test]
async fn a_young_redirect_charge_is_not_failed_as_a_dead_order_until_it_is_older()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        REDIRECT_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        REDIRECT_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        None,
    )
    .await?;
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        REDIRECT_RAIL,
        reference,
        Some(ANSWERED_SENTINEL),
    )
    .await?;

    let deferred = h.step(&policy).await?;
    assert_waited_out_the_rest_of_the_window(&deferred, &policy);

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(
        stored.state, "submitting",
        "a live order was killed as a dead one while its confirm was still running"
    );
    assert_eq!(stored.failure_code, None);
    let (status, error_code) = intent_status(&h.pool, &intent).await?;
    assert_eq!(status, "requires_payment_method");
    assert_eq!(
        error_code, None,
        "the merchant was told the rail was unavailable while it was answering"
    );
    assert!(
        events(&h.pool, &intent).await?.is_empty(),
        "a `payment_intent.payment_failed` event was written for a payment nobody failed"
    );

    // Aged: the response really was lost, and the order really is dead.
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    make_every_job_runnable(&h.pool).await?;

    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::Finished,
        "the aged redirect charge was not resolved; error: {:?}",
        settled.error
    );
    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(
        stored.state, "failed",
        "past the window `FailDeadOrder` is still the answer for a redirect charge"
    );
    assert_eq!(stored.failure_code.as_deref(), Some("provider_unavailable"));
    assert_eq!(
        intent_status(&h.pool, &intent).await?.1.as_deref(),
        Some("provider_unavailable")
    );
    Ok(())
}

/// The race itself, run: the real loop against the confirm's own
/// compare-and-swap, and the confirm wins.
///
/// The two cases above pin the decision one claimed job at a time. This one
/// stages the ordering that produced the defect — the worker claims and runs
/// the poll job **first**, and the confirm's `mark_submitted` lands after it —
/// against `vpay_worker::run_loop`, the loop `vpay-worker-bin` runs, with the
/// job at `run_at = now()` exactly as `insert_charge` writes it.
///
/// A redirect rail with no attempt row, because that is the shortest path to a
/// write: `SubmitAttempt::Never` on `ProviderFlow::Redirect` was
/// `FailDeadOrder` unconditionally, so before the age guard this test's
/// compare-and-swap matched no row and `vpay-api` answered the merchant
/// `500 write_matched_no_row` — which is exactly what
/// `docs/plans/step8-notes/lane-a.md` §3 recorded from the demo.
///
/// **What is staged and what is real.** The write performed here is
/// `TxRepositories::mark_submitted`, the compare-and-swap
/// `vpay_api::v1::payment_intents::persist_submitted` makes, with the
/// arguments a redirect confirm makes it with; the rest — the claim, the
/// recovery decision, the reschedule — is the shipping loop. What is *not*
/// here is the HTTP confirm handler around it, which this suite has no router
/// for; `confirm_rails.rs` owns that half.
#[tokio::test]
async fn a_confirms_compare_and_swap_wins_against_the_worker_that_claimed_its_poll_job()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    park_the_housekeeping_jobs(h.repositories.as_ref(), &h.pool).await?;
    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        REDIRECT_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let reference = Uuid::new_v4();
    // As `insert_charge` leaves it: `submitting`, its poll job runnable now,
    // and the confirm still inside the rail call.
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        REDIRECT_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        None,
    )
    .await?;

    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let repositories = Arc::clone(&h.repositories);
    let adapters = Arc::clone(&h.adapters);
    let rails = Arc::clone(&h.rails);
    let worker = tokio::spawn(async move {
        vpay_worker::run_loop(
            repositories,
            adapters,
            rails,
            policy,
            support::no_webhook_endpoints(),
            support::default_egress_policy(),
            std::sync::Arc::new(vpay_worker::CallbackPlan::none()),
            1,
            Duration::from_secs(10),
            "worker-recovery-suite".to_owned(),
            async move {
                let _ = stopped.await;
            },
        )
        .await
    });

    // The ordering under test. Waiting for the worker to have *finished* with
    // the job is what makes this a reproduction rather than a race the test
    // might win by being quick: whatever the worker was going to do to this
    // charge, it has already done by the time the confirm's write runs.
    let ran = wait_for_the_poll_job_to_run(&h.pool, &charge_id, Duration::from_secs(20)).await;

    // The confirm's own compare-and-swap, with a redirect rail's arguments.
    let ref_extra = json!({ "pay_token": "tok-the-rail-answered-with" });
    let confirmed = h
        .repositories
        .transaction(|tx| {
            let charge_id = &charge_id;
            let ref_extra = &ref_extra;
            Box::pin(async move {
                let charge = tx
                    .mark_submitted(
                        charge_id,
                        vpay_core::ChargeState::Submitted.as_wire_str(),
                        Some(ref_extra),
                        Some("https://webpay.orange.example/pay/tok"),
                    )
                    .await?;
                Ok::<_, vpay_db::DbError>(TxOutcome::Commit(charge))
            })
        })
        .await;

    let _ = stop.send(());
    let report = worker.await.context("the worker task panicked")?;
    ran?;
    assert!(
        report.claimed >= 1,
        "the loop never claimed the poll job, so nothing raced the confirm"
    );

    let committed = confirmed.context(
        "the confirm's `submitting` -> `submitted` compare-and-swap matched no row: the \
         worker moved the charge first. This is the `write_matched_no_row` 500 the \
         merchant saw, with a live order failed as `provider_unavailable` behind it",
    )?;
    let committed = match committed {
        TxOutcome::Commit(charge) => charge,
        TxOutcome::Abandon(charge) => charge,
    };
    assert_eq!(committed.state, "submitted");

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(
        stored.state, "submitted",
        "the final state must be the confirm's, not the recovery's"
    );
    assert_eq!(stored.failure_code, None);
    assert!(
        events(&h.pool, &intent).await?.is_empty(),
        "a failure event was written for an order the payer is about to be handed"
    );
    Ok(())
}

/// Three consecutive `NotFound`s, over a window, and only then a resubmit.
///
/// Both conditions are load-bearing and the case proves each separately: the
/// first two polls must **not** resubmit (a rail that is merely slow to index
/// a new charge answers 404 for a second or two), and the third must. The
/// window is 50 ms here rather than the documented 60 s, and the streak is the
/// documented 3 — the same `RecoveryPolicy` struct `main` builds, so this is
/// the production code path at a different number, not a test path.
///
/// The reference is `…0404`, which `requesttopay-status.json` answers
/// `RESOURCE_NOT_FOUND` to on every poll — so the streak is real and not
/// staged in the payload.
#[tokio::test]
async fn three_not_founds_over_the_window_resubmit_and_two_do_not() -> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy {
        not_found_streak: 3,
        not_found_window: Duration::from_millis(50),
        ..RecoveryPolicy::default()
    };

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        NOT_FOUND_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    // Kill point 2: the POST was issued and lost. Without an attempt row the
    // very first poll would resubmit for a different reason
    // (`SubmitAttempt::Never`), and this case would prove nothing about the
    // streak.
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        NOT_FOUND_REF,
        None,
    )
    .await?;

    for poll in 1..=2 {
        let settled = h.step(&policy).await?;
        assert!(
            matches!(settled.disposition, Disposition::Rescheduled(_)),
            "poll {poll} should stay on the ladder, got {:?}",
            settled.disposition
        );
        assert!(
            !jobs(&h.pool)
                .await?
                .iter()
                .any(|job| job.kind == "resubmit_charge"),
            "poll {poll} resubmitted: the streak threshold is not being applied, so a rail \
             that is briefly slow to index a charge would be sent a second one"
        );
        make_every_job_runnable(&h.pool).await?;
        // Just past `not_found_window`, once, so the third poll satisfies the
        // *time* condition as well as the count. Sixty milliseconds, not sixty
        // seconds: the window is a policy number and this suite chose it.
        if poll == 1 {
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }

    let third = h.step(&policy).await?;
    assert!(
        matches!(third.disposition, Disposition::Rescheduled(_)),
        "the poll job stays on the ladder while the resubmit runs"
    );
    let queued = jobs(&h.pool).await?;
    assert!(
        queued.iter().any(|job| job.kind == "resubmit_charge"
            && job.dedupe_key == format!("resubmit:{charge_id}")),
        "three consecutive NotFounds over the window did not resubmit; jobs: {queued:?}"
    );

    // Three polls asked the rail three times, all under the one reference the
    // charge has always had.
    assert_one_reference(&h.pool, &charge_id, NOT_FOUND_REF).await;
    Ok(())
}

/// The 24-hour horizon: escalate to `unresolved`, keep polling hourly, alert —
/// and never dead-letter.
///
/// `unresolved_after: 0` puts the horizon in the past for a charge created a
/// moment ago, which is the whole trick; everything else is the production
/// path. The three assertions are the three halves of
/// `docs/flows/reconciler.md`'s sentence, and the last one is the important
/// one: "a late success — minute 40, or hour 30 from `unresolved` — is the
/// normal transition", so a job that stopped polling would lose a payment that
/// eventually settled.
#[tokio::test]
async fn a_charge_past_the_horizon_is_unresolved_polled_hourly_and_alerted_never_parked()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy {
        unresolved_after: Duration::ZERO,
        ..RecoveryPolicy::default()
    };

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        PENDING_THEN_SUCCESS_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        PENDING_THEN_SUCCESS_REF,
        Some(ANSWERED_SENTINEL),
    )
    .await?;

    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "reconciler.md: an escalated charge is polled once an hour — not on the ladder's \
         last rung, and not never"
    );
    assert!(
        settled.alert,
        "the 24-hour escalation must reach a human: JobError::Exhausted is Severity::Error, \
         which is what makes Decision::RetryAfter carry alert: true"
    );
    assert!(
        settled.error.is_some_and(|e| e.contains("exhausted")),
        "the recorded reason must name the exhaustion"
    );

    assert_eq!(
        charge(&h.pool, &charge_id).await?.state,
        "unresolved",
        "the charge must be marked so a human reconciling against the rail's settlement \
         statement can find it"
    );
    assert_eq!(
        intent_status(&h.pool, &intent).await?.0,
        "requires_payment_method",
        "an unresolved charge has not failed, so nothing may stamp last_payment_error on \
         its intent; the status is where the crashed confirm left it (support::confirmed_intent)"
    );
    assert_eq!(
        intent_status(&h.pool, &intent).await?.1,
        None,
        "an escalation is not a decline: the merchant must not be shown an error for a \
         charge that may still succeed"
    );

    let queued = jobs(&h.pool).await?;
    let poll = queued
        .iter()
        .find(|job| job.dedupe_key == format!("poll:{charge_id}"))
        .context("the poll job was deleted; nothing will catch a late success")?;
    let seconds = poll.seconds_until_runnable.context(
        "the poll job was parked at 'infinity'; an unresolved charge is not a dead \
                  letter — it can still succeed",
    )?;
    assert!(
        (3_400.0..3_800.0).contains(&seconds),
        "expected the job about an hour out, got {seconds}s"
    );
    assert!(
        poll.locked_by.is_none(),
        "the lease must be released along with the reschedule"
    );
    assert!(poll.last_error.is_some(), "the reason must be recorded");
    Ok(())
}

/// A redirect-rail charge stuck in `submitting` is failed, not polled.
///
/// `docs/flows/crash-safety.md`: "that `order_id` is dead: abandon it and let
/// the merchant create a new PaymentIntent". Safe only here — the payer is
/// redirected strictly *after* the rail's `pay_token` is committed, so a
/// charge still in `submitting` is one nobody could have paid, and one nobody
/// can ever ask about either, because the token needed to ask was in the
/// response that was lost.
///
/// The sharpest assertion is the negative one: **zero** `provider_requests`
/// rows. Polling it would produce `ProviderError::Config` on every rung of the
/// ladder forever, which is a dead letter dressed up as an outage.
///
/// The branch is on `Capabilities::flow`, never on the rail's code (ADR-0002),
/// which is why this case configures Orange rather than naming it in a
/// condition.
#[tokio::test]
async fn a_redirect_charge_with_no_token_is_failed_without_ever_asking_the_rail()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        REDIRECT_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let reference = Uuid::new_v4();
    // No `payer_ref` and no `provider_ref_extra`: a redirect rail names no
    // payer up front, and the `pay_token` is exactly what was lost.
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        REDIRECT_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        None,
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;

    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::Finished,
        "the charge is resolved, so its job is done; error: {:?}",
        settled.error
    );

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(stored.state, "failed");
    assert_eq!(
        stored.failure_code.as_deref(),
        Some("provider_unavailable"),
        "the payer did nothing wrong: this is our lost response, not their declined payment"
    );

    let (status, error_code) = intent_status(&h.pool, &intent).await?;
    assert_eq!(
        status, "requires_payment_method",
        "the merchant must be able to act — payment-lifecycle.md returns a declined intent \
         here, so a new PaymentIntent is the documented retry"
    );
    assert_eq!(error_code.as_deref(), Some("provider_unavailable"));

    assert!(
        attempted_references(&h.pool, &charge_id).await?.is_empty(),
        "the rail was asked about an order it has no token for; every rung of that ladder \
         is ProviderError::Config and the charge would never resolve"
    );
    assert_eq!(
        events(&h.pool, &intent).await?,
        vec![(
            "payment_intent.payment_failed".to_owned(),
            "pending".to_owned()
        )],
        "exactly one event, awaiting fan-out"
    );
    Ok(())
}

/// A settlement lands on an intent a crashed confirm never moved.
///
/// This is the case F1 named: `confirm` commits the charge and its poll job
/// *before* calling the rail and moves the intent only afterwards, so kill
/// points 1 and 2 leave a live charge against an intent still reading
/// `requires_payment_method`. When the settlement writers' guard named only
/// the two confirmed statuses, the charge compare-and-swap fired, the intent
/// write matched nothing, and the whole settlement became
/// `DbError::WriteMatchedNoRow` → `Category::Internal` → `Retry::Never` → a
/// dead-lettered poll job: a charge the rail had collected, parked forever,
/// with the merchant's intent saying no payment was ever attempted.
///
/// It is asserted here as well as in `vpay-db`'s own suite because this is
/// where the *whole* path runs — the fixture writes what a crash writes, and
/// the shipping loop resolves it.
#[tokio::test]
async fn a_settlement_lands_on_the_intent_a_crashed_confirm_left_behind() -> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    assert_eq!(
        intent_status(&h.pool, &intent).await?.0,
        "requires_payment_method",
        "the fixture must stage the state a crashed confirm actually leaves, or this case \
         proves nothing"
    );

    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    // Kill point 2: the POST went out and the answer was lost. The rail has
    // the payment; vpay's intent says nothing was attempted.
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        reference,
        None,
    )
    .await?;

    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::Finished,
        "the poll job of a charge the rail reports as paid must be finished, not parked; \
         error: {:?}",
        settled.error
    );
    assert!(
        settled.error.is_none(),
        "settling a crashed confirm is not a job failure: {:?}",
        settled.error
    );

    assert_eq!(charge(&h.pool, &charge_id).await?.state, "succeeded");
    assert_eq!(
        intent_status(&h.pool, &intent).await?.0,
        "succeeded",
        "the intent must move with the charge even though the confirm never moved it"
    );
    let amount_received: i64 =
        sqlx::query_scalar("SELECT amount_received FROM payment_intents WHERE id = $1")
            .bind(&intent)
            .fetch_one(&h.pool)
            .await
            .context("reading amount_received")?;
    assert_eq!(
        amount_received, AMOUNT,
        "a settled payment collected the whole amount"
    );
    assert_eq!(
        events(&h.pool, &intent).await?,
        vec![("payment_intent.succeeded".to_owned(), "pending".to_owned())],
        "exactly one event, awaiting fan-out"
    );

    let queued = jobs(&h.pool).await?;
    assert!(
        !queued
            .iter()
            .any(|job| job.dedupe_key == format!("poll:{charge_id}")),
        "the poll job survived; a parked or rescheduled job here means the settlement did \
         not commit: {queued:?}"
    );
    assert_one_reference(&h.pool, &charge_id, reference).await;
    Ok(())
}

/// A rail whose status endpoint keeps failing is still escalated at the
/// horizon.
///
/// The horizon lived inside `keep_polling`, which is reachable only *after* a
/// **successful** `query_status`. A rail answering `503` on every poll —
/// `ProviderError::Unavailable`, `Severity::Warn`, `Retry` — therefore rode
/// the ladder quietly forever: no `unresolved`, no alert, nobody reconciling a
/// charge a payer may have paid. The escalation now hangs off the *failure*
/// branch of the query, so an outage past the horizon reaches a human.
///
/// Two decisive assertions, and they are decisive together:
///
/// * `status_queries == 1` — the rail **was** asked. Escalating without
///   asking would be the opposite bug and would lose the late success
///   `a_late_success_past_the_horizon_still_settles` proves must still land;
///   "past the horizon" is a change of interval and an alert, never a
///   decision to stop asking.
/// * `Rescheduled(3600s)` with the charge `unresolved` — the answer that
///   never came is what escalates. A rung of the ordinary ladder here (10 s,
///   charge still `submitted`, `provider_unavailable` in `last_error`) is
///   exactly the silent ladder this case exists to catch.
#[tokio::test]
async fn a_rail_that_never_answers_is_still_escalated_at_the_horizon() -> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy {
        unresolved_after: Duration::ZERO,
        ..RecoveryPolicy::default()
    };

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        UNAVAILABLE_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    // Kill point 3, so the recovery table advances the bookkeeping and the
    // run reaches the point where it would ask the rail — which is the point
    // under test.
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        UNAVAILABLE_REF,
        Some(ANSWERED_SENTINEL),
    )
    .await?;

    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "past the horizon the charge is polled hourly, whatever the rail is doing; a rung \
         of the ordinary ladder here means the horizon was skipped. error: {:?}",
        settled.error
    );
    assert!(
        settled.alert,
        "a charge past the horizon must reach a human even when the rail is the thing that \
         is broken — a 503 alone is only Severity::Warn"
    );
    assert!(
        settled
            .error
            .as_deref()
            .is_some_and(|error| error.contains("exhausted")),
        "the recorded reason must be the exhaustion, not the rail's outage: {:?}",
        settled.error
    );

    assert_eq!(
        charge(&h.pool, &charge_id).await?.state,
        "unresolved",
        "a charge nobody can get an answer about is exactly what `unresolved` is for"
    );
    assert_eq!(
        status_queries(&h.pool, &charge_id).await?,
        1,
        "the rail must still be asked past the horizon — this run's 503 is what escalates, \
         and a run that skipped the query would also skip a late SUCCESSFUL"
    );

    let queued = jobs(&h.pool).await?;
    let poll = queued
        .iter()
        .find(|job| job.dedupe_key == format!("poll:{charge_id}"))
        .context("the poll job was deleted; a late success would never be seen")?;
    assert!(
        poll.seconds_until_runnable.is_some(),
        "an unresolved charge is not a dead letter — it can still succeed"
    );
    Ok(())
}

/// Past the horizon the rail is still asked, and a terminal answer still
/// settles.
///
/// `docs/flows/reconciler.md` calls a late success — "minute 40, or hour 30
/// from `unresolved`" — **the normal transition**, so the horizon cannot be a
/// gate on asking. This case stages exactly that: a charge whose horizon has
/// passed, against a rail that answers `SUCCESSFUL` on the first status query
/// (`requesttopay-status.json`'s catch-all). The money must land on the intent
/// and the event must be written, exactly as it would have at minute one.
///
/// The decisive assertion is the pair `state == "succeeded"` and
/// `Disposition::Finished`: a worker that escalated *instead of* asking would
/// leave the charge `unresolved` and the job rescheduled an hour out — a
/// payment nothing would ever collect, produced by the very check that exists
/// so nothing is ever lost.
#[tokio::test]
async fn a_late_success_past_the_horizon_still_settles() -> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy {
        // The same trick the two cases above use: zero puts the horizon in
        // the past for a charge created a moment ago, and every other part of
        // the path is the production one.
        unresolved_after: Duration::ZERO,
        ..RecoveryPolicy::default()
    };

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id =
        charge_that_settles_in_one_poll(h.repositories.as_ref(), &h.pool, &intent).await?;

    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::Finished,
        "a settled charge finishes its job; a rescheduled one means the horizon returned \
         before the rail was asked. error: {:?}",
        settled.error
    );
    assert!(
        settled.error.is_none(),
        "settling is not a failure: {:?}",
        settled.error
    );
    assert!(!settled.alert, "a payment that landed pages nobody");

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(
        stored.state, "succeeded",
        "the rail said SUCCESSFUL past the horizon and that is the normal transition \
         (docs/flows/reconciler.md), not an escalation"
    );
    assert_eq!(
        stored.provider_txn_id.as_deref(),
        Some("1234567890"),
        "the rail's own transaction id is what a human reconciles against; the value is \
         requesttopay-status.json's catch-all, so reading it back proves the answer came \
         from the rail and not from a default"
    );
    assert_eq!(
        status_queries(&h.pool, &charge_id).await?,
        1,
        "the rail must be asked exactly once past the horizon — asking zero times is how a \
         late success is lost"
    );

    let (status, error_code) = intent_status(&h.pool, &intent).await?;
    assert_eq!(
        status, "succeeded",
        "the merchant's intent must follow the charge"
    );
    assert_eq!(error_code, None, "a success stamps no last_payment_error");
    assert_eq!(
        amount_received(&h.pool, &intent).await?,
        AMOUNT,
        "amount_received is what a merchant reconciles their books against"
    );

    assert_eq!(
        events(&h.pool, &intent).await?,
        vec![("payment_intent.succeeded".to_owned(), "pending".to_owned())],
        "one event, awaiting fan-out: settlement and event are one transaction"
    );

    let queued = jobs(&h.pool).await?;
    assert!(
        !queued
            .iter()
            .any(|job| job.dedupe_key == format!("poll:{charge_id}")),
        "the poll job must be deleted once the charge is terminal: {queued:?}"
    );

    assert_one_reference(&h.pool, &charge_id, stored.provider_reference_id).await;
    Ok(())
}

/// A resubmit past the horizon is enqueued **and** escalated.
///
/// The `RecoveryAction::Resubmit` arm of `handlers::recover` used to return a
/// rung of the fifteen-minute ladder unconditionally, ignoring the horizon its
/// own caller had already evaluated. A `submitting` charge whose rail answers
/// `404` on every poll therefore cycled forever: resubmit, ladder, three more
/// `NotFound`s, resubmit again — never `unresolved`, never alerting, for a
/// charge a payer may have paid a day ago. The escalation is what
/// `docs/flows/reconciler.md` promises at 24 hours, and it must not be
/// skippable by the one branch that happens to have something else to do
/// first.
///
/// The order is asserted as well as the escalation: the resubmit row is
/// committed *before* the escalation, so the recovery table's answer is
/// written rather than skipped. What it is worth is stated exactly in
/// `handlers::resubmit_then_escalate_if_late` and is deliberately narrower
/// than "the charge is resubmitted": the escalation moves the charge to
/// `unresolved`, so unless a concurrent worker claims the resubmit job in
/// between, that job will find the charge outside `submitting` and finish
/// without calling the rail. Past the horizon the guarantee is the alert and
/// the hourly poll — whether to push another submission at a charge a human is
/// already reconciling is theirs to decide.
///
/// This case crosses the horizon between polls, which is why it ages
/// `charges.created_at` instead of setting `unresolved_after: ZERO` — the
/// streak needs two polls on the near side of the horizon and the third on the
/// far side, and a policy of zero would escalate the first one.
#[tokio::test]
async fn a_resubmit_past_the_horizon_still_escalates() -> anyhow::Result<()> {
    let h = harness().await?;
    // The documented 24 hours, untouched. Only the window is tightened, the
    // same way `three_not_founds_over_the_window_resubmit_and_two_do_not`
    // tightens it, so this is the production horizon rather than a test one.
    let policy = RecoveryPolicy {
        not_found_streak: 3,
        not_found_window: Duration::from_millis(50),
        ..RecoveryPolicy::default()
    };

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        NOT_FOUND_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    // Kill point 2, exactly as the streak case stages it: without an attempt
    // row the very first poll would resubmit for a different reason
    // (`SubmitAttempt::Never`) and this case would never reach the arm it is
    // about.
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        NOT_FOUND_REF,
        None,
    )
    .await?;

    for poll in 1..=2 {
        let settled = h.step(&policy).await?;
        assert!(
            matches!(settled.disposition, Disposition::Rescheduled(_)),
            "poll {poll} is inside the horizon and should stay on the ladder, got {:?}",
            settled.disposition
        );
        assert!(
            !settled.alert,
            "poll {poll} is inside the horizon: nothing may alert yet"
        );
        make_every_job_runnable(&h.pool).await?;
        if poll == 1 {
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }
    assert_eq!(
        charge(&h.pool, &charge_id).await?.state,
        "submitting",
        "the two near-side polls must leave the charge where the crash left it, or the \
         third poll would not reach the recovery table at all"
    );

    // A day passes. Nothing else changes.
    age_past_the_horizon(&h.pool, &charge_id).await?;

    let third = h.step(&policy).await?;

    let queued = jobs(&h.pool).await?;
    assert!(
        queued.iter().any(|job| job.kind == "resubmit_charge"
            && job.dedupe_key == format!("resubmit:{charge_id}")),
        "the escalation was written before the recovery table's answer: the resubmit row \
         must be committed first, and the escalation only on top of it; jobs: {queued:?}"
    );
    assert_eq!(
        third.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "past the horizon the poll is hourly, not a rung of the ladder. error: {:?}",
        third.error
    );
    assert_eq!(
        charge(&h.pool, &charge_id).await?.state,
        "unresolved",
        "a charge whose rail has denied all knowledge of it for a day is exactly what \
         `unresolved` is for; leaving it `submitting` is the ladder running forever"
    );
    assert!(
        third.alert,
        "the whole point of the escalation is that a human is told"
    );
    assert!(
        third
            .error
            .as_deref()
            .is_some_and(|error| error.contains("exhausted")),
        "the recorded reason must be the exhaustion: {:?}",
        third.error
    );

    assert_one_reference(&h.pool, &charge_id, NOT_FOUND_REF).await;
    Ok(())
}

/// The same rule in the crash-recovery block, which runs *before* the ladder.
///
/// `SubmitAttempt::Never` — no `provider_requests` row at all — answers
/// `Resubmit` on the first poll, before any rail is asked. That branch is
/// normally self-limiting, because the resubmit it enqueues moves the charge
/// out of `submitting`; it is not self-limiting when the resubmit itself is
/// dead-lettered, which leaves the poll cycling `Never → Resubmit` on the
/// ladder forever. Evaluating the horizon above the block, rather than below
/// it, makes the one rule cover both arms: enqueue the resubmit, then
/// escalate.
///
/// The negative assertion is the one worth keeping: the rail is still never
/// asked here. An escalation must not become an excuse to query a charge the
/// recovery table has just concluded was never sent.
#[tokio::test]
async fn a_never_submitted_charge_past_the_horizon_escalates_after_enqueuing_the_resubmit()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    age_past_the_horizon(&h.pool, &charge_id).await?;

    let settled = h.step(&policy).await?;

    let queued = jobs(&h.pool).await?;
    assert!(
        queued.iter().any(|job| job.kind == "resubmit_charge"
            && job.dedupe_key == format!("resubmit:{charge_id}")),
        "the resubmit row must be committed before the escalation, not skipped by it \
         (`handlers::resubmit_then_escalate_if_late` says what that is worth); \
         jobs: {queued:?}"
    );
    assert_eq!(
        settled.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "hourly, not a rung. error: {:?}",
        settled.error
    );
    assert_eq!(
        charge(&h.pool, &charge_id).await?.state,
        "unresolved",
        "a day-old charge that has never reached the rail must be escalated, not left \
         cycling on the ladder"
    );
    assert!(settled.alert, "a human must be told");
    assert!(
        attempted_references(&h.pool, &charge_id).await?.is_empty(),
        "the rail must still not be asked about a charge that was never sent to it"
    );
    Ok(())
}

/// Past the horizon, a **poisoned** job is still parked — it is not
/// re-classified as the rail's silence.
///
/// The horizon's error arm used to catch every `JobError`. A `Poisoned` row —
/// here a charge whose `currency_code` this build cannot parse — therefore
/// came back as `JobError::Exhausted`: `Category::Rail`, hourly, alerting
/// forever, on a job that will fail identically at every one of those hours.
/// ADR-0011 forbids exactly that: a composite delegates its classification and
/// never re-decides a leaf's. The arm now names `JobError::Provider`, which is
/// the only leaf whose silence the escalation is about.
///
/// The decisive pair is `DeadLettered` **and** the charge still `submitting`:
/// an escalation would have written `unresolved` and rescheduled an hour out,
/// which is a broken row dressed up as an unreconciled payment.
#[tokio::test]
async fn a_poisoned_job_past_the_horizon_is_parked_rather_than_rescheduled_hourly()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy {
        unresolved_after: Duration::ZERO,
        ..RecoveryPolicy::default()
    };

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    // Kill point 2, so the recovery table answers `Poll` and the run reaches
    // the status query — which is where the unparseable row is read.
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        reference,
        None,
    )
    .await?;
    write_a_currency_this_build_cannot_parse(&h.pool, &charge_id).await?;

    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::DeadLettered,
        "a job whose row cannot be interpreted must be parked; rescheduling it hourly is \
         the horizon re-classifying a leaf error it does not own (ADR-0011). error: {:?}",
        settled.error
    );
    assert!(
        settled
            .error
            .as_deref()
            .is_some_and(|error| error.contains("poisoned")),
        "the recorded reason must be the poisoning, not an exhaustion: {:?}",
        settled.error
    );
    assert_eq!(
        charge(&h.pool, &charge_id).await?.state,
        "submitting",
        "nothing about a broken job row says the payment is unreconciled; marking it \
         `unresolved` would put a bug on an operator's reconciliation list"
    );
    assert_eq!(
        status_queries(&h.pool, &charge_id).await?,
        0,
        "the row was rejected before the rail was called, which is why no retry can fix it"
    );

    let queued = jobs(&h.pool).await?;
    let parked = queued
        .iter()
        .find(|job| job.dedupe_key == format!("poll:{charge_id}"))
        .context("the poisoned job was deleted rather than parked")?;
    assert!(
        parked.seconds_until_runnable.is_none(),
        "a parked job sits at run_at = 'infinity'; an hourly reschedule is what this case \
         exists to refuse"
    );
    Ok(())
}

/// The second hourly poll of a charge that is already `unresolved` re-raises
/// the alert and writes nothing.
///
/// `escalate_to_unresolved` skips its `set_live_state` when the charge is
/// already `unresolved`, and that skip is what makes the hourly escalation
/// idempotent. It matters because `charges.updated_at` is the only record this
/// schema keeps of "something changed" — there is no charge-transition table —
/// so an escalation that re-wrote the row every hour would erase the timestamp
/// an operator uses to see when the charge last actually moved.
///
/// Both halves are asserted together: the row does not move **and** the alert
/// still fires. Either alone would be satisfied by a bug — a no-op that also
/// dropped the alert would pass the first, and a rewrite that kept alerting
/// would pass the second.
#[tokio::test]
async fn a_second_hourly_poll_of_an_unresolved_charge_re_alerts_without_writing_it_again()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy {
        unresolved_after: Duration::ZERO,
        ..RecoveryPolicy::default()
    };

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        NOT_FOUND_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    // Kill point 3: the rail answered the submit, so the recovery table
    // advances the bookkeeping and the poll runs the ordinary path.
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        NOT_FOUND_REF,
        Some(ANSWERED_SENTINEL),
    )
    .await?;

    let first = h.step(&policy).await?;
    assert_eq!(
        first.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "the first poll past the horizon escalates. error: {:?}",
        first.error
    );
    assert_eq!(charge(&h.pool, &charge_id).await?.state, "unresolved");
    let after_escalation = charge_updated_at(&h.pool, &charge_id).await?;

    make_every_job_runnable(&h.pool).await?;
    let second = h.step(&policy).await?;

    assert_eq!(
        charge_updated_at(&h.pool, &charge_id).await?,
        after_escalation,
        "the second hourly poll re-wrote the charge; `updated_at` must keep naming the \
         moment the charge last actually changed, which is the escalation"
    );
    assert_eq!(
        charge(&h.pool, &charge_id).await?.state,
        "unresolved",
        "and it must still be escalated"
    );
    assert_eq!(
        second.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "still hourly. error: {:?}",
        second.error
    );
    assert!(
        second.alert,
        "the alert repeats: a charge nobody has reconciled after 25 hours is not less \
         urgent than it was at 24"
    );
    assert!(
        second
            .error
            .as_deref()
            .is_some_and(|error| error.contains("exhausted")),
        "the reason must still be the exhaustion: {:?}",
        second.error
    );
    assert_eq!(
        status_queries(&h.pool, &charge_id).await?,
        2,
        "every hourly run asks the rail again — that is what a late success arrives \
         through (docs/flows/reconciler.md)"
    );
    assert_one_reference(&h.pool, &charge_id, NOT_FOUND_REF).await;
    Ok(())
}

/// A decline reaches a charge that is already `unresolved`, and settles it.
///
/// This is the half of "past the horizon a terminal answer settles normally"
/// that the success cases do not cover, and the thing that makes it work is
/// one constant: `vpay_db::payment_intents::LIVE_CHARGE_STATES` includes
/// `'unresolved'`, so `apply_failed`'s compare-and-swap matches an escalated
/// charge. Drop `unresolved` from that list and every escalated charge becomes
/// unsettleable — the rail's verdict would match no row, the settlement would
/// raise `DbError::WriteMatchedNoRow`, and the charge would alert hourly
/// forever with the answer already in hand. This case pins it.
///
/// The escalated state is staged rather than reached through the loop because
/// a WireMock reference answers the same way every time: the case needs a
/// non-terminal answer to escalate and a terminal one to settle.
/// `escalated_charge` writes exactly what
/// `a_charge_past_the_horizon_is_unresolved_polled_hourly_and_alerted_never_parked`
/// proves the worker writes.
#[tokio::test]
async fn a_decline_past_the_horizon_settles_an_unresolved_charge_and_clears_the_alert()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy {
        unresolved_after: Duration::ZERO,
        ..RecoveryPolicy::default()
    };

    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        DECLINED_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    escalated_charge(&h.pool, &charge_id).await?;

    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::Finished,
        "a terminal answer settles an escalated charge exactly as it settles any other, \
         and its job is done. error: {:?}",
        settled.error
    );
    assert!(
        settled.error.is_none(),
        "settling is not a job failure: {:?}",
        settled.error
    );
    assert!(
        !settled.alert,
        "the escalation's alert stops when the charge stops being unreconciled"
    );

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(
        stored.state, "failed",
        "the rail declined; `unresolved` is an escalation, not a terminal state, and \
         LIVE_CHARGE_STATES is what lets the settlement reach it"
    );
    assert_eq!(stored.failure_code.as_deref(), Some("insufficient_funds"));

    let (status, error_code) = intent_status(&h.pool, &intent).await?;
    assert_eq!(
        status, "requires_payment_method",
        "the merchant must be able to open a new PaymentIntent"
    );
    assert_eq!(error_code.as_deref(), Some("insufficient_funds"));
    assert_eq!(
        events(&h.pool, &intent).await?,
        vec![(
            "payment_intent.payment_failed".to_owned(),
            "pending".to_owned()
        )],
        "one event, awaiting fan-out"
    );

    let queued = jobs(&h.pool).await?;
    assert!(
        !queued
            .iter()
            .any(|job| job.dedupe_key == format!("poll:{charge_id}")),
        "the poll job must be deleted once the charge is terminal — an hourly poll of a \
         settled charge is an alert nobody can act on: {queued:?}"
    );
    assert_eq!(
        status_queries(&h.pool, &charge_id).await?,
        1,
        "the rail was asked once, past the horizon, and its answer is what settled the \
         charge"
    );
    assert_one_reference(&h.pool, &charge_id, DECLINED_REF).await;
    Ok(())
}

/// A status read the rail **answered** and the adapter refused is recorded as
/// answered; one that got a `503` is not.
///
/// The defect, measured on 2026-10-21 against Notch Pay's real sandbox
/// (RFC-0012 step 5): eleven reads of one charge, each a `complete` whose
/// `amounts.total` the adapter refused as `Malformed`, each written to
/// `provider_requests` as `status_code IS NULL, responded_at IS NULL` — "no
/// answer", although the rail had answered and the adapter had parsed it. The
/// `UPDATE` is what is asserted, read back from the table: the worker's
/// `read_failure_status` is a pure function, and a unit test of it proves the
/// table of variants, not that the row says so.
///
/// The `503` is the other half of the case, and it keeps it honest: a fix that
/// recorded *every* refused read as answered would pass the first charge and
/// fail the second. A `503` is `Transport`, and `Transport` stays `NULL` because
/// the port cannot tell "the rail answered 5xx" from "the rail was never
/// reached" (`handlers::read_failure_status`) — which means this row
/// under-claims, and the assertion says so rather than hiding it.
///
/// Both charges are staged at kill point 3 so the recovery table advances the
/// bookkeeping and the poll reaches the rail, which is the point under test.
#[tokio::test]
async fn a_status_read_the_adapter_refused_is_recorded_as_answered_and_a_503_is_not()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let mut charge_ids = Vec::new();
    for reference in [HUGE_ANSWER_REF, UNAVAILABLE_REF] {
        let intent = confirmed_intent(
            h.repositories.as_ref(),
            MERCHANT,
            PUSH_RAIL,
            AMOUNT,
            CURRENCY,
        )
        .await?;
        let charge_id = crashed_charge(
            h.repositories.as_ref(),
            &intent,
            PUSH_RAIL,
            reference,
            AMOUNT,
            CURRENCY,
            Some(MSISDN),
        )
        .await?;
        age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
        record_submit_attempt(
            h.repositories.as_ref(),
            &charge_id,
            PUSH_RAIL,
            reference,
            Some(ANSWERED_SENTINEL),
        )
        .await?;
        charge_ids.push(charge_id);
    }
    let [huge, unavailable] = charge_ids.as_slice() else {
        panic!("two charges were staged");
    };

    // One poll each. Neither read settles anything, so each job is
    // rescheduled on the ladder rather than finished.
    for _ in 0..2 {
        let settled = h.step(&policy).await?;
        assert_eq!(settled.kind, "poll_charge");
        assert!(
            matches!(settled.disposition, Disposition::Rescheduled(_)),
            "a read that settles nothing keeps its charge on the ladder; got {:?}, error: {:?}",
            settled.disposition,
            settled.error
        );
    }

    assert_eq!(
        attempts_of(&h.pool, huge, "query_status").await?,
        vec![StoredAttempt {
            status_code: Some(ANSWERED_SENTINEL),
            answered: true,
            error_kind: Some("provider_error".to_owned()),
        }],
        "the rail answered this read with a body the adapter refused; recording it as `NULL` \
         is the 2026-10-21 defect — 'no answer' for eleven reads Notch Pay had answered"
    );
    assert_eq!(
        attempts_of(&h.pool, unavailable, "query_status").await?,
        vec![StoredAttempt {
            status_code: None,
            answered: false,
            error_kind: Some("provider_unavailable".to_owned()),
        }],
        "`Transport` stays unanswered: the port cannot say whether a 503 reached us, so the \
         row under-claims rather than claims an answer it cannot prove"
    );

    // Nothing about the submit rows or the charges moved: this is a change to
    // how a refused *read* is written, and to nothing a settlement reads.
    for charge_id in [huge, unavailable] {
        assert_eq!(charge(&h.pool, charge_id).await?.state, "submitted");
        assert_eq!(
            attempts_of(&h.pool, charge_id, "submit").await?,
            vec![StoredAttempt {
                status_code: Some(ANSWERED_SENTINEL),
                answered: true,
                error_kind: None,
            }],
            "the staged submit row is not this change's to touch"
        );
    }
    Ok(())
}

/// A charge a crash left in `submitting` with nothing ever sent (kill point 1),
/// old enough for the recovery table to apply to it, whose rail reference the
/// WireMock tree answers specially.
async fn never_sent_charge(h: &Harness, reference: Uuid) -> anyhow::Result<String> {
    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    assert!(
        attempted_references(&h.pool, &charge_id).await?.is_empty(),
        "kill point 1 must be staged with no attempt row at all"
    );
    Ok(charge_id)
}

/// Runs the two jobs a never-sent charge needs — the poll that enqueues the
/// resubmit, then the resubmit — and returns what the **resubmit** settled as.
async fn run_the_resubmit(
    h: &Harness,
    policy: &RecoveryPolicy,
    charge_id: &str,
) -> anyhow::Result<Settled> {
    let poll = h.step(policy).await?;
    assert_eq!(poll.kind, "poll_charge");
    make_every_job_runnable(&h.pool).await?;
    let resubmit = h.step(policy).await?;
    assert_eq!(
        resubmit.kind, "resubmit_charge",
        "the second job for {charge_id} must be the resubmit the first one enqueued"
    );
    Ok(resubmit)
}

/// A resubmit whose answer the adapter refused as `Malformed` stays
/// **unanswered**, and recovery still reads it as unresolved.
///
/// This is the half of the fix that must *not* change. `submit_again` shares
/// its failure path's shape with `query_status`, and the tempting one-line fix
/// for the 2026-10-21 defect is to make both record `Malformed` as answered.
/// That is wrong on a submit: `recovery_step` turns an answered `submit` row
/// into `RecoveryAction::Advance` — "the rail accepted it; advance the charge" —
/// and a `307` the adapter would not follow, like Notch Pay's unresolvable `409`
/// (`vpay-adapter-notchpay`), is `Malformed` precisely because the charge's
/// fate is *unknown*. The row is read back through `latest_submit_attempt`,
/// the statement `recovery_action` runs, so what is asserted is the evidence
/// recovery would act on.
#[tokio::test]
async fn a_resubmit_the_adapter_refuses_as_malformed_stays_unanswered_for_recovery()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();
    let charge_id = never_sent_charge(&h, REDIRECTED_SUBMIT_REF).await?;

    let resubmit = run_the_resubmit(&h, &policy, &charge_id).await?;
    assert!(
        matches!(resubmit.disposition, Disposition::Rescheduled(_)),
        "a `Malformed` leaves the fate unknown, so the resubmit is retried on the ladder; \
         got {:?}, error: {:?}",
        resubmit.disposition,
        resubmit.error
    );

    assert_eq!(
        attempts_of(&h.pool, &charge_id, "submit").await?,
        vec![StoredAttempt {
            status_code: None,
            answered: false,
            error_kind: Some("provider_error".to_owned()),
        }],
        "a refused answer to a submit is not an acceptance, so it must not be written the way \
         one is: recovery would advance the charge"
    );
    let latest = h
        .repositories
        .latest_submit_attempt(&charge_id)
        .await?
        .context("the resubmit left no attempt row")?;
    assert_eq!(
        latest.status_code, None,
        "`SubmitAttempt::Unanswered` is what `recovery_step` must see after this"
    );
    assert_eq!(charge(&h.pool, &charge_id).await?.state, "submitting");
    assert_one_reference(&h.pool, &charge_id, REDIRECTED_SUBMIT_REF).await;
    Ok(())
}

/// A resubmit the rail **declines** fails the charge, and only then is its
/// attempt row marked answered.
///
/// This case used to be named `a_resubmit_the_rail_declines_stays_unanswered_
/// because_nothing_fails_the_charge` and pinned the opposite: the loop finished
/// the job and nothing wrote the charge, so it stayed `submitting` beside an
/// unanswered row (vaam-apps/vpay#314). Now the worker settles it the way
/// `vpay-api`'s first submit does — charge `failed` with the rail's failure
/// code, the intent back to `requires_payment_method` carrying the error pair,
/// and one `payment_intent.payment_failed` event, in one transaction — and the
/// row then says what the rail said: the sentinel and `charge_declined`, the
/// row `vpay-api`'s confirm writes for the same answer.
///
/// The row is asserted **together with** the charge, not instead of it. An
/// answered `submit` row beside a `submitting` charge is the combination
/// `recovery_step` reads as `Advance` ("the rail accepted it"), so a change
/// that wrote the row and not the settlement would pass the row assertion and
/// fail the state one — which is the pair that matters.
///
/// The job is still `Finished`: the error is raised unchanged after the
/// settlement, so `Classify::retry` (`Retry::NewAttempt`) still decides that,
/// and `log_failure` still counts it and still pages a blocked partner account.
#[tokio::test]
async fn a_resubmit_the_rail_declines_fails_the_charge_and_is_recorded_as_answered()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();
    let charge_id = never_sent_charge(&h, DECLINED_SUBMIT_REF).await?;
    let intent = intent_of(&h.pool, &charge_id).await?;

    let resubmit = run_the_resubmit(&h, &policy, &charge_id).await?;
    assert_eq!(
        resubmit.disposition,
        Disposition::Finished,
        "a `Rejected` is `Retry::NewAttempt`: the loop finishes the job; error: {:?}",
        resubmit.error
    );

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(
        stored.state, "failed",
        "a declined resubmit must fail the charge; left `submitting` it is resent after every \
         third poll until the 24-hour horizon"
    );
    assert_eq!(
        stored.failure_code.as_deref(),
        Some(vpay_core::FailureCode::InvalidPayer.as_str()),
        "MTN's `PAYER_NOT_FOUND` is `invalid_payer` in the closed vocabulary"
    );
    assert_eq!(
        intent_status(&h.pool, &intent).await?,
        (
            "requires_payment_method".to_owned(),
            Some(vpay_core::FailureCode::InvalidPayer.as_str().to_owned())
        ),
        "the intent goes back to the payer with the reason, as it does for a decline at confirm"
    );
    assert_eq!(
        event_types(&h.pool, &intent).await?,
        vec!["payment_intent.payment_failed".to_owned()],
        "the merchant is told exactly once"
    );

    assert_eq!(
        attempts_of(&h.pool, &charge_id, "submit").await?,
        vec![StoredAttempt {
            status_code: Some(ANSWERED_SENTINEL),
            answered: true,
            error_kind: Some("charge_declined".to_owned()),
        }],
        "the rail answered, and the charge is terminal, so the row may say so"
    );
    assert_one_reference(&h.pool, &charge_id, DECLINED_SUBMIT_REF).await;
    Ok(())
}

/// After a declined resubmit **nothing asks the rail again**.
///
/// The measurement this replaces: against a rail that declines every submit and
/// answers `404` to every read, a charge left `submitting` was resubmitted after
/// every third poll — submits 1, 2, 3, 4 at steps 2, 6, 10, 14 of sixteen, with
/// eleven status reads between them — and the intent stayed
/// `requires_payment_method` with no reason, for as long as the charge lived.
/// The stub pair is what makes the rail honest: `…0404` is `NotFound` on every
/// read and MSISDN `237670000400` is declined on every submit, so a charge that
/// stays live is resubmitted by the production table and not by a staged payload.
///
/// Asserted as an empty queue and as counts, not as "sixteen steps passed": the
/// leftover poll job finds a terminal charge and finishes without a status read,
/// so the rail is asked for a submit once and for a status never.
#[tokio::test]
async fn a_declined_resubmit_is_not_resent_by_the_poll_that_follows() -> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy {
        not_found_streak: 3,
        not_found_window: Duration::from_millis(50),
        ..RecoveryPolicy::default()
    };
    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        NOT_FOUND_REF,
        AMOUNT,
        CURRENCY,
        Some(DECLINED_PAYER_MSISDN),
    )
    .await?;
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;

    let resubmit = run_the_resubmit(&h, &policy, &charge_id).await?;
    assert_eq!(resubmit.disposition, Disposition::Finished);
    assert_eq!(charge(&h.pool, &charge_id).await?.state, "failed");

    // The poll job the first step rescheduled is still queued. Past the window
    // it would have started a `NotFound` streak against `…0404` and resubmitted
    // on the third; on a terminal charge it must finish without asking.
    tokio::time::sleep(Duration::from_millis(60)).await;
    make_every_job_runnable(&h.pool).await?;
    let leftover = h.step(&policy).await?;
    assert_eq!(leftover.kind, "poll_charge");
    assert_eq!(
        leftover.disposition,
        Disposition::Finished,
        "a terminal charge's poll is done work; error: {:?}",
        leftover.error
    );

    assert!(
        jobs(&h.pool).await?.is_empty(),
        "a job is still queued for a charge the rail declined, so something will ask again"
    );
    assert_eq!(
        attempts_of(&h.pool, &charge_id, "submit").await?.len(),
        1,
        "the rail was sent the declined charge more than once"
    );
    assert_eq!(
        status_queries(&h.pool, &charge_id).await?,
        0,
        "a terminal charge must not cost the rail a status read"
    );
    Ok(())
}

/// What `vpay-api`'s confirm leaves when it **could not settle** a decline — a
/// `submitting` charge and a pending attempt row — heals: the worker resubmits
/// into the same decline and fails the charge.
///
/// This is the second half of the argument for the confirm path's order
/// (`confirm_rails.rs`'s `a_decline_the_database_refuses_to_settle_leaves_the_attempt_unanswered`
/// shows the first: the refused settlement leaves this state and no other). The
/// row is staged at kill point 2 — the same pair — against the honest stub pair:
/// `…0404` is `NotFound` on every read and MSISDN `237670000400` is declined on
/// every submit, so the production table has to do the work: `Unanswered` →
/// poll → three `NotFound`s over the window → resubmit → `Rejected` →
/// `settle_failed_with` (vaam-apps/vpay#316). Nothing here is staged past the
/// first row.
///
/// Asserted as the end state and its cost: charge `failed` with the rail's code,
/// the intent told once, **three** status reads and **two** submit rows (the
/// staged pending one and the answered `charge_declined` one), and nothing left
/// in the queue.
#[tokio::test]
async fn a_declined_submit_whose_settlement_was_lost_is_failed_by_the_worker() -> anyhow::Result<()>
{
    let h = harness().await?;
    let policy = RecoveryPolicy {
        not_found_streak: 3,
        not_found_window: Duration::from_millis(50),
        ..RecoveryPolicy::default()
    };
    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        PUSH_RAIL,
        NOT_FOUND_REF,
        AMOUNT,
        CURRENCY,
        Some(DECLINED_PAYER_MSISDN),
    )
    .await?;
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        PUSH_RAIL,
        NOT_FOUND_REF,
        None,
    )
    .await?;

    let mut steps = 0;
    let mut state = charge(&h.pool, &charge_id).await?.state;
    while state == "submitting" {
        steps += 1;
        anyhow::ensure!(
            steps <= 12,
            "a charge the rail declined was still `submitting` after {steps} steps"
        );
        make_every_job_runnable(&h.pool).await?;
        tokio::time::sleep(Duration::from_millis(60)).await;
        h.step(&policy).await?;
        state = charge(&h.pool, &charge_id).await?.state;
        anyhow::ensure!(
            state != "submitted",
            "recovery advanced a charge the rail declined to `submitted` at step {steps}; a \
             pending row beside a `submitting` charge is `Unanswered`, never `Advance`"
        );
    }

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(stored.state, "failed");
    assert_eq!(
        stored.failure_code.as_deref(),
        Some(vpay_core::FailureCode::InvalidPayer.as_str())
    );
    assert_eq!(
        event_types(&h.pool, &intent).await?,
        vec!["payment_intent.payment_failed".to_owned()],
        "the merchant is told once"
    );
    assert_eq!(
        status_queries(&h.pool, &charge_id).await?,
        3,
        "the streak is three reads, and the resubmit that failed the charge ends the asking"
    );
    assert_eq!(
        attempts_of(&h.pool, &charge_id, "submit").await?,
        vec![
            StoredAttempt {
                status_code: None,
                answered: false,
                error_kind: None,
            },
            StoredAttempt {
                status_code: Some(ANSWERED_SENTINEL),
                answered: true,
                error_kind: Some("charge_declined".to_owned()),
            },
        ],
        "the staged pending row stays as the crash left it; the resubmit's row says what the \
         rail said"
    );

    // The leftover poll job finds a terminal charge and finishes without asking.
    make_every_job_runnable(&h.pool).await?;
    let leftover = h.step(&policy).await?;
    assert_eq!(leftover.disposition, Disposition::Finished);
    assert!(jobs(&h.pool).await?.is_empty());
    Ok(())
}

// ------------------------------------- an answer only a person can act on

/// A Notch Pay charge the rail has accepted: `pending`, holding
/// `trx.test_{reference}` as its `notchpay_reference`, with its poll job due.
///
/// Staged from [`crashed_charge`] and moved by hand, because the stub selects
/// its answer by reference and a confirm cannot choose one. `pending` and the
/// key material are exactly what `persist_submitted` and the first rung leave
/// for a push rail; nothing else on the row differs from a confirmed charge.
async fn accepted_notchpay_charge(h: &Harness, reference: Uuid) -> anyhow::Result<String> {
    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        NOTCHPAY,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        NOTCHPAY,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    let moved = sqlx::query(
        "UPDATE charges SET state = 'pending', \
         provider_ref_extra = jsonb_build_object('notchpay_reference', $2::TEXT) \
         WHERE id = $1 AND state = 'submitting'",
    )
    .bind(&charge_id)
    .bind(format!("trx.test_{reference}"))
    .execute(&h.pool)
    .await
    .context("moving the charge to pending with its Notch Pay reference")?
    .rows_affected();
    anyhow::ensure!(moved == 1, "the staged charge was not there to move");
    Ok(charge_id)
}

/// The `jobs.id` of a charge's poll job, so a step can be matched to the
/// charge it ran for when two are due together.
async fn poll_job_id(pool: &PgPool, charge_id: &str) -> anyhow::Result<Uuid> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM jobs WHERE dedupe_key = $1")
        .bind(format!("poll:{charge_id}"))
        .fetch_one(pool)
        .await
        .context("reading the poll job's id")
}

/// Runs one poll for each of `charges` and returns each charge's [`Settled`],
/// in the order given.
async fn poll_each(
    h: &Harness,
    policy: &RecoveryPolicy,
    charges: &[&str],
) -> anyhow::Result<Vec<Settled>> {
    let mut by_job = BTreeMap::new();
    for charge_id in charges {
        by_job.insert(poll_job_id(&h.pool, charge_id).await?, *charge_id);
    }
    let mut settled = BTreeMap::new();
    for _ in charges {
        let step = h.step(policy).await?;
        assert_eq!(step.kind, "poll_charge");
        let charge_id = by_job
            .get(&step.job_id)
            .with_context(|| format!("a job no charge here owns ran: {}", step.job_id))?;
        settled.insert(*charge_id, step);
    }
    charges
        .iter()
        .map(|charge_id| {
            settled
                .remove(charge_id)
                .with_context(|| format!("{charge_id}'s poll never ran"))
        })
        .collect()
}

/// A short collection is `unresolved` on the **first** poll that reads it,
/// with the hourly poll and the alert — and an answer the adapter merely
/// could not read is not (vaam-apps/vpay#313).
///
/// The short charge is Notch Pay's sandbox "insufficient funds" read, as
/// RFC-0012 step 5 met it on 2026-10-21: `complete` for the charge's amount
/// with `amounts.total` short of it. That run kept the charge `pending` on the
/// ladder for 15 minutes at `WARN`, and by the code it would have stayed there
/// until the 24-hour horizon. Here the policy **is** the production one —
/// `RecoveryPolicy::default()`, 24 hours — so nothing but the leaf's own
/// variant can explain the escalation.
///
/// The second charge keeps the case honest: a body past
/// `MAX_RAIL_BODY_BYTES` is `Malformed`, an answer a later read may well fix,
/// and it must still ride the ladder quietly. A worker that escalated every
/// refused read would pass the first half and fail this one.
///
/// Then a second hourly poll, as
/// `a_second_hourly_poll_of_an_unresolved_charge_re_alerts_without_writing_it_again`
/// pins for the horizon: the alert repeats, the row is not rewritten, and the
/// rail is asked again — which is the road a late answer arrives by.
#[tokio::test]
async fn a_short_collection_is_unresolved_on_the_first_poll_and_an_unreadable_answer_is_not()
-> anyhow::Result<()> {
    let h = notchpay_harness().await?;
    let policy = RecoveryPolicy::default();
    let short = accepted_notchpay_charge(&h, SHORT_COLLECTION_REF).await?;
    let unreadable = accepted_notchpay_charge(&h, HUGE_ANSWER_REF).await?;

    let [short_poll, unreadable_poll] = <[Settled; 2]>::try_from(
        poll_each(&h, &policy, &[short.as_str(), unreadable.as_str()]).await?,
    )
    .map_err(|_| anyhow::anyhow!("two polls ran"))?;

    assert_eq!(
        short_poll.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "a short collection is escalated on its first read and polled hourly from there — \
         not left on the ladder for 24 hours, and not parked. error: {:?}",
        short_poll.error
    );
    assert!(
        short_poll.alert,
        "the escalation must reach a person now: that is the whole of #313"
    );
    assert!(
        short_poll
            .error
            .as_deref()
            .is_some_and(|error| error.contains("exhausted")),
        "the job records the escalation, as at the horizon: {:?}",
        short_poll.error
    );
    assert_eq!(
        charge(&h.pool, &short).await?.state,
        "unresolved",
        "`unresolved` is the state the runbook's query and VpayUnresolvedChargesRising find"
    );
    assert_eq!(
        charge(&h.pool, &short).await?.failure_code,
        None,
        "an escalation is not a decline: money may have moved"
    );
    assert_eq!(
        attempts_of(&h.pool, &short, "query_status").await?,
        vec![StoredAttempt {
            status_code: Some(ANSWERED_SENTINEL),
            answered: true,
            error_kind: Some("charge_needs_reconciliation".to_owned()),
        }],
        "Notch Pay answered this read, and the row must say so and say why — `NULL` is the \
         2026-10-21 defect #314 fixed"
    );

    assert!(
        matches!(unreadable_poll.disposition, Disposition::Rescheduled(delay) if delay < Duration::from_secs(60)),
        "an unreadable answer stays on the ladder's first rungs; got {:?}, error: {:?}",
        unreadable_poll.disposition,
        unreadable_poll.error
    );
    assert!(
        !unreadable_poll.alert,
        "a `Malformed` read is `Warn` and alerts nobody"
    );
    assert_eq!(charge(&h.pool, &unreadable).await?.state, "pending");
    assert_eq!(
        attempts_of(&h.pool, &unreadable, "query_status").await?,
        vec![StoredAttempt {
            status_code: Some(ANSWERED_SENTINEL),
            answered: true,
            error_kind: Some("provider_error".to_owned()),
        }],
    );

    // The second hourly poll of the escalated charge.
    let escalated_at = charge_updated_at(&h.pool, &short).await?;
    make_every_job_runnable(&h.pool).await?;
    let [again, _] = <[Settled; 2]>::try_from(
        poll_each(&h, &policy, &[short.as_str(), unreadable.as_str()]).await?,
    )
    .map_err(|_| anyhow::anyhow!("two polls ran"))?;
    assert_eq!(
        again.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "still hourly. error: {:?}",
        again.error
    );
    assert!(again.alert, "the alert repeats until a person acts");
    assert_eq!(
        charge_updated_at(&h.pool, &short).await?,
        escalated_at,
        "the second poll must not rewrite an already-escalated charge"
    );
    assert_eq!(charge(&h.pool, &short).await?.state, "unresolved");
    assert_eq!(
        status_queries(&h.pool, &short).await?,
        2,
        "every hourly poll asks Notch Pay again: a late answer arrives that way"
    );

    let queued = jobs(&h.pool).await?;
    let poll = queued
        .iter()
        .find(|job| job.dedupe_key == format!("poll:{short}"))
        .context("the escalated charge's poll job was deleted")?;
    assert!(
        poll.seconds_until_runnable.is_some(),
        "an escalated charge is never parked: it can still settle"
    );
    assert_one_reference(&h.pool, &short, SHORT_COLLECTION_REF).await;
    Ok(())
}

/// An escalated short collection still settles when the rail's answer
/// changes: the escalation is not a verdict.
///
/// The stub's short read cannot change on its own, so a higher-priority
/// mapping is added through WireMock's admin API after the escalation: the
/// same payment, now `complete` with `amounts.total` equal to the charge's
/// amount. The next hourly poll must settle the charge `succeeded` through
/// the ordinary settlement — the path
/// `a_late_success_past_the_horizon_still_settles` pins for the horizon.
#[tokio::test]
async fn a_short_collection_the_worker_escalated_still_settles_on_a_later_answer()
-> anyhow::Result<()> {
    let h = notchpay_harness().await?;
    let policy = RecoveryPolicy::default();
    let short = accepted_notchpay_charge(&h, SHORT_COLLECTION_REF).await?;
    let intent = intent_of(&h.pool, &short).await?;

    let escalated = h.step(&policy).await?;
    assert_eq!(
        escalated.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "error: {:?}",
        escalated.error
    );
    assert_eq!(charge(&h.pool, &short).await?.state, "unresolved");

    support::ensure_crypto_provider_installed();
    let origin = h
        .notchpay_origin
        .as_deref()
        .context("the Notch Pay harness records its origin")?;
    let reference = format!("trx.test_{SHORT_COLLECTION_REF}");
    let added = reqwest::Client::new()
        .post(format!("{origin}/__admin/mappings"))
        .json(&json!({
            "priority": 0,
            "request": { "method": "GET", "urlPath": format!("/payments/{reference}") },
            "response": {
                "status": 200,
                "jsonBody": {
                    "code": 200,
                    "transaction": {
                        "amount": AMOUNT,
                        "amounts": { "total": AMOUNT },
                        "currency": CURRENCY,
                        "reference": reference,
                        "status": "complete",
                        "trxref": SHORT_COLLECTION_REF.to_string(),
                    }
                }
            }
        }))
        .send()
        .await
        .context("adding the later answer to the stub")?;
    anyhow::ensure!(
        added.status().is_success(),
        "WireMock refused the mapping: {}",
        added.status()
    );

    make_every_job_runnable(&h.pool).await?;
    let settled = h.step(&policy).await?;
    assert_eq!(
        settled.disposition,
        Disposition::Finished,
        "a terminal answer settles an escalated charge and ends its job. error: {:?}",
        settled.error
    );
    assert!(!settled.alert, "a payment that landed pages nobody");
    let stored = charge(&h.pool, &short).await?;
    assert_eq!(stored.state, "succeeded");
    assert_eq!(
        stored.provider_txn_id.as_deref(),
        Some(reference.as_str()),
        "Notch Pay's answers carry no `id`, so its reference is the transaction id"
    );
    assert_eq!(intent_status(&h.pool, &intent).await?.0, "succeeded");
    assert_eq!(amount_received(&h.pool, &intent).await?, AMOUNT);
    assert_one_reference(&h.pool, &short, SHORT_COLLECTION_REF).await;
    Ok(())
}

// ---------------------------------------- a two-step rail's crash windows (ADR-0032)
//
// Notch Pay is two calls: initialize, which creates the payment and issues the
// `trx.…` reference Notch Pay alone can find it by, and process, which prompts
// the payer. The port splits them into `prepare` and `submit`, and the core
// commits what `prepare` returned before it writes the `submit` row. Every
// crash between the two therefore leaves one of three states, and each case
// below stages one and asserts the recovery reads it correctly:
//
// | state after the crash | what recovery knows | case |
// |---|---|---|
// | `prepare` row, no `trx.…`, no `submit` row | nobody was prompted | the lost prepare |
// | `trx.…` held, no `submit` row | nobody was prompted, and the payment can be found | the held reference, prompt never sent |
// | `trx.…` held, unanswered `submit` row | the payer may have been prompted, and the payment can be read | the held reference, prompt answer lost |
//
// and one state no build after ADR-0032 can make, which is the one before it:
// an unanswered `submit` row and no `trx.…` — a one-call build's initialize
// whose answer was lost. Those stay `NeedsReconciliation`, by both of the
// paths that can reach them.
//
// The stub journal is part of every assertion. "No process call" is what
// "never prompted" means on the wire, and it is the claim ADR-0032 makes.

/// A reference no `wiremock/notchpay` mapping singles out, so initialize
/// answers `201`, process `202`, and a read of `trx.test_{it}` `complete` for
/// [`AMOUNT`].
const TWO_STEP_REF: Uuid = Uuid::from_u128(0x2a01);

/// A Notch Pay charge a crashed confirm left in `submitting` with no attempt
/// row and no key material, aged past the recovery window, with its poll job
/// due. Returns the intent and the charge.
async fn crashed_notchpay_charge(h: &Harness, reference: Uuid) -> anyhow::Result<(String, String)> {
    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        NOTCHPAY,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id = crashed_charge(
        h.repositories.as_ref(),
        &intent,
        NOTCHPAY,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    age_the_crash(&h.pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    Ok((intent, charge_id))
}

/// Commits `trx.test_{reference}` on a `submitting` charge through
/// `Charges::record_prepared`, the statement a `prepare` that came back runs.
async fn hold_the_notchpay_reference(
    h: &Harness,
    charge_id: &str,
    reference: Uuid,
) -> anyhow::Result<()> {
    h.repositories
        .record_prepared(
            charge_id,
            &json!({ "notchpay_reference": format!("trx.test_{reference}") }),
        )
        .await
        .context("committing the prepared reference")?;
    Ok(())
}

/// How many requests the Notch Pay stub recorded matching a WireMock request
/// pattern.
async fn notchpay_requests(h: &Harness, pattern: serde_json::Value) -> anyhow::Result<u64> {
    support::ensure_crypto_provider_installed();
    let origin = h
        .notchpay_origin
        .as_deref()
        .context("the Notch Pay harness records its origin")?;
    let counted: serde_json::Value = reqwest::Client::new()
        .post(format!("{origin}/__admin/requests/count"))
        .json(&pattern)
        .send()
        .await
        .context("the stub's admin API answers")?
        .json()
        .await
        .context("the count is JSON")?;
    counted
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .context("the admin answer carries a count")
}

/// Initializes, process calls and reads the stub received, in that order.
async fn notchpay_calls(h: &Harness) -> anyhow::Result<(u64, u64, u64)> {
    Ok((
        notchpay_requests(h, json!({ "method": "POST", "url": "/payments" })).await?,
        notchpay_requests(
            h,
            json!({ "method": "POST", "urlPathPattern": "/payments/trx\\..+" }),
        )
        .await?,
        notchpay_requests(
            h,
            json!({ "method": "GET", "urlPathPattern": "/payments/trx\\..+" }),
        )
        .await?,
    ))
}

/// A crash between `prepare` and the commit of its answer — the initialize
/// reached Notch Pay, created the payment, and the `trx.…` it answered with
/// was lost — **fails the charge** on the retry, and nothing ever prompts
/// the payer.
///
/// Staged as that crash leaves it: an unanswered `prepare` row and no
/// `submit` row. The resubmit's `prepare` meets Notch Pay's `409` for the
/// same reference. Before ADR-0032 that was `NeedsReconciliation` and an
/// operator, because a one-call submit's initialize also prompted, so the
/// payer might have paid a payment nobody could read. Now the absence of a
/// `submit` row proves nothing ever asked Notch Pay to prompt, so the payment
/// it holds was never shown to anyone: the charge fails with
/// `provider_unavailable`, the intent goes back to the payer to try again on
/// a new charge, and the orphaned payment expires on Notch Pay unprocessed.
///
/// The journal is the proof the payer was never asked: one initialize (the
/// retry's — the lost one is staged, not sent) and **no** process call.
#[tokio::test]
async fn a_prepare_whose_answer_was_lost_fails_the_charge_and_never_prompts() -> anyhow::Result<()>
{
    let h = notchpay_harness().await?;
    let policy = RecoveryPolicy::default();
    let (intent, charge_id) = crashed_notchpay_charge(&h, NOTCHPAY_DUPLICATE_REF).await?;
    record_attempt(
        h.repositories.as_ref(),
        &charge_id,
        NOTCHPAY,
        "prepare",
        NOTCHPAY_DUPLICATE_REF,
        None,
    )
    .await?;

    let resubmit = run_the_resubmit(&h, &policy, &charge_id).await?;
    assert_eq!(
        resubmit.disposition,
        Disposition::Finished,
        "a `Rejected` is `Retry::NewAttempt`: the job is done. error: {:?}",
        resubmit.error
    );
    assert!(
        !resubmit.alert,
        "a charge nobody was asked to pay pages nobody"
    );

    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(stored.state, "failed");
    assert_eq!(
        stored.failure_code.as_deref(),
        Some(vpay_core::FailureCode::ProviderUnavailable.as_str())
    );
    assert_eq!(
        intent_status(&h.pool, &intent).await?,
        (
            "requires_payment_method".to_owned(),
            Some(
                vpay_core::FailureCode::ProviderUnavailable
                    .as_str()
                    .to_owned()
            )
        ),
        "the intent goes back to the payer, who can try again on a new charge"
    );
    assert_eq!(
        event_types(&h.pool, &intent).await?,
        vec!["payment_intent.payment_failed".to_owned()]
    );
    assert_eq!(
        attempts_of(&h.pool, &charge_id, "prepare").await?,
        vec![
            StoredAttempt {
                status_code: None,
                answered: false,
                error_kind: None,
            },
            StoredAttempt {
                status_code: Some(ANSWERED_SENTINEL),
                answered: true,
                error_kind: Some("charge_declined".to_owned()),
            },
        ],
        "the lost prepare, and the retry Notch Pay answered 409 to"
    );
    assert!(
        attempts_of(&h.pool, &charge_id, "submit").await?.is_empty(),
        "no submit row: nothing was asked to prompt"
    );

    // The poll job the first step rescheduled finds a terminal charge.
    make_every_job_runnable(&h.pool).await?;
    let leftover = h.step(&policy).await?;
    assert_eq!(leftover.kind, "poll_charge");
    assert_eq!(leftover.disposition, Disposition::Finished);
    assert!(jobs(&h.pool).await?.is_empty());
    assert_eq!(status_queries(&h.pool, &charge_id).await?, 0);

    assert_eq!(
        notchpay_calls(&h).await?,
        (1, 0, 0),
        "one initialize, and no process call: the payer was never prompted"
    );
    assert_one_reference(&h.pool, &charge_id, NOTCHPAY_DUPLICATE_REF).await;
    Ok(())
}

/// A crash after the reference was committed and before the `submit` row:
/// the charge **holds** `trx.…` and nothing was asked to prompt. The
/// resubmit skips `prepare` — the payment exists, and a second initialize
/// would only meet the `409` — and prompts by the held reference.
#[tokio::test]
async fn a_prepared_charge_whose_prompt_never_left_is_prompted_without_initializing_again()
-> anyhow::Result<()> {
    let h = notchpay_harness().await?;
    let policy = RecoveryPolicy::default();
    let (_intent, charge_id) = crashed_notchpay_charge(&h, TWO_STEP_REF).await?;
    record_attempt(
        h.repositories.as_ref(),
        &charge_id,
        NOTCHPAY,
        "prepare",
        TWO_STEP_REF,
        Some(ANSWERED_SENTINEL),
    )
    .await?;
    hold_the_notchpay_reference(&h, &charge_id, TWO_STEP_REF).await?;

    let resubmit = run_the_resubmit(&h, &policy, &charge_id).await?;
    assert_eq!(
        resubmit.disposition,
        Disposition::Finished,
        "error: {:?}",
        resubmit.error
    );
    assert_eq!(charge(&h.pool, &charge_id).await?.state, "submitted");
    assert_eq!(
        attempts_of(&h.pool, &charge_id, "prepare").await?.len(),
        1,
        "the staged prepare only: the held reference is not prepared again"
    );
    assert_eq!(
        attempts_of(&h.pool, &charge_id, "submit").await?,
        vec![StoredAttempt {
            status_code: Some(ANSWERED_SENTINEL),
            answered: true,
            error_kind: None,
        }]
    );
    assert_eq!(
        notchpay_calls(&h).await?,
        (0, 1, 0),
        "no initialize, one process by the held reference"
    );
    assert_eq!(
        notchpay_requests(
            &h,
            json!({ "method": "POST", "url": format!("/payments/trx.test_{TWO_STEP_REF}") })
        )
        .await?,
        1
    );
    assert_one_reference(&h.pool, &charge_id, TWO_STEP_REF).await;
    Ok(())
}

/// A crash after the prompt was sent: the charge holds `trx.…` beside an
/// unanswered `submit` row. The payer may have been prompted, so this is the
/// state the read decides, as it does for every rail — and on Notch Pay it
/// now can, because the reference the read needs was committed first.
#[tokio::test]
async fn a_prepared_charge_whose_prompt_lost_its_answer_is_read_by_the_reference_it_holds()
-> anyhow::Result<()> {
    let h = notchpay_harness().await?;
    let policy = RecoveryPolicy::default();
    let (intent, charge_id) = crashed_notchpay_charge(&h, TWO_STEP_REF).await?;
    record_attempt(
        h.repositories.as_ref(),
        &charge_id,
        NOTCHPAY,
        "prepare",
        TWO_STEP_REF,
        Some(ANSWERED_SENTINEL),
    )
    .await?;
    hold_the_notchpay_reference(&h, &charge_id, TWO_STEP_REF).await?;
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        NOTCHPAY,
        TWO_STEP_REF,
        None,
    )
    .await?;

    let settled = h.step(&policy).await?;
    assert_eq!(settled.kind, "poll_charge");
    assert_eq!(
        settled.disposition,
        Disposition::Finished,
        "error: {:?}",
        settled.error
    );
    assert_eq!(charge(&h.pool, &charge_id).await?.state, "succeeded");
    assert_eq!(intent_status(&h.pool, &intent).await?.0, "succeeded");
    assert_eq!(
        notchpay_calls(&h).await?,
        (0, 0, 1),
        "one read, by the held reference; nothing sent again"
    );
    assert_one_reference(&h.pool, &charge_id, TWO_STEP_REF).await;
    Ok(())
}

/// A Notch Pay charge with nothing sent at all goes through both calls, in
/// order: `prepare`, the reference committed, then the prompt.
///
/// This is also every charge a one-call build left at kill point 1, before
/// ADR-0032 shipped: no attempt row means nothing was sent, under either
/// build, so it needs nothing special.
#[tokio::test]
async fn a_never_sent_two_step_charge_is_prepared_committed_and_then_prompted() -> anyhow::Result<()>
{
    let h = notchpay_harness().await?;
    let policy = RecoveryPolicy::default();
    let (_intent, charge_id) = crashed_notchpay_charge(&h, TWO_STEP_REF).await?;

    let resubmit = run_the_resubmit(&h, &policy, &charge_id).await?;
    assert_eq!(
        resubmit.disposition,
        Disposition::Finished,
        "error: {:?}",
        resubmit.error
    );
    let stored = charge(&h.pool, &charge_id).await?;
    assert_eq!(stored.state, "submitted");
    let held: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT provider_ref_extra FROM charges WHERE id = $1")
            .bind(&charge_id)
            .fetch_one(&h.pool)
            .await?;
    assert_eq!(
        held,
        Some(json!({ "notchpay_reference": format!("trx.test_{TWO_STEP_REF}") }))
    );
    let operations: Vec<String> = sqlx::query_scalar(
        "SELECT operation FROM provider_requests WHERE charge_id = $1 ORDER BY id",
    )
    .bind(&charge_id)
    .fetch_all(&h.pool)
    .await?;
    assert_eq!(
        operations,
        vec!["prepare".to_owned(), "submit".to_owned()],
        "the prepare row, then the submit row, written only once the reference was committed"
    );
    assert_eq!(notchpay_calls(&h).await?, (1, 1, 0));
    assert_one_reference(&h.pool, &charge_id, TWO_STEP_REF).await;
    Ok(())
}

/// A charge a **one-call** build left behind — an initialize sent under a
/// `submit` row whose answer was lost, so the charge holds no `trx.…` — stays
/// an operator's, reached through its resubmit.
///
/// The resubmit job is the one a one-call build's ladder enqueued, re-run
/// after the deploy. Its `prepare` is told a submit was recorded for this
/// charge ([`vpay_provider::PriorSubmits::Some`]): under a one-call build
/// that initialize also prompted, so the payer may have paid the payment
/// Notch Pay holds, and nobody can read it. The `409` is
/// `NeedsReconciliation`, and the charge is escalated at once and alerted,
/// as vaam-apps/vpay#313 left it. It is never failed, never processed, and
/// never initialized again on the re-run.
///
/// The poll job is pushed out of the way so the resubmit is what runs; the
/// next case covers the poll.
#[tokio::test]
async fn a_one_call_charge_whose_resubmit_lost_its_answer_is_escalated_and_never_prompted()
-> anyhow::Result<()> {
    let h = notchpay_harness().await?;
    let policy = RecoveryPolicy::default();
    let (intent, charge_id) = crashed_notchpay_charge(&h, NOTCHPAY_DUPLICATE_REF).await?;
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        NOTCHPAY,
        NOTCHPAY_DUPLICATE_REF,
        None,
    )
    .await?;
    let pushed =
        sqlx::query("UPDATE jobs SET run_at = now() + INTERVAL '1 hour' WHERE dedupe_key = $1")
            .bind(vpay_worker::jobs::poll_dedupe_key(&charge_id))
            .execute(&h.pool)
            .await?
            .rows_affected();
    anyhow::ensure!(pushed == 1, "the poll job was not there to push back");
    let charge_for_job = charge_id.clone();
    h.repositories
        .transaction(|tx| {
            Box::pin(async move {
                let enqueued = tx
                    .enqueue_in_tx(
                        "resubmit_charge",
                        &vpay_worker::jobs::resubmit_dedupe_key(&charge_for_job),
                        &json!({ "charge_id": charge_for_job }),
                        Duration::ZERO,
                    )
                    .await?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit(enqueued))
            })
        })
        .await?;

    let resubmit = h.step(&policy).await?;
    assert_eq!(resubmit.kind, "resubmit_charge");
    assert_eq!(
        resubmit.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "escalated, not failed and not on the ladder. error: {:?}",
        resubmit.error
    );
    assert!(resubmit.alert, "and a person is told now");
    assert_eq!(charge(&h.pool, &charge_id).await?.state, "unresolved");
    assert_eq!(
        intent_status(&h.pool, &intent).await?,
        ("requires_payment_method".to_owned(), None),
        "an escalation stamps no error on the intent"
    );
    assert_eq!(
        attempts_of(&h.pool, &charge_id, "prepare").await?,
        vec![StoredAttempt {
            status_code: None,
            answered: false,
            error_kind: Some("charge_needs_reconciliation".to_owned()),
        }],
        "unanswered: what the rail holds is unknown"
    );

    // The resubmit's hourly re-run and the poll: neither sends anything.
    make_every_job_runnable(&h.pool).await?;
    let mut kinds = BTreeMap::new();
    for _ in 0..2 {
        let step = h.step(&policy).await?;
        kinds.insert(step.kind.clone(), step);
    }
    assert_eq!(
        kinds
            .get("resubmit_charge")
            .context("the resubmit's re-run did not run")?
            .disposition,
        Disposition::Finished,
        "the re-run finds the charge outside `submitting`"
    );
    assert_eq!(
        kinds
            .get("poll_charge")
            .context("the poll did not run")?
            .disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600))
    );
    assert_eq!(charge(&h.pool, &charge_id).await?.state, "unresolved");
    assert_eq!(attempts_of(&h.pool, &charge_id, "prepare").await?.len(), 1);
    assert_eq!(
        notchpay_calls(&h).await?,
        (1, 0, 0),
        "one initialize, answered 409; nothing processed, and nothing to read by"
    );
    assert_one_reference(&h.pool, &charge_id, NOTCHPAY_DUPLICATE_REF).await;
    Ok(())
}

/// The same one-call charge reached by its **poll**, which is how most of
/// them will be: the unanswered `submit` row reads `SubmitAttempt::Unanswered`,
/// recovery asks the rail, and the adapter cannot read a charge with no
/// `trx.…` — `NeedsReconciliation`, escalated at once (vaam-apps/vpay#313).
/// ADR-0032 changes nothing here; the case pins that it did not.
#[tokio::test]
async fn a_one_call_charge_whose_submit_lost_its_answer_is_escalated_by_its_poll()
-> anyhow::Result<()> {
    let h = notchpay_harness().await?;
    let policy = RecoveryPolicy::default();
    let (_intent, charge_id) = crashed_notchpay_charge(&h, TWO_STEP_REF).await?;
    record_submit_attempt(
        h.repositories.as_ref(),
        &charge_id,
        NOTCHPAY,
        TWO_STEP_REF,
        None,
    )
    .await?;

    let poll = h.step(&policy).await?;
    assert_eq!(poll.kind, "poll_charge");
    assert_eq!(
        poll.disposition,
        Disposition::Rescheduled(Duration::from_secs(3_600)),
        "error: {:?}",
        poll.error
    );
    assert!(poll.alert);
    assert_eq!(charge(&h.pool, &charge_id).await?.state, "unresolved");
    assert!(
        attempts_of(&h.pool, &charge_id, "prepare")
            .await?
            .is_empty(),
        "nothing is prepared for a charge a submit may have prompted"
    );
    assert!(
        !jobs(&h.pool)
            .await?
            .iter()
            .any(|job| job.kind == "resubmit_charge"),
        "and nothing is resubmitted"
    );
    assert_eq!(
        notchpay_calls(&h).await?,
        (0, 0, 0),
        "the adapter refuses the read before any request: there is no `trx.…` to read by"
    );
    Ok(())
}

/// Parks the six singleton jobs so nothing but the loop's own reaper can
/// free a lease.
///
/// `seed_singletons` is `ON CONFLICT (dedupe_key) DO NOTHING`, so seeding them
/// here and then moving them to `run_at = 'infinity'` means `run_loop`'s own
/// seed writes nothing and `sweep_expired` can never be claimed. That is not
/// an artificial state: it is precisely the deadlock F2 named — a worker that
/// dies holding `sweep:expired` leaves the only reaper unclaimable, and if the
/// sweep were the only reaper nothing would ever recover it.
///
/// The key list and the count are written out rather than derived, so a
/// singleton added to `seed_singletons` without being parked here fails this
/// helper instead of silently running inside a test whose subject is
/// something else.
///
/// **That claim was not true until 2026-09-06 and now is.** S4a added a fifth
/// singleton (`sweep:customers`), and this helper reported success: the
/// `UPDATE` matched exactly the four keys it named, so `parked == 4` held
/// while a fifth job sat claimable. What actually failed was
/// `a_lease_stranded_by_a_crash_is_freed_at_boot_before_any_sweep_runs`'s own
/// "the fixture must leave the job genuinely unclaimable" assertion — one of
/// this helper's three call sites, by luck rather than by design. The second
/// `ensure` below closes it: it counts the rows that are *still claimable*
/// rather than the rows this statement moved, so an unparked singleton fails
/// here, in the helper whose doc promises it. (It did, on 2026-10-22, when
/// RFC-0013 added the sixth, `ensure:callbacks`.)
async fn park_the_housekeeping_jobs(
    repositories: &dyn Repositories,
    pool: &PgPool,
) -> anyhow::Result<()> {
    vpay_worker::seed_singletons(repositories)
        .await
        .context("seeding the singletons")?;
    let parked = sqlx::query(
        "UPDATE jobs SET run_at = 'infinity'::TIMESTAMPTZ \
         WHERE dedupe_key IN ('sweep:expired', 'scan:live', 'fanout:events', 'scan:deliveries', \
                              'sweep:customers', 'ensure:callbacks')",
    )
    .execute(pool)
    .await
    .context("parking the singletons")?
    .rows_affected();
    anyhow::ensure!(
        parked == 6,
        "expected six singletons to park, parked {parked}"
    );

    // Counted from the table rather than from the statement above, which is
    // the whole difference between this helper keeping its promise and only
    // appearing to. `seed_singletons` has just run and nothing else in these
    // tests has enqueued anything yet, so every claimable row here is a
    // singleton this list failed to name.
    let unparked: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE run_at <> 'infinity'::TIMESTAMPTZ")
            .fetch_one(pool)
            .await
            .context("counting the singletons this helper did not park")?;
    anyhow::ensure!(
        unparked == 0,
        "seed_singletons seeded a job this helper does not name; {unparked} row(s) are still \
         claimable, so whichever test called this is about to race a housekeeping job it did \
         not intend to run"
    );
    Ok(())
}

/// Hands a charge's poll job to a worker that never comes back, in one
/// statement.
///
/// One statement so there is no window in which a running loop could claim
/// the row between the enqueue and the lease. `age` is how long ago the lease
/// was taken; the reaper compares it against `RecoveryPolicy::lease`.
async fn strand_the_poll_job(pool: &PgPool, charge_id: &str, age: Duration) -> anyhow::Result<()> {
    let stranded = sqlx::query(
        "UPDATE jobs \
         SET locked_at = now() - ($2::BIGINT * INTERVAL '1 second'), \
             locked_by = 'a-worker-that-was-sigkilled', \
             attempts = attempts + 1 \
         WHERE dedupe_key = $1",
    )
    .bind(vpay_worker::jobs::poll_dedupe_key(charge_id))
    .bind(i64::try_from(age.as_secs()).expect("a test's lease age fits in an i64"))
    .execute(pool)
    .await
    .context("stranding the poll job")?
    .rows_affected();
    anyhow::ensure!(stranded == 1, "the poll job was not there to strand");
    Ok(())
}

/// A charge already past its recovery branch, so a single poll settles it
/// against the catch-all `SUCCESSFUL` stub.
///
/// The reaping cases are about *whether a job is ever claimed*, so the job
/// itself has to resolve in one claim; a charge whose recovery table said
/// "resubmit" would need two and would not distinguish "reaped late" from
/// "still on the ladder".
async fn charge_that_settles_in_one_poll(
    repositories: &dyn Repositories,
    pool: &PgPool,
    intent: &str,
) -> anyhow::Result<String> {
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        repositories,
        intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(pool, &charge_id, RECOVERABLE_CRASH_AGE).await?;
    record_submit_attempt(
        repositories,
        &charge_id,
        PUSH_RAIL,
        reference,
        Some(ANSWERED_SENTINEL),
    )
    .await?;
    Ok(charge_id)
}

/// Waits until the charge's poll job has been claimed **and** settled by a
/// running loop.
///
/// `attempts` is incremented by `jobs::claim` and `locked_by` is cleared by
/// the write that ends the lease, so the pair says "a worker took this job and
/// is finished with it" — which is the ordering the race case needs before it
/// may run the confirm's write. Polling the row rather than sleeping a fixed
/// interval is what keeps that ordering true on a loaded machine, where the
/// window this whole guard is about was measured at 3.7 seconds.
async fn wait_for_the_poll_job_to_run(
    pool: &PgPool,
    charge_id: &str,
    within: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + within;
    loop {
        let row: Option<(i32, Option<String>)> =
            sqlx::query_as("SELECT attempts, locked_by FROM jobs WHERE dedupe_key = $1")
                .bind(vpay_worker::jobs::poll_dedupe_key(charge_id))
                .fetch_optional(pool)
                .await
                .context("reading the poll job")?;
        match row {
            Some((attempts, None)) if attempts >= 1 => return Ok(()),
            // The job is gone: the handler answered `Outcome::Done`, which for
            // a `submitting` charge means it resolved it. That is the failure
            // this case is about, and the assertions below name it.
            None => return Ok(()),
            _ => {}
        }
        if Instant::now() >= deadline {
            let queued = jobs(pool).await.unwrap_or_default();
            anyhow::bail!(
                "no worker claimed and settled the poll job within {within:?}; the race                  this case stages never happened. jobs: {queued:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits for a charge to reach `succeeded`, or says what the queue looked like
/// when it gave up.
async fn wait_for_settlement(
    pool: &PgPool,
    charge_id: &str,
    within: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + within;
    loop {
        if charge(pool, charge_id).await?.state == "succeeded" {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let queued = jobs(pool).await.unwrap_or_default();
            anyhow::bail!(
                "the charge was still not settled after {within:?}; nothing claimed its \
                 poll job. jobs: {queued:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A worker that boots after a crash frees the leases the dead process held,
/// before it does anything else.
///
/// `claim`'s predicate is `locked_at IS NULL` — that exactness is what makes
/// `jobs_claimable_idx` a usable index — so a job stranded by a SIGKILL is
/// unclaimable until something reaps it. When the only reaper was the hourly
/// `sweep_expired`, a restarted deployment could leave live charges undriven
/// for up to an hour; and, as this case stages, if the dead worker was holding
/// `sweep:expired` itself, forever.
///
/// The lease here is a minute and the strand two minutes old, so the loop's
/// periodic reaper (half a lease — thirty seconds) cannot be what frees it
/// inside the deadline below. Only the reap that runs *at boot* can.
#[tokio::test]
async fn a_lease_stranded_by_a_crash_is_freed_at_boot_before_any_sweep_runs() -> anyhow::Result<()>
{
    let h = harness().await?;
    let policy = RecoveryPolicy {
        lease: Duration::from_secs(60),
        ..RecoveryPolicy::default()
    };

    park_the_housekeeping_jobs(h.repositories.as_ref(), &h.pool).await?;
    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id =
        charge_that_settles_in_one_poll(h.repositories.as_ref(), &h.pool, &intent).await?;
    strand_the_poll_job(&h.pool, &charge_id, Duration::from_secs(120)).await?;

    assert!(
        Jobs::claim(h.repositories.as_ref(), "a-live-worker")
            .await?
            .is_none(),
        "the fixture must leave the job genuinely unclaimable, or this case proves nothing"
    );

    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let repositories = Arc::clone(&h.repositories);
    let adapters = Arc::clone(&h.adapters);
    let rails = Arc::clone(&h.rails);
    let worker = tokio::spawn(async move {
        vpay_worker::run_loop(
            repositories,
            adapters,
            rails,
            policy,
            support::no_webhook_endpoints(),
            support::default_egress_policy(),
            std::sync::Arc::new(vpay_worker::CallbackPlan::none()),
            1,
            Duration::from_secs(10),
            "worker-recovery-suite".to_owned(),
            async move {
                let _ = stopped.await;
            },
        )
        .await
    });

    // Ten seconds is a container-scheduling margin, not an expectation: the
    // boot reap runs before the first claim, so the settlement lands in
    // milliseconds. Thirty would also pass with the periodic reaper alone,
    // which is why it is ten.
    let settled = wait_for_settlement(&h.pool, &charge_id, Duration::from_secs(10)).await;
    let _ = stop.send(());
    let report = worker.await.context("the worker task panicked")?;
    settled?;

    assert!(
        report.claimed >= 1,
        "the loop never claimed anything, so the lease was never freed"
    );
    assert_eq!(
        intent_status(&h.pool, &intent).await?.0,
        "succeeded",
        "the stranded charge was driven all the way, not merely unlocked"
    );
    Ok(())
}

/// …and it keeps reaping while it runs, on its own timer.
///
/// The boot reap cannot cover a worker that dies while *this* one is up, and
/// `sweep_expired` is hourly — and, as staged here, may itself be unclaimable.
/// So the loop runs the reaper on a `tokio::interval` of half a lease.
///
/// The lease is three seconds and the strand is taken at `now()`, so at boot
/// it is not expired and the boot reap cannot be what frees it: only a pass
/// that happens **after** the lease matures can. That is the whole
/// distinction between this case and the one above.
#[tokio::test]
async fn a_lease_that_expires_while_the_worker_runs_is_reaped_on_its_own_timer()
-> anyhow::Result<()> {
    let h = harness().await?;
    const LEASE: Duration = Duration::from_secs(3);
    let policy = RecoveryPolicy {
        lease: LEASE,
        ..RecoveryPolicy::default()
    };

    park_the_housekeeping_jobs(h.repositories.as_ref(), &h.pool).await?;
    let intent = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let charge_id =
        charge_that_settles_in_one_poll(h.repositories.as_ref(), &h.pool, &intent).await?;
    // Taken *now*: at boot this lease has zero age, so it is younger than the
    // lease and the boot reap must leave it alone.
    strand_the_poll_job(&h.pool, &charge_id, Duration::ZERO).await?;

    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let repositories = Arc::clone(&h.repositories);
    let adapters = Arc::clone(&h.adapters);
    let rails = Arc::clone(&h.rails);
    let worker = tokio::spawn(async move {
        vpay_worker::run_loop(
            repositories,
            adapters,
            rails,
            policy,
            support::no_webhook_endpoints(),
            support::default_egress_policy(),
            std::sync::Arc::new(vpay_worker::CallbackPlan::none()),
            1,
            Duration::from_secs(10),
            "worker-recovery-suite".to_owned(),
            async move {
                let _ = stopped.await;
            },
        )
        .await
    });

    // The reaper ticks every 1.5 s and can only free a lease older than 3 s,
    // so the earliest possible reap is at 3 s. Fifteen is that plus a wide
    // container margin — and far short of the hour `sweep_expired` would
    // take, which is the number this case is really distinguishing itself
    // from.
    let settled = wait_for_settlement(&h.pool, &charge_id, Duration::from_secs(15)).await;
    let _ = stop.send(());
    worker.await.context("the worker task panicked")?;
    settled?;

    let leftover: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE locked_by = 'a-worker-that-was-sigkilled'",
    )
    .fetch_one(&h.pool)
    .await
    .context("counting leases still held by the dead worker")?;
    assert_eq!(
        leftover, 0,
        "the dead worker's lease is still held, so the row was never reaped"
    );
    Ok(())
}

/// A job whose row this build cannot interpret is parked, not retried forever.
///
/// `JobError::Poisoned` is `Retry::Never` — re-running cannot fix data that is
/// already wrong — and `Decision::DeadLetter` is the only thing that reaches
/// `vpay_db::jobs::dead_letter`. The assertion that matters is the shape of
/// the park: `run_at = 'infinity'` with the lease **cleared**, so neither the
/// claim (`run_at <= now()`) nor the lease reaper (`locked_at`) can resurrect
/// it, and the `dedupe_key` stays occupied so the backstop scan cannot
/// re-create the same work every ten minutes.
#[tokio::test]
async fn a_poisoned_job_is_parked_with_its_lease_cleared_and_its_reason_recorded()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    // A poll job naming a charge that does not exist. That is precisely
    // `Poisoned`: the row is wrong, and no rail, no retry and no amount of
    // waiting changes it. Due at once on the database's clock — the one the
    // claim inside `step` compares against; since 2026-09-23 (ADR-0026) the
    // enqueue takes a delay from that clock and no instant at all.
    h.repositories
        .transaction(|tx| {
            Box::pin(async move {
                tx.enqueue_in_tx(
                    "poll_charge",
                    "poll:ch_does_not_exist",
                    &serde_json::json!({ "charge_id": "ch_does_not_exist" }),
                    std::time::Duration::ZERO,
                )
                .await?;
                Ok::<_, anyhow::Error>(TxOutcome::Commit(()))
            })
        })
        .await?;

    let settled = h.step(&policy).await?;
    assert_eq!(settled.disposition, Disposition::DeadLettered);
    assert!(
        settled.alert,
        "a parked job is work nothing will ever do again; a human has to be told"
    );

    let queued = jobs(&h.pool).await?;
    let parked = queued
        .iter()
        .find(|job| job.dedupe_key == "poll:ch_does_not_exist")
        .context(
            "the poisoned job was deleted rather than parked, so nothing records why \
                  a charge stopped being driven",
        )?;
    assert!(
        parked.seconds_until_runnable.is_none(),
        "a parked job must sit at run_at = 'infinity', which no claim can reach"
    );
    assert!(
        parked.locked_by.is_none(),
        "the lease must be cleared, or the reaper frees the row and the failure repeats"
    );
    assert!(
        parked
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("poisoned")),
        "the park must record its reason: got {:?}",
        parked.last_error
    );
    assert_eq!(parked.attempts, 1, "it was tried exactly once");

    // And it stays parked: a second pass finds nothing to claim.
    assert!(
        vpay_worker::run_once(
            h.repositories.as_ref(),
            &h.adapters,
            &h.rails,
            &policy,
            &vpay_worker::WebhookContext {
                endpoints: &support::no_webhook_endpoints(),
                egress: support::default_egress_policy(),
            },
            &vpay_worker::CallbackPlan::none(),
            "worker-recovery-suite"
        )
        .await?
        .is_none(),
        "a parked job was claimable again"
    );
    Ok(())
}

/// The six singletons — the housekeeping sweep, the charge backstop, the
/// outbox drain, the delivery backstop, the twelve-month customer retention
/// sweep and the callback registration pass — seeded once whatever the
/// concurrency, and rescheduled rather than deleted.
///
/// Five until 2026-10-22, when RFC-0013 added `ensure:callbacks`.
///
/// Four until 2026-09-06, when S4a added `sweep:customers`. That one is worth
/// naming separately in both assertions below rather than being folded into a
/// count, because its absence is the one that is **invisible**: a deployment
/// that dropped this seed keeps every customer for ever, which looks exactly
/// like a healthy deployment and is a promise to a payer that vpay has
/// stopped keeping.
///
/// `ON CONFLICT (dedupe_key) DO NOTHING` is what makes N workers booting
/// against one database produce one row each rather than N, and it is a
/// property of the unique index, not of whichever enqueue happened to run
/// first.
#[tokio::test]
async fn the_housekeeping_jobs_are_seeded_once_and_reschedule_themselves() -> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    // Three "workers" booting at once.
    for _ in 0..3 {
        vpay_worker::seed_singletons(h.repositories.as_ref()).await?;
    }
    let queued = jobs(&h.pool).await?;
    assert_eq!(
        queued
            .iter()
            .map(|job| job.dedupe_key.as_str())
            .collect::<Vec<_>>(),
        vec![
            "ensure:callbacks",
            "fanout:events",
            "scan:deliveries",
            "scan:live",
            "sweep:customers",
            "sweep:expired"
        ],
        "three boots must leave exactly six rows"
    );

    // No `make_every_job_runnable` here, and its absence is the assertion.
    // Until 2026-09-23 `seed_singletons` stamped `run_at` from this host's
    // clock while the claim compared it with the database's `now()`, so a
    // Postgres container even milliseconds behind this host left every
    // freshly seeded row not yet due and the first `step` below found
    // nothing; #254 put that call here to move the seeds onto the database's
    // clock (`docs/status/verification/2026-09-23-test-clock-skew.md`). Since
    // ADR-0026 the shipping seed is due at the database's own `now()`, so the
    // steps below claim what `seed_singletons` wrote exactly as it wrote it,
    // with no retry, and neither the seed nor the claim reads this host's
    // clock. The first half of that is checked here rather than assumed:
    // each seed's `run_at` is its own `created_at` to the microsecond, and
    // `created_at` is the column's `DEFAULT now()` — the one clock a
    // host-clock instant could not land on exactly.
    let off_the_database_clock: Vec<String> = sqlx::query_scalar(
        "SELECT dedupe_key FROM jobs WHERE run_at <> created_at ORDER BY dedupe_key",
    )
    .fetch_all(&h.pool)
    .await?;
    assert!(
        off_the_database_clock.is_empty(),
        "every singleton is seeded due at the database's now(), not at an instant read \
         elsewhere: {off_the_database_clock:?}"
    );

    // Both run, and both go back on the clock. A sweep that finished would be
    // a deployment that swept once and never again — which is the bug
    // `vpay-server`'s boot-time sweep actually was.
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..6 {
        let settled = h.step(&policy).await?;
        assert!(
            matches!(settled.disposition, Disposition::Rescheduled(_)),
            "{} finished instead of rescheduling: {:?}",
            settled.kind,
            settled.disposition
        );
        seen.push(settled.kind);
    }
    seen.sort();
    assert_eq!(
        seen,
        vec![
            "ensure_callbacks",
            "fan_out_events",
            "scan_deliveries",
            "scan_live_charges",
            "sweep_expired",
            "sweep_idle_customers"
        ],
        "all six singletons must go back on the clock; a fan-out that finished \
         instead is a deployment that drains its outbox once and never again, a \
         delivery backstop that finished is one whose lost delivery jobs are lost for \
         good, a customer sweep that finished is one that keeps a payer's personal \
         data for ever while every log line says it ran, and a callback pass that \
         finished is one that never notices an endpoint deleted in a dashboard"
    );
    Ok(())
}

/// The backstop scan re-enqueues a live charge nothing is driving — and only
/// once.
///
/// It is explicitly *not* the mechanism: `insert_charge` enqueues the poll in
/// the same transaction as the charge, so a healthy deployment's scan finds
/// nothing. This covers what that transaction cannot — a row written before
/// the queue existed, and a job lost to operator error.
#[tokio::test]
async fn the_backstop_scan_re_enqueues_an_unattended_charge_and_leaves_attended_ones_alone()
-> anyhow::Result<()> {
    let h = harness().await?;
    let policy = RecoveryPolicy::default();

    let attended = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let attended_charge = crashed_charge(
        h.repositories.as_ref(),
        &attended,
        PUSH_RAIL,
        Uuid::new_v4(),
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &attended_charge, RECOVERABLE_CRASH_AGE).await?;

    let orphan = confirmed_intent(
        h.repositories.as_ref(),
        MERCHANT,
        PUSH_RAIL,
        AMOUNT,
        CURRENCY,
    )
    .await?;
    let orphan_charge = crashed_charge(
        h.repositories.as_ref(),
        &orphan,
        PUSH_RAIL,
        Uuid::new_v4(),
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // The crash is old enough for the recovery table to apply to it at all
    // (`support::age_the_crash`).
    age_the_crash(&h.pool, &orphan_charge, RECOVERABLE_CRASH_AGE).await?;
    // Operator error, or a row from before migration 0021: the charge is live
    // and nothing is polling it.
    sqlx::query("DELETE FROM jobs WHERE dedupe_key = $1")
        .bind(format!("poll:{orphan_charge}"))
        .execute(&h.pool)
        .await?;
    // The scan only considers charges that have been quiet for ten minutes,
    // which is what stops it fighting with a confirm that is still in flight.
    sqlx::query("UPDATE charges SET updated_at = now() - interval '20 minutes' WHERE id = $1")
        .bind(&orphan_charge)
        .execute(&h.pool)
        .await?;

    vpay_worker::seed_singletons(h.repositories.as_ref()).await?;
    // The four singletons and the attended charge's poll job are all
    // runnable; run until the scan has had its turn.
    for _ in 0..6 {
        if h.step(&policy).await?.kind == "scan_live_charges" {
            break;
        }
        make_every_job_runnable(&h.pool).await?;
    }

    let queued = jobs(&h.pool).await?;
    assert!(
        queued
            .iter()
            .any(|job| job.dedupe_key == format!("poll:{orphan_charge}")),
        "the backstop scan left a live charge with nothing driving it; jobs: {queued:?}"
    );
    assert_eq!(
        queued
            .iter()
            .filter(|job| job.dedupe_key == format!("poll:{attended_charge}"))
            .count(),
        1,
        "the scan must never produce a second job for a charge that already has one"
    );
    Ok(())
}

/// Two workers claiming at the same instant take two different jobs.
///
/// This is the one property the whole queue rests on and the only one that
/// cannot be tested without a real database: `SKIP LOCKED` is what turns "the
/// second writer blocks, re-evaluates its predicate, finds the row claimed and
/// matches nothing" into "take the next one". A plain
/// `UPDATE ... WHERE locked_at IS NULL` would pass every test that ran the
/// claims sequentially and silently lose work under load.
#[tokio::test]
async fn two_workers_claiming_together_never_take_the_same_job() -> anyhow::Result<()> {
    let h = harness().await?;

    // Due at once on the database's clock, the one the claims below compare
    // against (ADR-0026: the enqueue takes a delay, never an instant).
    h.repositories
        .transaction(|tx| {
            Box::pin(async move {
                for n in 0..8 {
                    tx.enqueue_in_tx(
                        "poll_charge",
                        &format!("poll:ch_{n}"),
                        &serde_json::json!({ "charge_id": format!("ch_{n}") }),
                        std::time::Duration::ZERO,
                    )
                    .await?;
                }
                Ok::<_, anyhow::Error>(TxOutcome::Commit(()))
            })
        })
        .await?;

    // Eight real tasks on the multi-thread runtime, two worker identities,
    // all racing one queue.
    let mut set = tokio::task::JoinSet::new();
    for n in 0..8 {
        let repositories = Arc::clone(&h.repositories);
        let worker = format!("worker-{}", n % 2);
        set.spawn(async move { Jobs::claim(repositories.as_ref(), &worker).await });
    }

    let mut ids: Vec<Uuid> = Vec::new();
    while let Some(joined) = set.join_next().await {
        let row = joined
            .context("a claim task panicked")?
            .context("a concurrent claim failed")?
            .context("a concurrent claim found nothing while eight jobs were runnable")?;
        ids.push(row.id);
    }

    assert_eq!(ids.len(), 8, "every claim must have taken a job");
    ids.sort();
    let before = ids.len();
    ids.dedup();
    assert_eq!(
        ids.len(),
        before,
        "two workers claimed the same job; SKIP LOCKED is not doing its work and two \
         processes would run one charge's poll at once"
    );
    Ok(())
}
