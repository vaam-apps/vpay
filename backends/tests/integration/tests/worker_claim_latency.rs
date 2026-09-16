//! How long one worker makes a claimable job wait — measured, and pinned
//! (issue #100).
//!
//! # What this file exists for
//!
//! The exp38 review (2026-09-10, `docs/plans/exp38-sigterm-scenario-notes/`)
//! staged its `Drain::TimedOut` scenario against a **single**
//! `--worker-concurrency 1` worker first, and abandoned it: the fixture
//! failed to reach its in-flight state on four attempts out of six. It
//! blamed "the settlement poll waiting tens of seconds behind the singleton
//! jobs the single task also has to run", moved to two workers — which pass
//! 10/10 — and wrote down plainly that the mechanism had *not* been
//! diagnosed. Issue #100 is that unfinished sentence: measure the latency,
//! pin a bound, and change the loop only if the bound is not met.
//!
//! # What is measured, exactly
//!
//! The interval between a job **becoming claimable** and its leaving the
//! claimable set, for every job of a small backlog, against a
//! `vpay_worker::run_loop` — the shipping loop, the shipping claim
//! (`vpay_db::jobs::claim`, `FOR UPDATE SKIP LOCKED`, one row per statement),
//! and the shipping housekeeping running throughout. Once at
//! `concurrency = 1`, which is the case issue #100 is about, and once at
//! `concurrency = 2` under the same bound, which is the control: the fixture
//! that prompted the issue passed with two workers and failed with one, so a
//! measurement that never looked at two could not say whether the claim path
//! was the difference.
//!
//! "Becoming claimable" is the **commit** of the enqueue and not the `run_at`
//! written into the row: until that transaction commits no worker can see the
//! row at all, so the commit is the earliest instant a claim could possibly
//! have happened. That also keeps the measurement off any comparison between
//! this process's clock and the database's — [`Instant`] is read once, on the
//! line after the commit returns, and every number here is an offset from it.
//!
//! A job leaves the claimable set when [`vpay_db::Jobs::claim`] stamps its
//! lease. [`claim_curve`] samples `locked_at IS NULL AND run_at <= now()`
//! every [`SAMPLE`], so each number carries at most one sample interval of
//! quantisation — stated rather than hidden, and small against the bounds
//! below.
//!
//! # The probe, and why it is this job
//!
//! `scan_live_charges` rows under dedupe keys of this suite's own. Three
//! properties earn it the job:
//!
//! - it is a **real kind**, spelled by [`JobKind::as_wire_str`] and accepted
//!   by migration 0034's `kind_is_known` CHECK, so these are rows the queue
//!   would accept from the shipping enqueue path — which is the path that
//!   writes them here;
//! - on an empty `charges` table its handler is one indexed read and no
//!   write, so what the interval measures is the **loop's cadence** and not a
//!   handler's work. Anything heavier would measure the handler;
//! - its dedupe key is not the singleton's (`vpay_worker::jobs`'
//!   `SCAN_DEDUPE_KEY`), so the seeded `scan_live_charges` job keeps its own
//!   row and every housekeeping
//!   singleton — including the five-second outbox drain — keeps running for
//!   the whole measurement. That is deliberate: the exp38 note blamed the
//!   singletons, so the number has to be taken with them live or it answers a
//!   different question.
//!
//! # No test doubles
//!
//! Real Postgres in a container, the shipping loop, the shipping repositories.
//! The adapter and rail maps are **empty**, and that is a tripwire rather than
//! a stand-in: no job this suite enqueues may reach a rail, and an empty map
//! makes a probe that somehow did fail loudly (`no adapter for …`) instead of
//! quietly performing HTTP. `support::no_webhook_endpoints` carries the same
//! argument for the endpoint table.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use sqlx::PgPool;
use time::OffsetDateTime;
use vpay_db::{Repositories, TxOutcome, UnitOfWork as _};
use vpay_worker::run_loop::IDLE_SLEEP;
use vpay_worker::{Adapters, JobKind, RailConfigs, RecoveryPolicy};

