//! [`JobError`], the job loop's Tier-2 composite error, and the retry
//! [`Decision`] every boundary in the worker derives from it.
//!
//! [`crate::run_loop::run_loop`] is the one caller of [`JobError::decision`], and the
//! only place a [`Decision`] becomes a write: `RetryAfter` reschedules,
//! `Terminal` deletes, `DeadLetter` parks. This module's own tests prove
//! exactly one thing — that the decision is a pure function of
//! [`Classify::retry`], [`Classify::severity`] and the two delays
//! ([`crate::poll_delay`], [`crate::UNRESOLVED_POLL_INTERVAL`]), with no
//! second opinion anywhere. That a job is *actually* retried, finished or
//! parked is a claim about Postgres, and it is proven where it can be —
//! `backends/tests/integration/tests/worker_recovery.rs`.
//!
//! Per [ADR-0011](../../../../docs/adr/0011-error-modelling.md), a composite
//! error **never re-classifies** what it wraps: a `DbError` is
//! [`Category::Storage`] whether it surfaces through the HTTP layer or
//! through a job, so every `Classify` method here delegates wholesale for
//! the wrapped variants rather than only forwarding `category()`. Forwarding
//! `category()` alone would silently drop a leaf's deliberate override —
//! `ProviderError::Rejected` overrides `retry()` to [`Retry::NewAttempt`]
//! while its category ([`Category::Conflict`]) defaults to [`Retry::Never`],
//! so a category-only delegation would dead-letter a declined charge instead
//! of letting the intent's own state machine decide.

use std::time::Duration;

use uuid::Uuid;
use vpay_core::{Category, Classify, MoneyError, Retry, Severity};
use vpay_db::DbError;
use vpay_ledger::LedgerError;
use vpay_provider::ProviderError;

