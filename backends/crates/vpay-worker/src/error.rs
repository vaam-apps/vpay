//! [`JobError`], the job loop's Tier-2 composite error, and the retry
//! [`Decision`] every boundary in the worker derives from it.
//!
//! **There is no job loop yet** (`docs/status.md`): nothing in this workspace
//! dequeues a job, so nothing calls [`JobError::decision`] in production
//! today. This module is the *contract* Phase 5 builds against, and its tests
//! prove exactly one thing — that the retry decision is a pure function of
//! [`Classify::retry`] and [`crate::poll_delay`], with no second opinion
//! anywhere. It does not prove that a job runs, is retried, or is
//! dead-lettered, because none of that exists.
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
use vpay_core::{Category, Classify, Retry, Severity};
use vpay_db::DbError;
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
    /// This is deliberately *not* modelled as "the job failed". The payment
    /// is not lost, it is escalated: the charge stays polled (hourly), the
    /// intent stays `processing`, and a human reconciles it against the
    /// rail's settlement statement. Encoding it as a `JobError` with
    /// [`Retry::Never`] is what routes it to [`Decision::DeadLetter`] — the
    /// queue's "a human looks at this" lane — rather than letting it churn
    /// on the ladder forever or, far worse, be silently failed.
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
            // An inconsistent job row is an invariant this code was supposed
            // to guarantee when it wrote the row. That is a bug, and
            // `Internal` is the only category that pages.
            Self::Poisoned { .. } => Category::Internal,
            // `Rail`, not `Internal` or `Conflict`: nothing is broken on our
            // side and no caller did anything wrong — the rail simply never
            // answered. The category is honest about whose problem it is;
            // the `retry`/`severity` overrides below are what make it stop
            // being retried and start being noticed.
            Self::Exhausted { .. } => Category::Rail,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Db(e) => e.code(),
            Self::Provider(e) => e.code(),
            Self::Poisoned { .. } => "job_poisoned",
            // Named after the charge state it corresponds to
            // (`docs/flows/reconciler.md`), not after the category's generic
            // `provider_unavailable`: an operator reading a dead-letter needs
            // to know this is the 24h escalation, not one more rail timeout.
            Self::Exhausted { .. } => "charge_unresolved",
        }
    }

    fn retry(&self) -> Retry {
        match self {
            Self::Db(e) => e.retry(),
            Self::Provider(e) => e.retry(),
            Self::Poisoned { .. } => self.category().default_retry(),
            // Overrides `Category::Rail`'s `AfterBackoff`. The ladder is
            // already over — that is the entire meaning of this variant — so
            // "retry after backoff" would be a loop with no exit. `Never`
            // routes it to `Decision::DeadLetter`, where a human picks it up.
            Self::Exhausted { .. } => Retry::Never,
        }
    }

    fn severity(&self) -> Severity {
        match self {
            Self::Db(e) => e.severity(),
            Self::Provider(e) => e.severity(),
            Self::Poisoned { .. } => self.category().default_severity(),
            // Overrides `Category::Rail`'s `Warn`. A single rail timeout is
            // self-healing and warns; a payment that has been in flight for
            // 24 hours with no answer is not self-healing and needs a person,
            // so it logs at `Error`. Not `Page`: money is not lost and
            // nothing is corrupt, so this is "look at it today", not "wake
            // someone up".
            Self::Exhausted { .. } => Severity::Error,
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::Db(e) => e.public_message(),
            Self::Provider(e) => e.public_message(),
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
    /// Re-run this same job after the given delay, which is always
    /// [`crate::poll_delay`] of the attempt that just failed
    /// (`docs/flows/reconciler.md`'s ladder).
    RetryAfter(Duration),
    /// This job is over and must not be re-run as-is. Whatever comes next is
    /// a *new* attempt decided by the intent's own state machine
    /// (`docs/flows/payment-lifecycle.md`: retry means a new
    /// `PaymentIntent`), not by the queue.
    Terminal,
    /// Park it for a human. Nothing the loop can do will change the outcome.
    DeadLetter,
}

impl JobError {
    /// What the loop should do next, given that this error came from
    /// attempt number `attempt` (0-indexed, same indexing as
    /// [`crate::poll_delay`]).
    ///
    /// Derived from [`Classify::retry`] and nothing else — deliberately no
    /// `match` on `self`'s variants here. That is the whole point of
    /// ADR-0011's "classified once": if the worker branched on variants it
    /// would grow a second retry policy that could drift from the API's, and
    /// the two boundaries could disagree about whether a `DbError` is
    /// transient. The `match` below is exhaustive over [`Retry`] with no
    /// wildcard, so adding a fourth retry value fails to compile here rather
    /// than silently falling into a default.
    #[must_use]
    pub fn decision(&self, attempt: u32) -> Decision {
        match self.retry() {
            Retry::AfterBackoff => Decision::RetryAfter(crate::poll_delay(attempt)),
            Retry::NewAttempt => Decision::Terminal,
            Retry::Never => Decision::DeadLetter,
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
                name: "Db(Connect) — Storage, transient, rides the ladder",
                error: JobError::Db(DbError::Connect(sqlx::Error::PoolTimedOut)),
                at_0: Decision::RetryAfter(FIRST_RUNG),
                at_6: Decision::RetryAfter(FLAT_RUNG),
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
                name: "Provider(Transport) — Rail, transient, rides the ladder",
                error: JobError::Provider(ProviderError::Transport("connection reset".to_owned())),
                at_0: Decision::RetryAfter(FIRST_RUNG),
                at_6: Decision::RetryAfter(FLAT_RUNG),
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
                name: "Exhausted — the 24h `unresolved` escalation, never silently failed",
                error: JobError::Exhausted {
                    job_id: job_id(),
                    attempts: 64,
                },
                at_0: Decision::DeadLetter,
                at_6: Decision::DeadLetter,
                severity: Severity::Error,
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
                Retry::AfterBackoff => Decision::RetryAfter(crate::poll_delay(3)),
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
    }

    #[test]
    fn from_conversions_exist_for_both_wrapped_leaves() {
        let from_db: JobError = DbError::Connect(sqlx::Error::PoolTimedOut).into();
        assert!(matches!(from_db, JobError::Db(_)));
        let from_provider: JobError = ProviderError::Unsupported.into();
        assert!(matches!(from_provider, JobError::Provider(_)));
    }

    #[test]
    fn exhausted_escalates_rather_than_retrying_or_failing_silently() {
        let e = JobError::Exhausted {
            job_id: job_id(),
            attempts: 96,
        };
        // Rail, because the rail is the one that never answered...
        assert_eq!(e.category(), Category::Rail);
        // ...but explicitly not the category's default retry/severity: the
        // ladder is over, and 24 hours of silence needs a person.
        assert_eq!(Category::Rail.default_retry(), Retry::AfterBackoff);
        assert_eq!(e.retry(), Retry::Never);
        assert_eq!(Category::Rail.default_severity(), Severity::Warn);
        assert_eq!(e.severity(), Severity::Error);
        assert_eq!(e.decision(0), Decision::DeadLetter);
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
