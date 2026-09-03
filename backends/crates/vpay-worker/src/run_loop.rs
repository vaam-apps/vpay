//! The job loop: claim, run, settle — and the bounded drain that stops it.
//!
//! [`handle`] says what one job *did*; this module owns the row it did it to.
//! That split is deliberate and is why [`crate::handlers`] returns an
//! [`Outcome`] instead of writing to `jobs` itself: a handler that could
//! delete its own row could delete one it had not finished, and a handler
//! that could park a row could dead-letter a charge by returning `Ok`.
//! Everything that ends a lease is in this file, and every one of those
//! writes is guarded on `locked_by` (`vpay_db::jobs`), so a worker whose
//! lease was reaped mid-run discards its answer instead of stamping it over
//! whoever holds the job now.
//!
//! # Why the loop is a function and not the binary's `main`
//!
//! `vpay-worker-bin` calls [`run_loop`] and so does
//! `backends/tests/integration/tests/worker_e2e.rs`. There is no second
//! implementation, no `#[cfg(test)]` variant and no injected clock: the
//! integration suite drives *this* loop, against a real Postgres and a real
//! WireMock rail, which is the only way a claim/settle protocol can be
//! proven at all — `SKIP LOCKED`, the `locked_by` guard and the drain are
//! properties of Postgres and of concurrency, not of Rust types.
//!
//! [`run_once`] is public for the same reason and is not a seam: it is the
//! loop's own body, called by [`run_loop`] N times per task. A test that
//! wants to observe exactly one job's disposition calls it directly rather
//! than racing a background loop and scraping logs for the answer.
//!
//! # What this module does not decide
//!
//! Not the retry policy — that is [`JobError::decision`], derived from
//! `Classify` so the worker and the API cannot disagree about whether a
//! Postgres failure is transient (ADR-0011). Not the poll ladder — that is
//! [`crate::poll_delay`]. Not what a rail's answer means — that is
//! `vpay_core::settle`. This module maps a [`Decision`] onto one of three
//! writes and counts how often each happened.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use time::OffsetDateTime;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;
use vpay_core::{Classify as _, Severity};
use vpay_db::{DbError, JobRow, PgPool};

use crate::error::{Decision, JobError};
use crate::handlers::{Adapters, RailConfigs, WebhookContext, handle};
use crate::jobs::{
    FANOUT_DEDUPE_KEY, JobKind, Outcome, SCAN_DEDUPE_KEY, SCAN_DELIVERIES_DEDUPE_KEY,
    SWEEP_DEDUPE_KEY,
};
use crate::recovery::RecoveryPolicy;
use crate::webhooks::EndpointRegistry;

/// How long a task waits after finding the queue empty before asking again.
///
/// One second, and a plain sleep rather than `LISTEN`/`NOTIFY`. Latency here
/// is not a payment's latency: the fastest rung of the poll ladder is ten
/// seconds (`crate::poll_delay`), so a job is never waiting on this loop —
/// it is waiting on its own `run_at`. What this number actually trades is
/// idle database load, and one empty `claim` per second per task is
/// negligible against `jobs_claimable_idx`.
pub const IDLE_SLEEP: Duration = Duration::from_secs(1);

/// How often the loop emits its gauge line.
///
/// A minute, matched to the coarsest thing the line reports (queue age).
/// Deliberately a log line and not a metrics endpoint: this binary serves no
/// HTTP at all, and the alerting path this repository has today is the log
/// pipeline `--log-format json` feeds.
pub const GAUGE_INTERVAL: Duration = Duration::from_secs(60);

/// The floor under the lease reaper's period ([`reap_interval`]).
///
/// The reaper normally runs every `RecoveryPolicy::lease / 2`, which at the
/// documented five-minute lease is every 150 s. A deployment (or a test) that
/// sets a very short lease must not turn that into a hot loop against
/// Postgres, so the period never falls below the same second the idle claim
/// path uses: nothing in this loop asks the database anything more often than
/// [`IDLE_SLEEP`], and a reaper is the least urgent of them.
const MIN_REAP_INTERVAL: Duration = IDLE_SLEEP;

/// What the loop did with a claimed row.
///
/// Four values, because there are exactly four writes that can end a lease
/// and each one means something different to an operator reading the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// `DELETE`d. The work is done — or was already done by an earlier run,
    /// which the compare-and-swap writes make indistinguishable on purpose.
    /// Also the answer for [`Decision::Terminal`]: a declined charge is
    /// finished business for the queue, and whatever happens next is a new
    /// PaymentIntent decided by the intent's state machine.
    Finished,
    /// Released and moved this far into the future, with the failure (if
    /// there was one) recorded in `last_error`.
    Rescheduled(Duration),
    /// Parked at `run_at = 'infinity'` for a human
    /// (`vpay_db::jobs::dead_letter`). Only [`Decision::DeadLetter`] reaches
    /// this, i.e. only `Retry::Never`.
    DeadLettered,
    /// The guarded write matched no row: this worker's lease was reaped
    /// while it was running, someone else holds the job now, and this
    /// worker's answer has been discarded.
    ///
    /// Not an error. It is the honest name for "we were too slow", and it is
    /// counted separately because a deployment seeing any of these has a
    /// lease shorter than one of its handlers — which is a real defect, and
    /// invisible if it were folded into either neighbour.
    Lost,
}

