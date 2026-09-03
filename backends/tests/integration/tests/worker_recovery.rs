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
use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use uuid::Uuid;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, ProviderHost};
use vpay_worker::{Adapters, Disposition, RailConfigs, RecoveryPolicy, Settled};

mod support;

use support::{
    attempted_references, confirmed_intent, crashed_charge, make_every_job_runnable,
    migrated_postgres, rail_configs, reconcile_from_config,
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
    _mtn: ContainerAsync<GenericImage>,
    _orange: ContainerAsync<GenericImage>,
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
        let http = support::webhook_client();
        vpay_worker::run_once(
            &self.pool,
            &self.adapters,
            &self.rails,
            policy,
            &vpay_worker::WebhookContext {
                endpoints: &endpoints,
                http: &http,
            },
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
            },
        ],
        currencies: vec![CurrencyEntry {
            code: CURRENCY.to_owned(),
            exponent: 0,
        }],
        merchant_clients: vec![],
        dashboard_client: None,
    }
}

async fn harness() -> anyhow::Result<Harness> {
    let (postgres, pool) = migrated_postgres().await?;

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
    reconcile_from_config(&pool, &config).await?;

    Ok(Harness {
        _postgres: postgres,
        _mtn: mtn,
        _orange: orange,
        pool,
        adapters: Arc::new(support::adapters_by_code()),
        rails: Arc::new(rail_configs(&config)),
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
    pool: &PgPool,
    charge_id: &str,
    rail: &str,
    reference: Uuid,
    status_code: Option<i32>,
) -> anyhow::Result<()> {
    let attempt_id =
        vpay_db::provider_requests::insert_pending(pool, charge_id, rail, "submit", reference, 1)
            .await
            .context("writing the submit attempt row")?;
    if let Some(code) = status_code {
        vpay_db::provider_requests::record_response(pool, attempt_id, Some(code), None)
            .await
            .context("recording the submit answer")?;
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
        "UPDATE charges SET state = 'unresolved'::charge_state, updated_at = now() \
         WHERE id = $1 AND state = 'submitting'::charge_state",
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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;

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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    record_submit_attempt(&h.pool, &charge_id, PUSH_RAIL, reference, None).await?;

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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    record_submit_attempt(
        &h.pool,
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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        NOT_FOUND_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // Kill point 2: the POST was issued and lost. Without an attempt row the
    // very first poll would resubmit for a different reason
    // (`SubmitAttempt::Never`), and this case would prove nothing about the
    // streak.
    record_submit_attempt(&h.pool, &charge_id, PUSH_RAIL, NOT_FOUND_REF, None).await?;

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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        PENDING_THEN_SUCCESS_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    record_submit_attempt(
        &h.pool,
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

    let intent = confirmed_intent(&h.pool, MERCHANT, REDIRECT_RAIL, AMOUNT, CURRENCY).await?;
    let reference = Uuid::new_v4();
    // No `payer_ref` and no `provider_ref_extra`: a redirect rail names no
    // payer up front, and the `pay_token` is exactly what was lost.
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        REDIRECT_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        None,
    )
    .await?;

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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    assert_eq!(
        intent_status(&h.pool, &intent).await?.0,
        "requires_payment_method",
        "the fixture must stage the state a crashed confirm actually leaves, or this case \
         proves nothing"
    );

    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // Kill point 2: the POST went out and the answer was lost. The rail has
    // the payment; vpay's intent says nothing was attempted.
    record_submit_attempt(&h.pool, &charge_id, PUSH_RAIL, reference, None).await?;

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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        UNAVAILABLE_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // Kill point 3, so the recovery table advances the bookkeeping and the
    // run reaches the point where it would ask the rail — which is the point
    // under test.
    record_submit_attempt(
        &h.pool,
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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let charge_id = charge_that_settles_in_one_poll(&h.pool, &intent).await?;

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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        NOT_FOUND_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // Kill point 2, exactly as the streak case stages it: without an attempt
    // row the very first poll would resubmit for a different reason
    // (`SubmitAttempt::Never`) and this case would never reach the arm it is
    // about.
    record_submit_attempt(&h.pool, &charge_id, PUSH_RAIL, NOT_FOUND_REF, None).await?;

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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // Kill point 2, so the recovery table answers `Poll` and the run reaches
    // the status query — which is where the unparseable row is read.
    record_submit_attempt(&h.pool, &charge_id, PUSH_RAIL, reference, None).await?;
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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        NOT_FOUND_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    // Kill point 3: the rail answered the submit, so the recovery table
    // advances the bookkeeping and the poll runs the ordinary path.
    record_submit_attempt(
        &h.pool,
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

    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let charge_id = crashed_charge(
        &h.pool,
        &intent,
        PUSH_RAIL,
        DECLINED_REF,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
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

/// Parks the four singleton jobs so nothing but the loop's own reaper can
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
async fn park_the_housekeeping_jobs(pool: &PgPool) -> anyhow::Result<()> {
    vpay_worker::seed_singletons(pool)
        .await
        .context("seeding the singletons")?;
    let parked = sqlx::query(
        "UPDATE jobs SET run_at = 'infinity'::TIMESTAMPTZ \
         WHERE dedupe_key IN ('sweep:expired', 'scan:live', 'fanout:events', 'scan:deliveries')",
    )
    .execute(pool)
    .await
    .context("parking the singletons")?
    .rows_affected();
    anyhow::ensure!(
        parked == 4,
        "expected four singletons to park, parked {parked}"
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
async fn charge_that_settles_in_one_poll(pool: &PgPool, intent: &str) -> anyhow::Result<String> {
    let reference = Uuid::new_v4();
    let charge_id = crashed_charge(
        pool,
        intent,
        PUSH_RAIL,
        reference,
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
    record_submit_attempt(
        pool,
        &charge_id,
        PUSH_RAIL,
        reference,
        Some(ANSWERED_SENTINEL),
    )
    .await?;
    Ok(charge_id)
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

    park_the_housekeeping_jobs(&h.pool).await?;
    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let charge_id = charge_that_settles_in_one_poll(&h.pool, &intent).await?;
    strand_the_poll_job(&h.pool, &charge_id, Duration::from_secs(120)).await?;

    assert!(
        vpay_db::jobs::claim(&h.pool, "a-live-worker")
            .await?
            .is_none(),
        "the fixture must leave the job genuinely unclaimable, or this case proves nothing"
    );

    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let pool = h.pool.clone();
    let adapters = Arc::clone(&h.adapters);
    let rails = Arc::clone(&h.rails);
    let worker = tokio::spawn(async move {
        vpay_worker::run_loop(
            &pool,
            adapters,
            rails,
            policy,
            support::no_webhook_endpoints(),
            support::webhook_client(),
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

    park_the_housekeeping_jobs(&h.pool).await?;
    let intent = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let charge_id = charge_that_settles_in_one_poll(&h.pool, &intent).await?;
    // Taken *now*: at boot this lease has zero age, so it is younger than the
    // lease and the boot reap must leave it alone.
    strand_the_poll_job(&h.pool, &charge_id, Duration::ZERO).await?;

    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let pool = h.pool.clone();
    let adapters = Arc::clone(&h.adapters);
    let rails = Arc::clone(&h.rails);
    let worker = tokio::spawn(async move {
        vpay_worker::run_loop(
            &pool,
            adapters,
            rails,
            policy,
            support::no_webhook_endpoints(),
            support::webhook_client(),
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
    // waiting changes it.
    let mut tx = h.pool.begin().await?;
    vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        "poll_charge",
        "poll:ch_does_not_exist",
        &serde_json::json!({ "charge_id": "ch_does_not_exist" }),
        time::OffsetDateTime::now_utc(),
    )
    .await?;
    tx.commit().await?;

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
            &h.pool,
            &h.adapters,
            &h.rails,
            &policy,
            &vpay_worker::WebhookContext {
                endpoints: &support::no_webhook_endpoints(),
                http: &support::webhook_client(),
            },
            "worker-recovery-suite"
        )
        .await?
        .is_none(),
        "a parked job was claimable again"
    );
    Ok(())
}

/// The four singletons — the sweep, the charge backstop, the outbox drain and
/// the delivery backstop — seeded once whatever the concurrency, and
/// rescheduled rather than deleted.
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
        vpay_worker::seed_singletons(&h.pool).await?;
    }
    let queued = jobs(&h.pool).await?;
    assert_eq!(
        queued
            .iter()
            .map(|job| job.dedupe_key.as_str())
            .collect::<Vec<_>>(),
        vec![
            "fanout:events",
            "scan:deliveries",
            "scan:live",
            "sweep:expired"
        ],
        "three boots must leave exactly four rows"
    );

    // Both run, and both go back on the clock. A sweep that finished would be
    // a deployment that swept once and never again — which is the bug
    // `vpay-server`'s boot-time sweep actually was.
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..4 {
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
            "fan_out_events",
            "scan_deliveries",
            "scan_live_charges",
            "sweep_expired"
        ],
        "all four singletons must go back on the clock; a fan-out that finished \
         instead is a deployment that drains its outbox once and never again, and a \
         delivery backstop that finished is one whose lost delivery jobs are lost for \
         good"
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

    let attended = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let attended_charge = crashed_charge(
        &h.pool,
        &attended,
        PUSH_RAIL,
        Uuid::new_v4(),
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;

    let orphan = confirmed_intent(&h.pool, MERCHANT, PUSH_RAIL, AMOUNT, CURRENCY).await?;
    let orphan_charge = crashed_charge(
        &h.pool,
        &orphan,
        PUSH_RAIL,
        Uuid::new_v4(),
        AMOUNT,
        CURRENCY,
        Some(MSISDN),
    )
    .await?;
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

    vpay_worker::seed_singletons(&h.pool).await?;
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

    let mut tx = h.pool.begin().await?;
    for n in 0..8 {
        vpay_db::jobs::enqueue_in_tx(
            &mut tx,
            "poll_charge",
            &format!("poll:ch_{n}"),
            &serde_json::json!({ "charge_id": format!("ch_{n}") }),
            time::OffsetDateTime::now_utc(),
        )
        .await?;
    }
    tx.commit().await?;

    // Eight real tasks on the multi-thread runtime, two worker identities,
    // all racing one queue.
    let mut set = tokio::task::JoinSet::new();
    for n in 0..8 {
        let pool = h.pool.clone();
        let worker = format!("worker-{}", n % 2);
        set.spawn(async move { vpay_db::jobs::claim(&pool, &worker).await });
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