/// Everything a job can fail with: the leaves it calls into, plus the two
/// failures that belong to the job *row* rather than to anything it called.
///
/// Not `#[non_exhaustive]`: this is workspace-internal (the SDKs model the
/// wire, not the system — ADR-0011), and an exhaustive `match` on it in the
/// job loop is the point.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// Postgres failed while the job was reading its own row, claiming it,
    /// or writing the result.
    #[error(transparent)]
    Db(#[from] DbError),

    /// A payment rail failed, declined, or is not built yet. Includes
    /// `ProviderError::Rejected`, which is a rail *decision* rather than a
    /// system failure — see this module's header for why that distinction
    /// survives the wrapping.
    #[error(transparent)]
    Provider(#[from] ProviderError),

    /// An amount was negative, mixed currencies, or overflowed while the job
    /// was computing what to write.
    ///
    /// Wrapped even though no job computes anything yet, for the reason
    /// ADR-0011 gives composites: a leaf the layer can meet but does not
    /// carry is a leaf the first real job will be tempted to classify at its
    /// own call site. `Negative`/`CurrencyMismatch` are `InvalidRequest` and
    /// `Overflow` is `Internal`, and the delegation below keeps that split —
    /// both dead-letter, but only one of them pages, and that difference is
    /// the whole reason the composite delegates instead of deciding.
    #[error(transparent)]
    Money(#[from] MoneyError),

    /// A ledger transaction the job built did not balance, or had too few
    /// entries.
    ///
    /// The job loop is what will write ledger transactions (a capture, a
    /// refund), so this is the composite that has to carry `LedgerError`.
    /// `Unbalanced`/`TooFewEntries` page — this code's own invariant failing
    /// in the money path — and the delegation keeps that rather than
    /// flattening it into the queue's idea of a failed job.
    #[error(transparent)]
    Ledger(#[from] LedgerError),

    /// The job row itself is inconsistent: a payload that does not
    /// deserialise into the shape its `kind` promises, a `charge_id` with no
    /// charge behind it, an attempt counter that moved backwards. Retrying
    /// cannot fix data that is already wrong, so this is
    /// [`Category::Internal`] — a bug in whatever wrote the row, and it
    /// pages.
    #[error("job {job_id} is poisoned and cannot be interpreted: {reason}")]
    Poisoned {
        /// The `jobs` row that could not be interpreted.
        job_id: Uuid,
        /// What was inconsistent about it. Operator-facing text, never a
        /// secret and never a payload dump.
        reason: String,
    },

    /// The poll ladder reached its 24-hour horizon without the rail ever
    /// giving a terminal answer — the `unresolved` state of
    /// `docs/flows/reconciler.md`.
    ///
    /// This is deliberately *not* modelled as "the job failed", and just as
    /// deliberately not as a dead-letter. `docs/flows/reconciler.md` says
    /// what `unresolved` means: "still polled, once an hour, and now raising
    /// an alert for a human to reconcile against the rail's settlement
    /// statement", and "a late success — minute 40, or hour 30 from
    /// `unresolved` — is the normal transition". A dead-letter would stop
    /// the polling that exists to catch that late success. So the variant
    /// keeps [`Retry::AfterBackoff`] — at [`crate::UNRESOLVED_POLL_INTERVAL`]
    /// rather than a ladder rung — and raises its severity to
    /// [`Severity::Error`], which is what makes [`Decision::RetryAfter`]
    /// carry `alert: true`: the loop keeps going *and* a human is looking.
    #[error(
        "job {job_id} exhausted the poll ladder after {attempts} attempts with no terminal answer from the rail"
    )]
    Exhausted {
        /// The `jobs` row that ran out of ladder.
        job_id: Uuid,
        /// How many polls were made before giving up. Carried so the
        /// escalation names a number instead of "a lot".
        attempts: u32,
    },
}

impl Classify for JobError {
    fn category(&self) -> Category {
        match self {
            Self::Db(e) => e.category(),
            Self::Provider(e) => e.category(),
            Self::Money(e) => e.category(),
            Self::Ledger(e) => e.category(),
            // An inconsistent job row is an invariant this code was supposed
            // to guarantee when it wrote the row. That is a bug, and
            // `Internal` is the only category that pages.
            Self::Poisoned { .. } => Category::Internal,
            // `Rail`, not `Internal` or `Conflict`: nothing is broken on our
            // side and no caller did anything wrong — the rail simply never
            // answered. The category is honest about whose problem it is;
            // the `severity` override below is what gets it noticed, and the
            // delay override in `retry_delay` is what slows the polling to
            // hourly without ever stopping it.
            Self::Exhausted { .. } => Category::Rail,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Db(e) => e.code(),
            Self::Provider(e) => e.code(),
            Self::Money(e) => e.code(),
            Self::Ledger(e) => e.code(),
            Self::Poisoned { .. } => "job_poisoned",
            // Named after the charge state it corresponds to
            // (`docs/flows/reconciler.md`), not after the category's generic
            // `provider_unavailable`: an operator reading the alert needs to
            // know this is the 24h escalation, not one more rail timeout.
            Self::Exhausted { .. } => "charge_unresolved",
        }
    }

    fn retry(&self) -> Retry {
        match self {
            Self::Db(e) => e.retry(),
            Self::Provider(e) => e.retry(),
            Self::Money(e) => e.retry(),
            Self::Ledger(e) => e.retry(),
            Self::Poisoned { .. } => self.category().default_retry(),
            // `Category::Rail`'s own default, kept rather than overridden.
            // The *ladder* is over; the polling is not
            // (docs/flows/reconciler.md: "still polled, once an hour"). What
            // changes is the delay — see `retry_delay` — and the severity,
            // which is what turns the resulting `Decision::RetryAfter` into
            // an alerting one.
            Self::Exhausted { .. } => self.category().default_retry(),
        }
    }

    fn severity(&self) -> Severity {
        match self {
            Self::Db(e) => e.severity(),
            Self::Provider(e) => e.severity(),
            Self::Money(e) => e.severity(),
            Self::Ledger(e) => e.severity(),
            Self::Poisoned { .. } => self.category().default_severity(),
            // Overrides `Category::Rail`'s `Warn`. A single rail timeout is
            // self-healing and warns; a payment that has been in flight for
            // 24 hours with no answer is not self-healing and needs a person,
            // so it logs at `Error`. Not `Page`: money is not lost and
            // nothing is corrupt, so this is "look at it today", not "wake
            // someone up". `Error` is also the threshold `decision` reads to
            // set `alert: true`, so this line is what puts the escalation in
            // front of a person while the hourly polling continues.
            Self::Exhausted { .. } => Severity::Error,
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::Db(e) => e.public_message(),
            Self::Provider(e) => e.public_message(),
            Self::Money(e) => e.public_message(),
            Self::Ledger(e) => e.public_message(),
            // The job id and the poison reason are operator-facing only; the
            // category's generic sentence is what a merchant may see if this
            // ever reaches a response.
            Self::Poisoned { .. } | Self::Exhausted { .. } => {
                self.category().generic_message().to_owned()
            }
        }
    }
}

/// What the job loop does next with a failed job.
///
/// Three outcomes, because [`Retry`] has three values — this enum exists so
/// the loop matches on an intention rather than re-reading a policy table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Re-run this same job after `delay`, and — when `alert` is set — put
    /// it in front of a human at the same time.
    ///
    /// The two fields are not alternatives. "Keep polling" and "a human
    /// should look" are independent, and the reconciler needs both at once:
    /// an `unresolved` charge is polled hourly *and* alerted on, because a
    /// late success at hour 30 is a normal transition
    /// (`docs/flows/reconciler.md`) and a dead-letter would stop the polling
    /// that catches it. Collapsing them would force a choice between
    /// silently retrying forever and giving up on a payment that is still
    /// live.
    RetryAfter {
        /// How long to wait: [`crate::poll_delay`] of the attempt that just
        /// failed, except for the escalation, which uses
        /// [`crate::UNRESOLVED_POLL_INTERVAL`].
        delay: Duration,
        /// Whether a human is told now rather than at the next review.
        /// Derived from [`Classify::severity`] — `Error` or above — so the
        /// loop never decides on its own what is worth waking someone for.
        alert: bool,
    },
    /// This job is over and must not be re-run as-is. Whatever comes next is
    /// a *new* attempt decided by the intent's own state machine
    /// (`docs/flows/payment-lifecycle.md`: retry means a new
    /// `PaymentIntent`), not by the queue.
    Terminal,
    /// Park it for a human, and stop: **nothing the loop can do will change
    /// the outcome**. A poisoned job row, an unimplemented adapter, a broken
    /// migration — re-running any of them produces the same failure. This is
    /// the shape of "give up", which is why the 24-hour escalation
    /// deliberately is not one: that charge can still succeed.
    DeadLetter,
}