/// One job, run to a disposition.
///
/// Returned by [`run_once`] so a caller — the loop, or an integration test —
/// can see what happened to the row without parsing the log line that
/// described it.
#[derive(Debug, Clone)]
pub struct Settled {
    /// The `jobs.id` that was claimed.
    pub job_id: Uuid,
    /// Its `kind`, carried as written in the row rather than as a
    /// [`JobKind`]: a row this build cannot interpret is exactly the case
    /// worth reporting, and it has no `JobKind`.
    pub kind: String,
    /// What was written.
    pub disposition: Disposition,
    /// Whether the job failed, and with what. `None` for a handler that
    /// returned `Ok`.
    ///
    /// A [`String`] rather than the [`JobError`] itself because [`Settled`]
    /// outlives the borrow the error was formed against and, more to the
    /// point, because the *classification* has already been consumed here —
    /// what remains is the sentence an operator reads.
    pub error: Option<String>,
    /// Whether a human was told, i.e. whether [`Decision::RetryAfter`]'s
    /// `alert` was set. Always `false` for a job that succeeded.
    pub alert: bool,
}

/// Whether in-flight jobs finished inside the grace period.
///
/// The same two outcomes, and the same names, as `vpay-server`'s
/// `DrainOutcome`, because an operator reads the two binaries' shutdown
/// behaviour the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drain {
    /// Every task finished the job it was on and stopped claiming.
    Clean,
    /// `--shutdown-grace-seconds` elapsed first. The remaining tasks were
    /// aborted and their leases handed back.
    TimedOut,
}

/// What one run of [`run_loop`] did, for the binary's exit code and for a
/// test's assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopReport {
    /// How the drain ended. [`Drain::TimedOut`] is what makes the binary
    /// exit non-zero.
    pub drain: Drain,
    /// Jobs claimed by this worker over the whole run.
    pub claimed: u64,
    /// Of those, how many ended [`Disposition::Finished`].
    pub finished: u64,
    /// … [`Disposition::Rescheduled`].
    pub rescheduled: u64,
    /// … [`Disposition::DeadLettered`].
    pub dead_lettered: u64,
    /// … [`Disposition::Lost`] — see that variant: any non-zero value here
    /// is a lease shorter than a handler.
    pub lost: u64,
    /// Leases handed back by the drain. Zero on a clean drain by
    /// construction — see [`run_loop`].
    pub released: u64,
}

/// The loop's running tallies.
///
/// Atomics rather than a mutex or a channel: every one is a single
/// increment on a path that is otherwise dominated by a network round trip,
/// and the gauge reader tolerates a skewed snapshot (it is a gauge, not a
/// ledger). `Relaxed` for the same reason — nothing orders anything else on
/// these values.
#[derive(Debug, Default)]
struct Counters {
    claimed: AtomicU64,
    finished: AtomicU64,
    rescheduled: AtomicU64,
    dead_lettered: AtomicU64,
    lost: AtomicU64,
}

impl Counters {
    fn record(&self, settled: &Settled) {
        self.claimed.fetch_add(1, Ordering::Relaxed);
        let counter = match settled.disposition {
            Disposition::Finished => &self.finished,
            Disposition::Rescheduled(_) => &self.rescheduled,
            Disposition::DeadLettered => &self.dead_lettered,
            Disposition::Lost => &self.lost,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.claimed.load(Ordering::Relaxed),
            self.finished.load(Ordering::Relaxed),
            self.rescheduled.load(Ordering::Relaxed),
            self.dead_lettered.load(Ordering::Relaxed),
            self.lost.load(Ordering::Relaxed),
        )
    }
}

/// A name for this process's leases, unique enough that two workers can
/// never share one.
///
/// Three parts, each covering a way the others collide: the hostname (two
/// pods), the pid (two processes in one container, which
/// `docker compose run` makes easy), and a random suffix (a pod restarting
/// onto the same name and pid, which is not exotic in a container with a
/// fixed hostname and pid 1).
///
/// The random part is what actually carries the guarantee, and it is why
/// this is not simply a UUID: `locked_by` is read by a human deciding
/// whether a stuck job belongs to a worker that is still alive, and
/// `vpay-worker-7f4c/1/9e21ab7c` tells them which pod to look at while a
/// bare UUID tells them nothing.
///
/// `HOSTNAME` rather than a `hostname` crate: the value is a label in a log
/// line and in a column, never an address anything connects to, so a
/// deployment that does not export it loses nothing but the hint. Reading an
/// environment variable is safe; only *setting* one is `unsafe` in edition
/// 2024, which is why this is a read and the tests do not try to control it.
#[must_use]
pub fn worker_id() -> String {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_owned());
    let short: String = Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect();
    format!("{host}/{pid}/{short}", pid = std::process::id())
}