mod support;

use support::migrated_postgres;

/// How many jobs one round drops on the queue at once.
///
/// Eight, because eight is the number the exp38 transcript recorded for the
/// worker that claimed everything in the scenario this issue comes from
/// (`job loop stopped … claimed=8`). A "small backlog" is not a stress test:
/// the question is whether a single worker walks a handful of ready jobs
/// promptly, not how it behaves at thousands.
const BACKLOG: usize = 8;

/// How many times the backlog is enqueued and drained.
///
/// Ten rounds, so the reported spread is a distribution rather than one
/// sample, and so a bound that only *usually* holds fails here rather than in
/// somebody else's suite. They share one container: the cost of a round is
/// milliseconds of queue time, and a fresh container per round would make
/// startup the thing being measured.
const ROUNDS: usize = 10;

/// How often [`claim_curve`] asks the queue what is still claimable.
///
/// Five milliseconds: two orders of magnitude under [`IDLE_SLEEP`], so the
/// quantisation it adds cannot be mistaken for the cadence being measured,
/// and coarse enough that the sampler is not itself a load on the pool the
/// worker is claiming through.
const SAMPLE: Duration = Duration::from_millis(5);

/// The bound this file pins: **a single worker claims every job of a small
/// backlog within three seconds of the backlog becoming claimable.**
///
/// Why three seconds. The only wait `vpay_worker::run_loop` imposes on a job
/// that arrives is [`IDLE_SLEEP`] — one second, paid once, when the arrival
/// lands just after an empty claim; after a *non-empty* claim the task loops
/// straight back into `Jobs::claim` with no sleep at all, so the rest of the
/// backlog costs a round trip each. Three seconds is three times the one
/// wait the loop can impose, which leaves the bound insensitive to a loaded
/// host and still red for every regression in reach: raising `IDLE_SLEEP`
/// past it, sleeping it per claimed job (eight seconds for this backlog), or
/// any per-job cadence over ~370 ms.
///
/// It is a literal and not an expression over [`IDLE_SLEEP`] on purpose. A
/// bound spelled `IDLE_SLEEP * 3` would widen itself the moment somebody
/// widened the cadence, which is the one change it exists to catch.
const CLAIM_BOUND: Duration = Duration::from_secs(3);

/// The second bound: **once claiming, the backlog is walked back-to-back.**
///
/// The span from the first claim of a round to the last, which is the loop's
/// per-job cost with the initial wait taken out. One second for eight jobs —
/// measured in tens of milliseconds — so it is red for any per-job sleep of
/// more than ~140 ms while tolerating a database round trip an order of
/// magnitude slower than the one this measurement was taken with.
///
/// It is what distinguishes "the loop waits once" from "the loop waits per
/// job", and [`CLAIM_BOUND`] alone cannot: a backlog claimed one job per
/// `IDLE_SLEEP` would fail both, but a *smaller* per-job cadence would slip
/// past a total-time bound and starve a real queue just the same.
const DRAIN_SPAN_BOUND: Duration = Duration::from_secs(1);

/// How long a round may take before the case gives up and prints the queue.
///
/// Far above [`CLAIM_BOUND`]: a round that busts the bound should fail on the
/// bound, with its numbers, rather than time out with none of them.
const ROUND_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the loop is given to work through the five jobs
/// `run_loop` seeds at boot before the first round is enqueued.
const QUIET_TIMEOUT: Duration = Duration::from_secs(60);

/// The drain budget the measurement's own shutdown is given.
///
/// Ten seconds, which this case never spends: every probe is milliseconds
/// long, so the drain is over as soon as the task between two claims notices
/// the signal. It is here so that a drain which *did* hang would fail the
/// case rather than hang the suite.
const DRAIN_GRACE: Duration = Duration::from_secs(10);

/// The worker id every claim in this suite is stamped with.
const WORKER: &str = "claim-latency-suite";