impl JobError {
    /// What the loop should do next, given that this error came from
    /// attempt number `attempt` (0-indexed, same indexing as
    /// [`crate::poll_delay`]).
    ///
    /// Derived from [`Classify::retry`] and [`Classify::severity`] and
    /// nothing else — deliberately no `match` on `self`'s variants here.
    /// That is the whole point of ADR-0011's "classified once": if the
    /// worker branched on variants it would grow a second retry policy that
    /// could drift from the API's, and the two boundaries could disagree
    /// about whether a `DbError` is transient. The `match` below is
    /// exhaustive over [`Retry`] with no wildcard, so adding a fourth retry
    /// value fails to compile here rather than silently falling into a
    /// default.
    #[must_use]
    pub fn decision(&self, attempt: u32) -> Decision {
        match self.retry() {
            Retry::AfterBackoff => Decision::RetryAfter {
                delay: self.retry_delay(attempt),
                // `Error` is the documented line between "counted" and
                // "someone looks" (`vpay_core::Severity`): a rail timeout
                // (`Warn`) rides the ladder quietly, a Postgres failure or
                // the 24-hour escalation (`Error`) does not.
                alert: self.severity() >= Severity::Error,
            },
            Retry::NewAttempt => Decision::Terminal,
            Retry::Never => Decision::DeadLetter,
        }
    }

    /// How long to wait before re-running, for the errors that are re-run.
    ///
    /// The poll ladder for everything transient, and the flat hourly
    /// [`crate::UNRESOLVED_POLL_INTERVAL`] for the escalation. This is the
    /// one variant-level `match` in this file, and it is here rather than in
    /// [`Self::decision`] on purpose: it decides *pacing*, not policy. The
    /// policy — retry at all? alert? — still comes only from `Classify`, so
    /// the API and the worker cannot disagree about it; a ladder rung is
    /// meaningless to the API and belongs to the worker alone.
    fn retry_delay(&self, attempt: u32) -> Duration {
        match self {
            // The ladder has already run out — that is what this variant
            // means — so continuing to index it would return its last rung
            // (15 minutes) forever. `docs/flows/reconciler.md` says hourly.
            Self::Exhausted { .. } => crate::UNRESOLVED_POLL_INTERVAL,
            Self::Db(_)
            | Self::Provider(_)
            | Self::Money(_)
            | Self::Ledger(_)
            | Self::Poisoned { .. } => crate::poll_delay(attempt),
        }
    }
}