/// Claims one job and settles it, or answers `Ok(None)` if the queue had
/// nothing runnable.
///
/// This is the loop body, and the whole of the claim/settle protocol:
///
/// 1. `vpay_db::jobs::claim` — one row, `FOR UPDATE SKIP LOCKED`, `attempts`
///    incremented by the claim itself so a job that kills its worker still
///    counts up;
/// 2. [`handle`] — the work, which may call a rail and may commit;
/// 3. exactly one of `finish` / `reschedule` / `dead_letter`, chosen from
///    the [`Outcome`] or from [`JobError::decision`], each guarded on this
///    worker's `locked_by`.
///
/// There is no step between 2 and 3. A job whose handler committed a
/// settlement and then failed to write to `jobs` is re-run, and every
/// handler is a compare-and-swap for that reason — the second run matches no
/// rows and answers [`Outcome::Done`].
///
/// # Errors
///
/// [`DbError`] only, and only from the queue's own statements: the claim, or
/// the write that ends the lease. A failure of the *work* is not an error
/// here — it has already been classified and turned into a
/// [`Disposition`] — because the loop's own health and a job's health are
/// different questions, and a caller that could not tell them apart would
/// back off from the queue every time a rail timed out.
pub async fn run_once(
    pool: &PgPool,
    adapters: &Adapters,
    rails: &RailConfigs,
    policy: &RecoveryPolicy,
    webhooks: &WebhookContext<'_>,
    worker_id: &str,
) -> Result<Option<Settled>, DbError> {
    let Some(job) = vpay_db::jobs::claim(pool, worker_id).await? else {
        return Ok(None);
    };

    let result = handle(pool, adapters, rails, policy, webhooks, &job).await;
    let settled = settle(pool, worker_id, &job, result).await?;
    log_disposition(&settled);
    Ok(Some(settled))
}

/// Turns a handler's answer into the one write that ends the lease.
///
/// The `Ok` arms are [`Outcome`]'s two values. The `Err` arm asks
/// [`JobError::decision`] and nothing else — deliberately no `match` on the
/// error's variants here, because that is the second retry policy ADR-0011
/// exists to prevent.
///
/// `last_error` is cleared (`None`) on a successful reschedule and set on a
/// failed one. A rung of the ladder that ran cleanly should not leave the
/// previous rung's rail timeout sitting in the column for an operator to
/// read as current.
async fn settle(
    pool: &PgPool,
    worker_id: &str,
    job: &JobRow,
    result: Result<Outcome, JobError>,
) -> Result<Settled, DbError> {
    let error = match result {
        Ok(Outcome::Done) => {
            let wrote = vpay_db::jobs::finish(pool, job.id, worker_id).await?;
            return Ok(settled(
                job,
                disposition(wrote, Disposition::Finished),
                None,
                false,
            ));
        }
        Ok(Outcome::RescheduleAfter(delay)) => {
            let wrote = vpay_db::jobs::reschedule(pool, job.id, worker_id, delay, None).await?;
            return Ok(settled(
                job,
                disposition(wrote, Disposition::Rescheduled(delay)),
                None,
                false,
            ));
        }
        Err(error) => error,
    };

    let text = error.to_string();
    match error.decision(attempt_index(job)) {
        Decision::RetryAfter { delay, alert } => {
            let wrote =
                vpay_db::jobs::reschedule(pool, job.id, worker_id, delay, Some(&text)).await?;
            Ok(settled(
                job,
                disposition(wrote, Disposition::Rescheduled(delay)),
                Some(text),
                alert,
            ))
        }
        Decision::Terminal => {
            let wrote = vpay_db::jobs::finish(pool, job.id, worker_id).await?;
            Ok(settled(
                job,
                disposition(wrote, Disposition::Finished),
                Some(text),
                false,
            ))
        }
        Decision::DeadLetter => {
            let wrote = vpay_db::jobs::dead_letter(pool, job.id, worker_id, &text).await?;
            Ok(settled(
                job,
                disposition(wrote, Disposition::DeadLettered),
                Some(text),
                // A parked job is work nothing will ever do again. That is
                // always worth a human, whatever the leaf error's severity
                // said about a single occurrence of it.
                true,
            ))
        }
    }
}

/// `Ok(false)` from a `locked_by`-guarded write means the lease moved on, and
/// the disposition the caller intended never happened.
const fn disposition(wrote: bool, intended: Disposition) -> Disposition {
    if wrote { intended } else { Disposition::Lost }
}

fn settled(job: &JobRow, disposition: Disposition, error: Option<String>, alert: bool) -> Settled {
    Settled {
        job_id: job.id,
        kind: job.kind.clone(),
        disposition,
        error,
        alert,
    }
}