// ------------------------------------------------------------------ fixture

/// One round's worth of probe rows, enqueued in one transaction.
///
/// Returns the dedupe keys and the instant the transaction committed — the
/// instant the rows became claimable, and the zero of every number this file
/// reports.
///
/// The enqueue is `vpay_db`'s own (`UnitOfWork::enqueue_in_tx`, the call the
/// confirm path and the backstop scan both make), so these rows go through
/// the same `ON CONFLICT (dedupe_key) DO NOTHING` insert and the same CHECKs
/// as any other job.
async fn enqueue_round(
    repositories: &dyn Repositories,
    round: usize,
) -> anyhow::Result<(Vec<String>, Instant)> {
    let keys: Vec<String> = (0..BACKLOG)
        .map(|index| format!("claim-latency-probe:{round}:{index}"))
        .collect();
    let payload = serde_json::Value::Object(serde_json::Map::new());
    let run_at = OffsetDateTime::now_utc();

    repositories
        .transaction(|tx| {
            let keys = &keys;
            let payload = &payload;
            Box::pin(async move {
                for key in keys {
                    let inserted = tx
                        .enqueue_in_tx(JobKind::ScanLiveCharges.as_wire_str(), key, payload, run_at)
                        .await?;
                    assert!(inserted, "the probe key `{key}` was already taken");
                }
                Ok::<_, vpay_db::DbError>(TxOutcome::Commit(()))
            })
        })
        .await
        .context("enqueueing a round of probe jobs")?;

    Ok((keys, Instant::now()))
}

/// How many of `keys` are still claimable, asked exactly as
/// [`vpay_db::Jobs::claim`]'s own subquery asks it.
///
/// Read with plain `sqlx` rather than through a repository: what this samples
/// is the state a *claim* would see, and the claim's predicate is SQL. A
/// repository method that happened to agree with it would prove only that the
/// two agree.
async fn still_claimable(pool: &PgPool, keys: &[String]) -> anyhow::Result<usize> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs \
         WHERE dedupe_key = ANY($1) AND locked_at IS NULL AND run_at <= now()",
    )
    .bind(keys)
    .fetch_one(pool)
    .await
    .context("counting the probe jobs still claimable")?;
    Ok(usize::try_from(count).unwrap_or(usize::MAX))
}

/// When each job of a round left the claimable set, in order.
///
/// The result is a claim *curve*: element `i` is how long after the commit
/// the `i + 1`-th job had been claimed. It says which jobs the single worker
/// picked up when, and it is what makes "the loop waits once, not per job"
/// checkable rather than assertable.
///
/// Ordering is by the sampler, not by row: two jobs claimed inside one
/// [`SAMPLE`] are recorded at the same instant, which is why the assertions
/// below are about the first and the last element and never about the gap
/// between two adjacent ones.
async fn claim_curve(
    pool: &PgPool,
    keys: &[String],
    committed: Instant,
) -> anyhow::Result<Vec<Duration>> {
    let mut claimed: Vec<Duration> = Vec::with_capacity(keys.len());
    loop {
        let remaining = still_claimable(pool, keys).await?;
        let gone = keys.len().saturating_sub(remaining);
        let now = committed.elapsed();
        while claimed.len() < gone {
            claimed.push(now);
        }
        if claimed.len() == keys.len() {
            return Ok(claimed);
        }
        if now > ROUND_TIMEOUT {
            panic!(
                "{remaining} of {} probe jobs were still claimable {now:?} after the commit; \
                 the claim curve so far is {claimed:?}",
                keys.len()
            );
        }
        tokio::time::sleep(SAMPLE).await;
    }
}