/// Translates a [`Severity`] into the `tracing` level a job loop logs at.
///
/// Lives here rather than in `vpay-core` because that crate deliberately
/// depends on no framework, `tracing` included — the translation belongs at
/// the boundary that owns a subscriber.
///
/// [`Severity::Page`] and [`Severity::Error`] both map to
/// [`tracing::Level::ERROR`] because `tracing` has no fifth level. They are
/// **not** the same thing, and the difference must not be lost: the job loop
/// is expected to add an `alert = true` field to the event when
/// `severity() == Severity::Page`, so an alerting rule can select pages
/// without also firing on every ordinary error. This function cannot add
/// that field for the caller — a `Level` is a value, an event field is
/// emitted at the `tracing::error!` call site — so the loop must do it, and
/// this comment is the reason it has to.
#[must_use]
pub fn tracing_level(sev: Severity) -> tracing::Level {
    match sev {
        Severity::Info => tracing::Level::INFO,
        Severity::Warn => tracing::Level::WARN,
        Severity::Error | Severity::Page => tracing::Level::ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two ladder rungs the table below pins, from
    /// `docs/flows/reconciler.md`: attempt 0 is the first, fastest poll and
    /// attempt 6 is the first rung of the flat 120s stretch. Picking one
    /// value from each shape of the ladder is what makes the assertion
    /// "`decision` uses `poll_delay(attempt)`" rather than "`decision`
    /// returns some constant".
    const FIRST_RUNG: Duration = Duration::from_secs(10);
    const FLAT_RUNG: Duration = Duration::from_secs(120);

    /// `docs/flows/reconciler.md`'s "still polled, once an hour". Spelled
    /// out here rather than reused from `crate::UNRESOLVED_POLL_INTERVAL` so
    /// the test states the document's number instead of agreeing with the
    /// implementation about whatever number it chose.
    const HOURLY: Duration = Duration::from_secs(60 * 60);

    /// Retryable *and* worth telling a human about — `Severity::Error` or
    /// above. The two `Storage` rows and the escalation.
    const fn retry_alerting(delay: Duration) -> Decision {
        Decision::RetryAfter { delay, alert: true }
    }

    /// Retryable and self-healing: it rides the ladder without waking
    /// anyone. `Severity::Warn` or below.
    const fn retry_quietly(delay: Duration) -> Decision {
        Decision::RetryAfter {
            delay,
            alert: false,
        }
    }

    fn job_id() -> Uuid {
        Uuid::nil()
    }

    /// One case per wrapped leaf plus both job-level variants, each pinning
    /// the decision at attempt 0 and attempt 6. `expected_at_0`/`_at_6`
    /// differ only for the retryable rows, which is exactly the property
    /// under test.
    struct Case {
        name: &'static str,
        error: JobError,
        at_0: Decision,
        at_6: Decision,
        severity: Severity,
    }

    fn table() -> Vec<Case> {
        vec![
            Case {
                // `sqlx::Error::PoolTimedOut` is a real sqlx variant, not a
                // stand-in: `DbError::Connect` is what `vpay_db::connect`
                // returns when the acquire timeout elapses.
                name: "Db(Connect) — Storage is severity Error, so it rides the ladder *and* alerts",
                error: JobError::Db(DbError::Connect(sqlx::Error::PoolTimedOut)),
                at_0: retry_alerting(FIRST_RUNG),
                at_6: retry_alerting(FLAT_RUNG),
                severity: Severity::Error,
            },
            Case {
                name: "Db(Migrate) — Configuration, a broken deploy, never retried",
                error: JobError::Db(DbError::Migrate(
                    sqlx::migrate::MigrateError::VersionMissing(1),
                )),
                at_0: Decision::DeadLetter,
                at_6: Decision::DeadLetter,
                severity: Severity::Error,
            },
            Case {
                name: "Provider(Transport) — Rail is severity Warn: rides the ladder, wakes nobody",
                error: JobError::Provider(ProviderError::transport("connection reset".to_owned())),
                at_0: retry_quietly(FIRST_RUNG),
                at_6: retry_quietly(FLAT_RUNG),
                severity: Severity::Warn,
            },
            Case {
                name: "Provider(Rejected) — a rail decision: terminal here, new intent elsewhere",
                error: JobError::Provider(ProviderError::Rejected {
                    code: vpay_core::FailureCode::InsufficientFunds,
                    message: "balance too low".to_owned(),
                }),
                at_0: Decision::Terminal,
                at_6: Decision::Terminal,
                severity: Severity::Info,
            },
            Case {
                name: "Provider(NotImplemented) — an honest stub never retries itself into working",
                // A real, already-declared token from
                // `vpay-adapter-mtn-momo` (and so already listed in
                // docs/status.md, which `cargo xtask verify-status` checks),
                // not an invented one: this is literally what a submit job
                // would get back from that adapter today.
                error: JobError::Provider(ProviderError::NotImplemented("mtn_momo::submit")),
                at_0: Decision::DeadLetter,
                at_6: Decision::DeadLetter,
                severity: Severity::Error,
            },
            Case {
                name: "Poisoned — our bug, pages, a human looks",
                error: JobError::Poisoned {
                    job_id: job_id(),
                    reason: "payload does not match kind".to_owned(),
                },
                at_0: Decision::DeadLetter,
                at_6: Decision::DeadLetter,
                severity: Severity::Page,
            },
            Case {
                name: "Exhausted — the 24h `unresolved` escalation: hourly polling *and* an alert",
                error: JobError::Exhausted {
                    job_id: job_id(),
                    attempts: 64,
                },
                // Flat at both ends, unlike every other retryable row: the
                // ladder is over, so the attempt number no longer moves the
                // delay.
                at_0: retry_alerting(HOURLY),
                at_6: retry_alerting(HOURLY),
                severity: Severity::Error,
            },
            Case {
                name: "Money(Negative) — a caller's amount: nothing to retry, and not our bug",
                error: JobError::Money(vpay_core::MoneyError::Negative(-1)),
                at_0: Decision::DeadLetter,
                at_6: Decision::DeadLetter,
                severity: Severity::Info,
            },
            Case {
                name: "Money(Overflow) — same enum, our arithmetic: pages",
                error: JobError::Money(vpay_core::MoneyError::Overflow),
                at_0: Decision::DeadLetter,
                at_6: Decision::DeadLetter,
                severity: Severity::Page,
            },
            Case {
                name: "Ledger(Money) — a caller's amount reaching the ledger: not our bug, does not page",
                error: JobError::Ledger(LedgerError::Money(MoneyError::Negative(-1))),
                at_0: Decision::DeadLetter,
                at_6: Decision::DeadLetter,
                severity: Severity::Info,
            },
            Case {
                name: "Ledger(Unbalanced) — the money path's own invariant broke: pages",
                error: JobError::Ledger(LedgerError::Unbalanced {
                    debits: 5_000,
                    credits: 4_900,
                }),
                at_0: Decision::DeadLetter,
                at_6: Decision::DeadLetter,
                severity: Severity::Page,
            },
        ]
    }

    #[test]
    fn the_decision_table_holds_at_both_ends_of_the_poll_ladder() {
        for case in table() {
            assert_eq!(
                case.error.decision(0),
                case.at_0,
                "{}: wrong decision at attempt 0",
                case.name
            );
            assert_eq!(
                case.error.decision(6),
                case.at_6,
                "{}: wrong decision at attempt 6",
                case.name
            );
            assert_eq!(
                case.error.severity(),
                case.severity,
                "{}: wrong severity",
                case.name
            );
        }
    }

    /// The decision is a function of `Classify::retry()` alone. Asserting it
    /// directly (rather than only through the table) is what fails if
    /// `decision` ever grows a `match self { .. }` of its own.
    #[test]
    fn every_decision_agrees_with_the_errors_own_retry_policy() {
        for case in table() {
            let expected = match case.error.retry() {
                Retry::AfterBackoff => Decision::RetryAfter {
                    // The escalation is the one row whose delay is not a
                    // ladder rung; everything else retries on the ladder.
                    delay: match case.error {
                        JobError::Exhausted { .. } => HOURLY,
                        _ => crate::poll_delay(3),
                    },
                    alert: case.error.severity() >= Severity::Error,
                },
                Retry::NewAttempt => Decision::Terminal,
                Retry::Never => Decision::DeadLetter,
            };
            assert_eq!(case.error.decision(3), expected, "{}", case.name);
        }
    }

    /// A table that only ever exercised one or two `Retry` values would pass
    /// while leaving a whole branch of `decision` unproven.
    #[test]
    fn the_table_exercises_all_three_retry_values() {
        let seen: Vec<Retry> = table().iter().map(|c| c.error.retry()).collect();
        for r in [Retry::Never, Retry::AfterBackoff, Retry::NewAttempt] {
            assert!(seen.contains(&r), "no case in the table produces {r:?}");
        }
    }

    /// Wrapping must not change the answer. This is ADR-0011's "composites
    /// do not re-classify" as an executable assertion: it fails if a
    /// `Classify` method here delegates `category()` but forgets one of the
    /// leaf's deliberate overrides.
    #[test]
    fn wrapping_a_leaf_preserves_every_classification_the_leaf_chose() {
        let leaf = ProviderError::Rejected {
            code: vpay_core::FailureCode::PayerDeclined,
            message: "payer said no".to_owned(),
        };
        let wrapped = JobError::Provider(ProviderError::Rejected {
            code: vpay_core::FailureCode::PayerDeclined,
            message: "payer said no".to_owned(),
        });
        assert_eq!(wrapped.category(), leaf.category());
        assert_eq!(wrapped.code(), leaf.code());
        assert_eq!(wrapped.retry(), leaf.retry());
        assert_eq!(wrapped.severity(), leaf.severity());
        assert_eq!(wrapped.public_message(), leaf.public_message());

        let db_leaf = DbError::Connect(sqlx::Error::PoolTimedOut);
        let db_wrapped = JobError::Db(DbError::Connect(sqlx::Error::PoolTimedOut));
        assert_eq!(db_wrapped.category(), db_leaf.category());
        assert_eq!(db_wrapped.code(), db_leaf.code());
        assert_eq!(db_wrapped.retry(), db_leaf.retry());
        assert_eq!(db_wrapped.severity(), db_leaf.severity());

        // Neither `MoneyError` nor `LedgerError` is `Clone` (deliberately —
        // an error is moved, not copied around), so each case names its leaf
        // twice rather than cloning.
        macro_rules! assert_delegates {
            ($wrapped:expr, $leaf:expr) => {{
                let wrapped: JobError = $wrapped;
                let leaf = $leaf;
                let label = format!("{leaf:?}");
                assert_eq!(wrapped.category(), leaf.category(), "{label}: category");
                assert_eq!(wrapped.code(), leaf.code(), "{label}: code");
                assert_eq!(wrapped.retry(), leaf.retry(), "{label}: retry");
                assert_eq!(wrapped.severity(), leaf.severity(), "{label}: severity");
                assert_eq!(
                    wrapped.public_message(),
                    leaf.public_message(),
                    "{label}: public message"
                );
            }};
        }

        // `MoneyError`, one variant per side of its split: `Overflow` is our
        // arithmetic and pages, `Negative` is the caller's and overrides its
        // `code` to `amount_negative`. A delegation that forwarded the
        // category and let the trait defaults fill in the rest would answer
        // `invalid_request` and lose the field a merchant needs.
        assert_delegates!(JobError::Money(MoneyError::Overflow), MoneyError::Overflow);
        assert_delegates!(
            JobError::Money(MoneyError::Negative(-1)),
            MoneyError::Negative(-1)
        );
        assert_ne!(
            MoneyError::Negative(-1).code(),
            Category::InvalidRequest.default_code(),
            "the assertions above only bite because this leaf overrides its code"
        );

        // `LedgerError`, including the variant that delegates onward to
        // `MoneyError`. Flattening the enum to `Internal` — right for the
        // other two variants — would answer 500 and page for a caller's bad
        // amount.
        assert_delegates!(
            JobError::Ledger(LedgerError::TooFewEntries),
            LedgerError::TooFewEntries
        );
        assert_delegates!(
            JobError::Ledger(LedgerError::Money(MoneyError::Negative(-1))),
            LedgerError::Money(MoneyError::Negative(-1))
        );
        assert_ne!(
            LedgerError::Money(MoneyError::Negative(-1)).category(),
            LedgerError::TooFewEntries.category(),
            "the two ledger cases must disagree, or flattening the enum would pass"
        );
    }

    #[test]
    fn from_conversions_exist_for_every_wrapped_leaf() {
        let from_db: JobError = DbError::Connect(sqlx::Error::PoolTimedOut).into();
        assert!(matches!(from_db, JobError::Db(_)));
        let from_provider: JobError = ProviderError::Unsupported.into();
        assert!(matches!(from_provider, JobError::Provider(_)));
        let from_money: JobError = MoneyError::Overflow.into();
        assert!(matches!(from_money, JobError::Money(_)));
        let from_ledger: JobError = LedgerError::TooFewEntries.into();
        assert!(matches!(from_ledger, JobError::Ledger(_)));
        // `LedgerError::Money` is itself a `#[from]` of `MoneyError`, so the
        // two conversions above are genuinely different paths into the
        // composite rather than one shadowing the other.
        let nested: JobError = LedgerError::Money(MoneyError::Overflow).into();
        assert!(matches!(nested, JobError::Ledger(LedgerError::Money(_))));
    }

    /// `docs/flows/reconciler.md`, in one test: "the charge moves to
    /// `unresolved`: still polled, once an hour, and now raising an alert
    /// for a human … A late success — minute 40, or hour 30 from
    /// `unresolved` — is the normal transition."
    ///
    /// A dead-letter would satisfy "alert a human" and quietly break "still
    /// polled", and the charge that succeeds at hour 30 would never be seen
    /// to succeed. Both halves are asserted here for that reason.
    #[test]
    fn exhausted_keeps_polling_hourly_and_alerts_rather_than_giving_up() {
        let e = JobError::Exhausted {
            job_id: job_id(),
            attempts: 96,
        };
        // Rail, because the rail is the one that never answered.
        assert_eq!(e.category(), Category::Rail);
        // Retry policy is the category's own: this is not "stop", it is
        // "slow down".
        assert_eq!(Category::Rail.default_retry(), Retry::AfterBackoff);
        assert_eq!(e.retry(), Retry::AfterBackoff);
        // Severity *is* overridden: a single rail timeout warns, 24 hours of
        // silence needs a person.
        assert_eq!(Category::Rail.default_severity(), Severity::Warn);
        assert_eq!(e.severity(), Severity::Error);

        // Both halves, and the delay is the document's hour rather than the
        // ladder's last rung, at every attempt number.
        for attempt in [0, 6, 96, 10_000] {
            assert_eq!(
                e.decision(attempt),
                Decision::RetryAfter {
                    delay: HOURLY,
                    alert: true,
                },
                "attempt {attempt}"
            );
        }
        assert_ne!(
            HOURLY,
            crate::poll_delay(96),
            "the ladder's last rung is 15 minutes; the escalation must not silently reuse it"
        );
        assert_ne!(
            e.decision(0),
            Decision::DeadLetter,
            "a dead-letter would stop the polling that catches the hour-30 success"
        );
        // The number reaches the operator, not just "a lot".
        assert!(e.to_string().contains("96"), "{e}");
    }

    #[test]
    fn displays_name_the_job_and_never_hide_the_leaf() {
        let poisoned = JobError::Poisoned {
            job_id: job_id(),
            reason: "no charge for charge_id".to_owned(),
        };
        assert!(poisoned.to_string().contains("no charge for charge_id"));
        // `#[error(transparent)]`: the leaf's own message is what an operator
        // reads, with no "job error:" prefix in front of it.
        let wrapped = JobError::Provider(ProviderError::Unsupported);
        assert_eq!(
            wrapped.to_string(),
            ProviderError::Unsupported.to_string(),
            "a transparent wrapper must not restate the leaf"
        );
    }

    #[test]
    fn tracing_levels_collapse_page_onto_error() {
        assert_eq!(tracing_level(Severity::Info), tracing::Level::INFO);
        assert_eq!(tracing_level(Severity::Warn), tracing::Level::WARN);
        assert_eq!(tracing_level(Severity::Error), tracing::Level::ERROR);
        // Documented collapse: `tracing` has no level above ERROR, so a
        // `Page` is distinguished by the `alert` field the loop adds, not by
        // the level. See `tracing_level`'s own doc comment.
        assert_eq!(tracing_level(Severity::Page), tracing::Level::ERROR);
    }
}