/// This job's rung on the poll ladder, 0-indexed.
///
/// `jobs.attempts` counts from one and is incremented by the claim, so the
/// run executing now is attempt `attempts` and its rung is one less. The
/// same indexing `crate::handlers` uses, so a handler that reschedules
/// itself and a handler that fails wait the same length of time.
fn attempt_index(job: &JobRow) -> u32 {
    u32::try_from(job.attempts)
        .unwrap_or(u32::MAX)
        .saturating_sub(1)
}

/// The per-job log line: what happened to the row.
///
/// Distinct from `crate::handlers`' own failure line, which reports the
/// *error* at `Classify::severity` and flags `Severity::Page`. This one
/// reports the *disposition* and carries `alert` from
/// [`Decision::RetryAfter`], which fires at `Severity::Error` and above — a
/// wider net, and the one the 24-hour `unresolved` escalation needs
/// (`JobError::Exhausted` is `Severity::Error`, not `Page`, so the handler's
/// line does not flag it and this one must).
///
/// The consequence, stated so nobody has to rediscover it: a `Page`-severity
/// failure produces two events carrying `alert = true` — the error and its
/// disposition. That is deliberate. They say different things ("the rail
/// refused our credentials" and "so this job is parked forever"), and an
/// alerting rule that deduplicates on `job_id` sees one incident either way.
fn log_disposition(settled: &Settled) {
    let Some(error) = settled.error.as_deref() else {
        tracing::debug!(
            job_id = %settled.job_id,
            kind = %settled.kind,
            disposition = ?settled.disposition,
            "job settled"
        );
        return;
    };

    // Four arms rather than a `Level` variable, for the reason
    // `crate::tracing_level`'s doc comment gives: `alert` is an event field
    // and has to be written at the macro call site.
    match (settled.alert, settled.disposition) {
        (true, disposition) => tracing::error!(
            alert = true,
            job_id = %settled.job_id,
            kind = %settled.kind,
            disposition = ?disposition,
            "job failed: {error}"
        ),
        (false, Disposition::DeadLettered | Disposition::Lost) => tracing::error!(
            job_id = %settled.job_id,
            kind = %settled.kind,
            disposition = ?settled.disposition,
            "job failed: {error}"
        ),
        (false, disposition) => tracing::warn!(
            job_id = %settled.job_id,
            kind = %settled.kind,
            disposition = ?disposition,
            "job failed: {error}"
        ),
    }
}

/// Seeds the four singleton jobs this deployment always wants running.
///
/// `sweep_expired`, `scan_live_charges`, `fan_out_events` and
/// `scan_deliveries` are not enqueued by anything that creates work — there is
/// no request that produces a sweep, and the settlement transaction writes an
/// `events` row without knowing a drain exists — so they are seeded at boot
/// and reschedule themselves for as long as the deployment lives.
/// `ON CONFLICT (dedupe_key) DO NOTHING` (`jobs::enqueue_in_tx`) is what makes
/// N workers all doing this produce one row each, and what stops a restart
/// from dragging a job that is already scheduled an hour out back to now.
///
/// One transaction for all four because they are one fact ("this deployment
/// runs its own background work"), and a partial seed is worse than none: the
/// sweep without the scan is a deployment whose backstop is silently absent,
/// and a deployment without `fan_out_events` settles payments and tells no
/// merchant about any of them — every `events` row it writes stays
/// `fanout_state = 'pending'`, which looks exactly like a healthy deployment
/// until somebody reads the backlog.
///
/// This is **not** where lease recovery happens. `sweep_expired` also reaps
/// expired leases, but a worker that has just booted after a crash cannot
/// wait an hour for that — and if the crashed worker was holding the
/// `sweep:expired` row itself, the sweep is exactly the job that can never
/// run. See [`run_loop`], which reaps before it seeds and again on its own
/// timer.
///
/// # Errors
///
/// [`DbError`] if the write or the commit fails.
pub async fn seed_singletons(pool: &PgPool) -> Result<(), DbError> {
    let now = OffsetDateTime::now_utc();
    let empty = serde_json::Value::Object(serde_json::Map::new());

    let mut tx = pool.begin().await.map_err(DbError::Query)?;
    vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        JobKind::SweepExpired.as_wire_str(),
        SWEEP_DEDUPE_KEY,
        &empty,
        now,
    )
    .await?;
    vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        JobKind::ScanLiveCharges.as_wire_str(),
        SCAN_DEDUPE_KEY,
        &empty,
        now,
    )
    .await?;
    // The outbox drain. `run_at = now` rather than a delay: an event written
    // before this process started is already waiting, and the first thing a
    // freshly-booted worker should do about a settled payment nobody was
    // told about is tell them.
    vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        JobKind::FanOutEvents.as_wire_str(),
        FANOUT_DEDUPE_KEY,
        &empty,
        now,
    )
    .await?;
    // The delivery backstop (migration 0023). Seeded here rather than left
    // out because "the queue owns every delivery" is only true while no job
    // is ever deleted, dead-lettered or lost with the table — and a
    // deployment that drops this seed looks exactly like a healthy one until
    // a merchant asks why they never heard about a payment.
    vpay_db::jobs::enqueue_in_tx(
        &mut tx,
        JobKind::ScanDeliveries.as_wire_str(),
        SCAN_DELIVERIES_DEDUPE_KEY,
        &empty,
        now,
    )
    .await?;
    tx.commit().await.map_err(DbError::Query)?;
    Ok(())
}