/// Waits until nothing is claimable and nothing is leased.
///
/// Called once, before the first round: `run_loop` seeds five housekeeping
/// jobs at `run_at = now()` and this case is not measuring the boot backlog.
/// Every round after that starts wherever the housekeeping happens to be,
/// which is the state a deployment is actually in.
async fn wait_until_quiet(pool: &PgPool) -> anyhow::Result<()> {
    let deadline = Instant::now() + QUIET_TIMEOUT;
    loop {
        let busy: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM jobs WHERE locked_at IS NOT NULL OR run_at <= now()",
        )
        .fetch_one(pool)
        .await
        .context("counting the jobs the worker still has to run")?;
        if busy == 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "the worker had not finished the {busy} jobs it seeds at boot within \
                 {QUIET_TIMEOUT:?}"
            );
        }
        tokio::time::sleep(SAMPLE).await;
    }
}

// ------------------------------------------------------------------ reading

/// The three numbers a round is summarised by.
#[derive(Debug, Clone, Copy)]
struct Round {
    /// When the worker took the first job of the backlog. This is the wait
    /// the loop imposes on an *arrival*: the task is idling when the round
    /// lands, so it is somewhere in `[0, IDLE_SLEEP]` plus a claim.
    first: Duration,
    /// When it took the last. The number [`CLAIM_BOUND`] is about.
    last: Duration,
    /// `last - first`: the loop's per-job cost for this backlog, with the
    /// arrival wait removed. The number [`DRAIN_SPAN_BOUND`] is about.
    span: Duration,
}

impl Round {
    fn of(curve: &[Duration]) -> Self {
        let first = *curve.first().expect("a round has BACKLOG samples");
        let last = *curve.last().expect("a round has BACKLOG samples");
        Self {
            first,
            last,
            span: last.saturating_sub(first),
        }
    }
}

/// `min / median / max` of one column of the table, for the report.
fn spread(values: &[Duration]) -> (Duration, Duration, Duration) {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let median = sorted
        .get(sorted.len() / 2)
        .copied()
        .expect("a round was measured");
    (
        sorted.first().copied().expect("a round was measured"),
        median,
        sorted.last().copied().expect("a round was measured"),
    )
}

// --------------------------------------------------------------- the measure