/// Runs `concurrency` claim/settle tasks until `shutdown` resolves, then
/// drains them within `grace`.
///
/// # The drain, and why a timed-out one hands leases back
///
/// On shutdown the tasks stop *claiming*; each finishes the job it is on.
/// That is the whole of a clean drain, and it is why [`LoopReport::released`]
/// is zero on one: a task only re-checks shutdown between jobs, so a claimed
/// job is always settled before its task exits, and there is no lease left
/// to hand back.
///
/// When `grace` elapses first the remaining tasks are aborted and
/// `vpay_db::jobs::release_all` clears every lease this worker still holds.
/// Without that call those rows stay leased until a reaper frees them — at
/// best half a lease (`reap_interval`), and only if a worker is running at
/// all — of a live charge going undriven for no reason other than that a pod
/// was rolled. The release is safe against the aborted tasks rather than
/// racing them: every
/// write that ends a lease is guarded on `locked_by`, so an abort that lands
/// mid-flight either committed before the release (and its guarded write
/// matched) or after it (and matched nothing, leaving the job for another
/// worker to re-run — which every handler is a compare-and-swap to make
/// safe).
///
/// # Errors
///
/// None: this returns a [`LoopReport`] and not a `Result`, because after the
/// seed there is nothing left whose failure should stop a worker. A claim
/// that fails is logged and retried after [`IDLE_SLEEP`] — Postgres being
/// briefly unreachable is the case the retry exists for, and exiting the
/// process instead would turn a blip into a restart loop. A seed that fails
/// is logged with `alert` and the loop starts anyway: the deployment loses
/// its sweeps, which matters, but the charges it is driving matter more.
#[expect(
    clippy::too_many_arguments,
    reason = "every argument is a distinct thing the loop needs from its caller, and the \
              two callers (the binary and the integration suite) supply different values \
              for each; a config struct would hide which of them a given test varies"
)]
pub async fn run_loop(
    pool: &PgPool,
    adapters: Arc<Adapters>,
    rails: Arc<RailConfigs>,
    policy: RecoveryPolicy,
    endpoints: Arc<EndpointRegistry>,
    http: reqwest::Client,
    concurrency: usize,
    grace: Duration,
    worker_id: String,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> LoopReport {
    // **Before the seed, and unconditionally.** A worker that was SIGKILLed
    // leaves every job it held with `locked_at` set, and `jobs::claim`
    // matches only `locked_at IS NULL` (that predicate is what makes
    // `jobs_claimable_idx` exact). So until something reaps those leases the
    // charges behind them are undriven — and if the dead worker was holding
    // `sweep:expired`, the job that used to be the only reaper is itself
    // among them, which is a deadlock the queue cannot leave on its own. The
    // reap uses `policy.lease`, so a *co-running* worker's fresh leases are
    // untouched; only a lease older than a worker could legitimately hold is
    // freed.
    reap_leases(pool, policy.lease, &worker_id).await;

    if let Err(error) = seed_singletons(pool).await {
        tracing::error!(
            alert = true,
            error = %error,
            "could not seed the housekeeping jobs; this deployment will not sweep expired \
             idempotency keys, client-assertion jtis or stale job leases until a worker \
             starts that can"
        );
    }

    let (tx, rx) = watch::channel(false);
    let signal: JoinHandle<()> = tokio::spawn(async move {
        shutdown.await;
        // A closed receiver means every task already stopped, which is the
        // state this send was trying to produce.
        let _ = tx.send(true);
    });

    let counters = Arc::new(Counters::default());
    let gauge = tokio::spawn(gauge_loop(
        pool.clone(),
        worker_id.clone(),
        Arc::clone(&counters),
    ));
    // Its own timer, not a rung of `sweep_expired`. The sweep runs hourly and
    // is itself a claimable job, so making it the only reaper means a lease
    // stranded by a crash can sit unclaimable for up to an hour — and forever
    // if the stranded lease is the sweep's own. Half a lease is the longest a
    // reaped job should wait to be picked up again, and it cannot free a
    // lease that is merely young: `reap_expired_leases` still compares
    // against `policy.lease`.
    let reaper = tokio::spawn(reaper_loop(pool.clone(), policy.lease, worker_id.clone()));

    let tasks: Vec<JoinHandle<()>> = (0..concurrency)
        .map(|_| {
            tokio::spawn(claim_loop(
                pool.clone(),
                Arc::clone(&adapters),
                Arc::clone(&rails),
                policy,
                Arc::clone(&endpoints),
                // `reqwest::Client` is an `Arc` internally, so this shares
                // the one connection pool the binary built rather than
                // opening `concurrency` of them.
                http.clone(),
                worker_id.clone(),
                rx.clone(),
                Arc::clone(&counters),
            ))
        })
        .collect();

    tracing::info!(
        worker_id = %worker_id,
        concurrency,
        shutdown_grace_seconds = grace.as_secs(),
        "job loop running"
    );

    let aborts: Vec<tokio::task::AbortHandle> = tasks
        .iter()
        .map(tokio::task::JoinHandle::abort_handle)
        .collect();
    let drain_all = async move {
        for task in tasks {
            // A task that panicked has already reported it; the drain's job
            // is to wait for the others, not to re-raise it.
            let _ = task.await;
        }
    };

    // **The grace clock starts when the signal arrives, not at boot.** Waiting
    // here rather than racing `timeout(grace, drain_all)` from the start is
    // the whole difference between a bounded drain and a worker that aborts
    // every in-flight job `grace` seconds after it started, forever. The
    // shape mirrors `vpay-server`'s `grace_clock`, which waits for draining to
    // have *begun* before it sleeps, for the same reason.
    let mut signalled = rx.clone();
    while !*signalled.borrow_and_update() {
        // `Err` means the sender is gone. Only the signal task holds it, and
        // it drops it after sending, so this is "shutdown happened and we
        // missed the edge" — proceed to drain either way.
        if signalled.changed().await.is_err() {
            break;
        }
    }
    tracing::info!(worker_id = %worker_id, "shutdown signalled; draining in-flight jobs");

    let (drain, released) = match tokio::time::timeout(grace, drain_all).await {
        Ok(()) => (Drain::Clean, 0),
        Err(_) => {
            for abort in &aborts {
                abort.abort();
            }
            let released = match vpay_db::jobs::release_all(pool, &worker_id).await {
                Ok(released) => released,
                Err(error) => {
                    tracing::error!(
                        alert = true,
                        worker_id = %worker_id,
                        error = %error,
                        "could not hand back this worker's job leases on a timed-out drain; \
                         they stay held until a worker's lease reaper frees them"
                    );
                    0
                }
            };
            tracing::warn!(
                worker_id = %worker_id,
                shutdown_grace_seconds = grace.as_secs(),
                released,
                "the shutdown grace period elapsed before in-flight jobs finished; stopped \
                 waiting for them and handed their leases back"
            );
            (Drain::TimedOut, released)
        }
    };

    gauge.abort();
    reaper.abort();
    signal.abort();

    let (claimed, finished, rescheduled, dead_lettered, lost) = counters.snapshot();
    let report = LoopReport {
        drain,
        claimed,
        finished,
        rescheduled,
        dead_lettered,
        lost,
        released,
    };
    tracing::info!(
        worker_id = %worker_id,
        claimed = report.claimed,
        finished = report.finished,
        rescheduled = report.rescheduled,
        dead_lettered = report.dead_lettered,
        lost = report.lost,
        released = report.released,
        drain = ?report.drain,
        "job loop stopped"
    );
    report
}

/// One task: claim, run, settle, repeat, until shutdown.
///
/// Shutdown is checked at the *top* of an iteration and nowhere else, which
/// is what makes a clean drain mean something: a job that has been claimed is
/// always settled, whatever the signal does while it runs.
#[expect(
    clippy::too_many_arguments,
    reason = "one task's share of exactly the list `run_loop` was given; a config struct \
              here would only move the arguments to whatever constructs it"
)]
async fn claim_loop(
    pool: PgPool,
    adapters: Arc<Adapters>,
    rails: Arc<RailConfigs>,
    policy: RecoveryPolicy,
    endpoints: Arc<EndpointRegistry>,
    http: reqwest::Client,
    worker_id: String,
    mut shutdown: watch::Receiver<bool>,
    counters: Arc<Counters>,
) {
    // Built once per task rather than per claim: it borrows two values this
    // task owns for its whole life, so a per-iteration construction would be
    // the same two pointers written out again.
    let webhooks = WebhookContext {
        endpoints: &endpoints,
        http: &http,
    };
    while !*shutdown.borrow() {
        match run_once(&pool, &adapters, &rails, &policy, &webhooks, &worker_id).await {
            Ok(Some(settled)) => counters.record(&settled),
            Ok(None) => idle(&mut shutdown).await,
            Err(error) => {
                // The queue's own statements failed, not a job's work. Back
                // off exactly as for an empty queue: Postgres being briefly
                // unreachable is not a reason to spin on it, and it is not a
                // reason to exit the process either — the pool reconnects.
                let level_is_page = error.severity() == Severity::Page;
                if level_is_page {
                    tracing::error!(
                        alert = true,
                        worker_id = %worker_id,
                        error = %error,
                        "the job queue is not answering; retrying"
                    );
                } else {
                    tracing::error!(
                        worker_id = %worker_id,
                        error = %error,
                        "the job queue is not answering; retrying"
                    );
                }
                idle(&mut shutdown).await;
            }
        }
    }
}

/// Waits [`IDLE_SLEEP`], or until shutdown, whichever comes first.
///
/// `changed()` rather than a second `borrow()` poll: without it a shutdown
/// arriving just after an empty claim would wait out the full second for no
/// reason, on every task, on every deployment roll.
async fn idle(shutdown: &mut watch::Receiver<bool>) {
    tokio::select! {
        // `Err` means the sender is gone, which only happens once shutdown
        // has been sent; either way this future is done waiting.
        _ = shutdown.changed() => {}
        () = tokio::time::sleep(IDLE_SLEEP) => {}
    }
}

/// How often the lease reaper runs: half a lease, floored at
/// [`MIN_REAP_INTERVAL`].
///
/// Half, so a lease stranded by a crash waits at most one further lease
/// before it is claimable again — the reaper can only free a lease that is
/// already older than `lease`, so a period of `lease` would put the worst
/// case at two. Anything much finer buys nothing: the thing being recovered
/// from is a dead process, and the charge behind it is on a poll ladder whose
/// fastest rung is ten seconds.
const fn reap_interval(lease: Duration) -> Duration {
    let half = lease.checked_div(2);
    match half {
        // `Duration` has no `const` `max`, hence the comparison.
        Some(half) if half.as_nanos() > MIN_REAP_INTERVAL.as_nanos() => half,
        _ => MIN_REAP_INTERVAL,
    }
}