/// Boots a worker at `concurrency`, drains [`ROUNDS`] backlogs through it, and
/// asserts both bounds on every one of them.
///
/// Shared by the two cases below rather than copied into each: the only thing
/// they vary is the number of claim tasks, and that is exactly the variable
/// the issue is about. Two copies would be free to drift on the one number
/// they are being compared at.
async fn measure_backlog_latency(concurrency: usize) -> anyhow::Result<()> {
    let (_postgres, repositories, pool) = migrated_postgres().await?;

    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let loop_repositories = Arc::clone(&repositories);
    let worker = tokio::spawn(async move {
        vpay_worker::run_loop(
            loop_repositories,
            Arc::new(Adapters::new()),
            Arc::new(RailConfigs::new()),
            RecoveryPolicy::default(),
            support::no_webhook_endpoints(),
            support::default_egress_policy(),
            concurrency,
            DRAIN_GRACE,
            WORKER.to_owned(),
            async move {
                let _ = stopped.await;
            },
        )
        .await
    });

    wait_until_quiet(&pool).await?;

    let mut rounds: Vec<Round> = Vec::with_capacity(ROUNDS);
    let mut curves: Vec<Vec<Duration>> = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        let (keys, committed) = enqueue_round(repositories.as_ref(), round).await?;
        let curve = claim_curve(&pool, &keys, committed).await?;
        rounds.push(Round::of(&curve));
        curves.push(curve);
    }

    let _ = stop.send(());
    let report = worker.await.context("the worker task panicked")?;

    // Printed and not only asserted: issue #100 asks for numbers, and a bound
    // that passes says only "under three seconds". `cargo nextest run
    // --no-capture` (or `--success-output immediate`) prints them, and the
    // dated page under `docs/status/verification/` is where a run of them
    // was written down.
    eprintln!(
        "concurrency {concurrency}, {BACKLOG} jobs per round, {ROUNDS} rounds, sampled every \
         {SAMPLE:?}"
    );
    for (index, (round, curve)) in rounds.iter().zip(&curves).enumerate() {
        eprintln!(
            "  round {index}: first {:?}, last {:?}, span {:?}  curve {curve:?}",
            round.first, round.last, round.span
        );
    }
    let firsts: Vec<Duration> = rounds.iter().map(|round| round.first).collect();
    let lasts: Vec<Duration> = rounds.iter().map(|round| round.last).collect();
    let spans: Vec<Duration> = rounds.iter().map(|round| round.span).collect();
    eprintln!("  first claim  min/median/max {:?}", spread(&firsts));
    eprintln!("  last claim   min/median/max {:?}", spread(&lasts));
    eprintln!("  drain span   min/median/max {:?}", spread(&spans));
    eprintln!(
        "  the loop claimed {} jobs in all; {} of them were the housekeeping singletons \
         running alongside the probes",
        report.claimed,
        report
            .claimed
            .saturating_sub(u64::try_from(ROUNDS * BACKLOG).unwrap_or(u64::MAX))
    );

    for (index, (round, curve)) in rounds.iter().zip(&curves).enumerate() {
        assert!(
            round.last <= CLAIM_BOUND,
            "concurrency {concurrency}, round {index}: the last of {BACKLOG} jobs was claimed \
             {:?} after the backlog became claimable, over the {CLAIM_BOUND:?} bound. The \
             worker is starving its own queue: the only wait the loop imposes on an arrival \
             is IDLE_SLEEP ({IDLE_SLEEP:?}), paid once, and after a non-empty claim there is \
             no wait at all. Curve: {curve:?}",
            round.last,
        );
        assert!(
            round.span <= DRAIN_SPAN_BOUND,
            "concurrency {concurrency}, round {index}: the backlog took {:?} to walk from its \
             first claim to its last, over the {DRAIN_SPAN_BOUND:?} bound. The loop is paying \
             a cadence *per job* instead of once per empty claim. Curve: {curve:?}",
            round.span,
        );
    }

    assert!(
        report.claimed >= u64::try_from(ROUNDS * BACKLOG).unwrap_or(u64::MAX),
        "the loop reported {} claims for {} probe jobs; the numbers above were measured \
         against a worker that never claimed them",
        report.claimed,
        ROUNDS * BACKLOG
    );
    assert_eq!(
        report.lost, 0,
        "a lease was reaped out from under this worker mid-job, so the claim curve is \
         somebody else's: {report:?}"
    );
    Ok(())
}

// ---------------------------------------------------------------- the cases

/// **The bound this issue exists to pin.** One worker, a backlog of eight,
/// ten times: every job claimed inside [`CLAIM_BOUND`], and the backlog
/// walked back-to-back inside [`DRAIN_SPAN_BOUND`].
///
/// A multi-threaded runtime because that is what `vpay-server worker` runs on
/// (`#[tokio::main]`), and because a sampler sharing one thread with the loop
/// would be measuring its own scheduling as the loop's latency. `concurrency`
/// is the number of claim *tasks* either way, and that is what this case
/// holds at one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_worker_claims_a_small_backlog_within_the_bound() -> anyhow::Result<()> {
    measure_backlog_latency(1).await
}

/// The control: **two claim tasks are held to the same bound.**
///
/// Issue #100 comes from a fixture that failed four times in six with one
/// worker and passed ten in ten with two, and the conclusion drawn at the
/// time was that the shipping loop starves a single worker. This case is what
/// makes that conclusion checkable rather than plausible: if the claim path
/// were the difference, the same backlog would be claimed *faster* here by
/// something outside the bound the case above holds. It is not — both sit on
/// [`IDLE_SLEEP`] — so whatever separated those two stagings is not the loop's
/// cadence, and the numbers are on the dated verification page.
///
/// It is a second container and about fifteen seconds of gate. That is the
/// price of the comparison being in the suite instead of in a comment.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_workers_are_held_to_the_same_bound_as_one() -> anyhow::Result<()> {
    measure_backlog_latency(2).await
}