/// Frees leases whose worker died, on [`reap_interval`], for as long as this
/// worker runs.
///
/// Separate from `sweep_expired` (which still reaps, along with its two
/// deletes) for the reason [`run_loop`]'s boot reap exists: the sweep is a
/// job, and a job cannot recover the lease on itself.
async fn reaper_loop(pool: PgPool, lease: Duration, worker_id: String) {
    let mut ticker = tokio::time::interval(reap_interval(lease));
    // The first tick fires immediately and would duplicate the boot reap.
    ticker.tick().await;

    loop {
        ticker.tick().await;
        reap_leases(&pool, lease, &worker_id).await;
    }
}

/// One reaping pass, logged only when it found something.
///
/// A failure is logged and swallowed: this is recovery for *other* workers'
/// jobs, and a database blip here must not stop this worker claiming its own.
/// A non-zero count is a `warn` because in a deployment that only ever shuts
/// down gracefully it is zero — every non-zero one is a worker that died or a
/// handler that outran the lease.
async fn reap_leases(pool: &PgPool, lease: Duration, worker_id: &str) {
    match vpay_db::jobs::reap_expired_leases(pool, lease).await {
        Ok(0) => {}
        Ok(reaped) => tracing::warn!(
            worker_id = %worker_id,
            reaped,
            lease_seconds = lease.as_secs(),
            "freed job leases whose worker never came back; they are claimable again"
        ),
        Err(error) => tracing::warn!(
            worker_id = %worker_id,
            error = %error,
            "could not reap expired job leases; jobs held by a dead worker stay unclaimable \
             until the next pass"
        ),
    }
}

/// The periodic gauge line: this worker's tallies, and the queue's age.
///
/// The tallies are cumulative rather than per-interval so a scrape that
/// misses a line loses nothing, and the age comes from
/// `vpay_db::jobs::oldest_runnable_run_at` because it is a property of the
/// table and of every worker against it, not of this one.
///
/// An age drifting steadily into the past is the backlog signal — jobs whose
/// time has come with nobody taking them — and it is the number
/// `--worker-concurrency` (or another replica) exists to move.
///
/// `worker_id` is on the line because every other field except the queue age
/// is *this process's* tally: two replicas emit two of these a minute, and
/// without the identity they sum into a number that describes neither.
///
/// # Two fields are not named what the design named them
///
/// `docs/plans/2026-09-03-step4-worker.md` §5 asks for "claimed / succeeded /
/// rescheduled / dead-lettered / oldest `run_at`". This line writes
/// `finished` where that says `succeeded`, and `queue_behind_seconds` where
/// it says oldest `run_at`. Both are deliberate and both are drift worth
/// naming rather than quietly reconciling:
///
/// * `finished` is [`Disposition::Finished`], which counts a *declined*
///   charge too — the queue is done with it either way. Calling that
///   `succeeded` would read as a payment count, and an operator watching it
///   would be watching the wrong number.
/// * `queue_behind_seconds` is a duration, not an instant. A timestamp would
///   have to be diffed against now by whoever read the line, and the field
///   that alerting actually thresholds is the difference.
async fn gauge_loop(pool: PgPool, worker_id: String, counters: Arc<Counters>) {
    let mut ticker = tokio::time::interval(GAUGE_INTERVAL);
    // The first tick fires immediately and would report an empty run.
    ticker.tick().await;

    loop {
        ticker.tick().await;
        let (claimed, finished, rescheduled, dead_lettered, lost) = counters.snapshot();
        let oldest = vpay_db::jobs::oldest_runnable_run_at(&pool).await;
        let behind_seconds = match &oldest {
            Ok(Some(run_at)) => Some((OffsetDateTime::now_utc() - *run_at).whole_seconds()),
            // No runnable rows at all: the queue is empty, which is a
            // different fact from "zero seconds behind" and is reported as
            // one by leaving the field null.
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(error = %error, "could not read the job queue's age");
                None
            }
        };
        tracing::info!(
            worker_id = %worker_id,
            claimed,
            finished,
            rescheduled,
            dead_lettered,
            lost,
            queue_behind_seconds = behind_seconds,
            "job loop gauge"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three parts, in the order an operator scans them, with the
    /// hostname first because that is what they will `kubectl logs`.
    #[test]
    fn a_worker_id_names_the_host_the_process_and_a_random_tail() {
        let id = worker_id();
        let parts: Vec<&str> = id.split('/').collect();
        let [host, pid, tail] = parts.as_slice() else {
            panic!("expected host/pid/suffix, got `{id}`");
        };
        assert!(!host.is_empty(), "the host field must not be empty");
        assert_eq!(
            *pid,
            std::process::id().to_string(),
            "the middle field must be this process's pid"
        );
        assert_eq!(tail.len(), 8, "the random tail is eight hex characters");
        assert!(tail.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// The random tail is the part that actually carries uniqueness: the
    /// host and pid are identical for a pod that restarts in place.
    #[test]
    fn two_worker_ids_from_one_process_differ() {
        assert_ne!(worker_id(), worker_id());
    }

    /// A guarded write that matched nothing did not do what the loop
    /// intended, and must not be counted as though it had — a `Lost` job is
    /// running somewhere else right now.
    #[test]
    fn a_write_that_matched_no_row_is_lost_whatever_was_intended() {
        assert_eq!(disposition(false, Disposition::Finished), Disposition::Lost);
        assert_eq!(
            disposition(false, Disposition::DeadLettered),
            Disposition::Lost
        );
        assert_eq!(
            disposition(true, Disposition::Rescheduled(IDLE_SLEEP)),
            Disposition::Rescheduled(IDLE_SLEEP)
        );
    }

    /// `attempts` is incremented by the claim, so the run happening now is
    /// attempt N and its ladder rung is N-1. Off by one here and every
    /// retry waits one rung too long, forever.
    #[test]
    fn the_ladder_rung_is_one_below_the_attempt_count() {
        let mut job = JobRow {
            id: Uuid::nil(),
            kind: JobKind::PollCharge.as_wire_str().to_owned(),
            dedupe_key: "poll:ch_x".to_owned(),
            payload: serde_json::Value::Object(serde_json::Map::new()),
            run_at: OffsetDateTime::UNIX_EPOCH,
            attempts: 1,
            locked_by: Some("w".to_owned()),
            last_error: None,
        };
        assert_eq!(attempt_index(&job), 0);
        job.attempts = 7;
        assert_eq!(attempt_index(&job), 6);
        // A row whose counter is somehow zero or negative must not wrap into
        // the far end of the ladder.
        job.attempts = 0;
        assert_eq!(attempt_index(&job), 0);
        job.attempts = -1;
        assert_eq!(attempt_index(&job), u32::MAX.saturating_sub(1));
    }

    /// Half a lease, and never a hot loop. The floor is what a deployment
    /// (or an integration test) with a very short lease actually gets, and a
    /// zero-length interval is a panic in `tokio::time::interval` rather than
    /// a fast timer — which is why the floor exists at all and not merely as
    /// politeness toward Postgres.
    #[test]
    fn the_reaper_runs_at_half_a_lease_and_never_faster_than_the_idle_poll() {
        assert_eq!(
            reap_interval(Duration::from_secs(5 * 60)),
            Duration::from_secs(150),
            "the documented five-minute lease is reaped every 150s"
        );
        assert_eq!(
            reap_interval(Duration::from_secs(4)),
            Duration::from_secs(2)
        );
        assert_eq!(reap_interval(Duration::from_secs(2)), MIN_REAP_INTERVAL);
        assert_eq!(reap_interval(Duration::ZERO), MIN_REAP_INTERVAL);
        assert!(
            !reap_interval(Duration::ZERO).is_zero(),
            "tokio::time::interval panics on a zero period"
        );
        assert_eq!(MIN_REAP_INTERVAL, IDLE_SLEEP);
    }

    /// The gauge is slower than the idle poll by two orders of magnitude, and
    /// the idle poll is faster than the fastest ladder rung. Both are
    /// deliberate (see each constant); this pins the ordering so a change to
    /// one is a change to this test.
    #[test]
    fn the_two_intervals_keep_their_documented_ordering() {
        assert_eq!(IDLE_SLEEP, Duration::from_secs(1));
        assert_eq!(GAUGE_INTERVAL, Duration::from_secs(60));
        assert!(IDLE_SLEEP < crate::poll_delay(0));
        assert!(GAUGE_INTERVAL > IDLE_SLEEP);
    }
}
