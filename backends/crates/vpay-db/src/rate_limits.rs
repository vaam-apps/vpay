//! The `rate_limit_windows` repository
//! (`backends/migrations/0038_create-rate-limit-windows.sql`) — one durable
//! fixed-window counter per budget, so that every replica spends from the
//! same one (issue #79 item 2).
//!
//! # What this replaces, and why the replacement is one statement
//!
//! [ADR-0017](../../../../docs/adr/0017-staff-authentication.md) decision 2
//! limited sign-in **in process**, and its Consequences said what that cost:
//! "Three replicas admit three times the attempts a single one does … the
//! first thing to revisit if a deployment runs many replicas."
//!
//! The obvious durable shape is read-then-write — `SELECT attempts`, decide,
//! `UPDATE`. It is also the shape that does not work: two replicas reading 9
//! both write 10 and both admit, which is the same over-admission the change
//! exists to remove, now with a race in it. [`RateLimits::count_attempt`] is
//! therefore a single `INSERT … ON CONFLICT (id) DO UPDATE … RETURNING
//! attempts`. Postgres takes a row lock on the conflicting row before
//! evaluating the `DO UPDATE`, so two concurrent counts on one key are
//! serialised by the database and each is told its own position in the
//! window.
//!
//! That is also why this module is hand-written `sqlx` on a table
//! `schemas/vpay.cstack` models. `model RateLimitWindow` declares every
//! column, so the drift report compares the table; what it cannot do is
//! render the statement, because a CrateStack 0.12.0 `Update…Input` carries
//! **values** and the increment is an **expression** over the row's own
//! column. The model's own comment says so, and so does migration 0038's
//! header.
//!
//! # The sweep is in the same statement, and that is a bound rather than tidiness
//!
//! Every key on this table is chosen by an unauthenticated caller: the
//! per-email budget of a sign-in is keyed by an address that, on the
//! interesting attempts, has no account. A counter table with no bound is
//! therefore a disk-filling attack with a `401` in front of it.
//!
//! So `count_attempt` deletes up to [`SWEEP_BATCH`] elapsed rows in the same
//! round trip, in a data-modifying CTE. Three details make that safe rather
//! than clever, and each of them is a defect if it is dropped:
//!
//! * **`id <> $1`** — the sweep can never name the row the `INSERT` is
//!   about, so the two halves of the statement cannot interact through the
//!   same snapshot.
//! * **`FOR UPDATE SKIP LOCKED`** — two concurrent counts sweep disjoint
//!   rows, so they cannot take the same locks in a different order. Without
//!   it, two sign-in attempts arriving together could deadlock, and a
//!   deadlock on the sign-in path is a `500` for a person typing a password.
//! * **`LIMIT`** — the work per attempt is bounded whatever the table's
//!   size, so the first attempt after a long idle period does not pay for
//!   every row that accumulated during it.
//!
//! One attempt writes at most two rows (one per dimension) and removes up to
//! thirty-two, so a caller spending fresh keys drains the table faster than
//! they fill it.
//!
//! # What is NOT here
//!
//! No read. Nothing asks "how many attempts has this key made" except the
//! act of making one, and a read-only accessor would be an invitation to the
//! read-then-write this module exists to refuse.
//!
//! No lockout, no per-key ban list, no `staff_members` foreign key. All three
//! are argued in `vpay_api::staff::rate_limit`'s header, and migration 0038
//! records the last one: the commonest key here is an address with no
//! account.

use async_trait::async_trait;
use time::{Duration, OffsetDateTime};

use crate::error::{DbError, classify_write};

/// How many elapsed rows one count removes.
///
/// Thirty-two: sixteen times the two rows a single sign-in attempt adds, so
/// the table drains under exactly the load that fills it, and small enough
/// that the extra work is one bounded index scan on
/// `rate_limit_windows_window_idx`.
const SWEEP_BATCH: i64 = 32;

/// Count one attempt against one key and learn where it lands in the window.
///
/// One method, deliberately: see the module header for why there is no read
/// and no separate "increment".
#[async_trait]
pub trait RateLimits {
    /// Counts one attempt against `id` and answers how many attempts —
    /// **including this one** — have been made in the window that is current
    /// at `now`.
    ///
    /// A window whose start is at or before `now - window` has elapsed and is
    /// *replaced*: the answer is then `1`. That is what makes this fixed
    /// rather than sliding, and it is decided inside the statement so that
    /// two replicas cannot disagree about which window they are in.
    ///
    /// `id` is a digest, not a value. The caller composes and hashes the
    /// pre-image (`vpay_api::staff::rate_limit`), for migration 0038's two
    /// stated reasons: an unauthenticated caller chooses the value, and for
    /// the commonest key it is an email address with no account behind it.
    /// `scope` names the *kind* of budget without the value and must be one
    /// of the three `rate_limit_windows_scope_is_known` admits.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the statement fails — which the caller must
    /// treat as a refusal rather than as an allowance, because a limiter
    /// that fails open is not a limiter.
    async fn count_attempt(
        &self,
        id: &str,
        scope: &str,
        window: Duration,
        now: OffsetDateTime,
    ) -> Result<i64, DbError>;
}

#[async_trait]
impl RateLimits for crate::repository::PgRepositories {
    async fn count_attempt(
        &self,
        id: &str,
        scope: &str,
        window: Duration,
        now: OffsetDateTime,
    ) -> Result<i64, DbError> {
        // `now - window` is the horizon: a window that opened at or before it
        // has elapsed. Computed here, from the caller's `now`, rather than
        // from the database's `now()` — every other bound in this design
        // (`SessionRow::is_live_at`, the TOTP step) is decided from a clock
        // the caller passes in, and a statement that mixed the two would
        // compare one process's instant against another's.
        let horizon = now.saturating_sub(window);

        sqlx::query_scalar::<_, i64>(COUNT_ATTEMPT)
            .bind(id)
            .bind(scope)
            .bind(now)
            .bind(horizon)
            .bind(SWEEP_BATCH)
            .fetch_one(&self.pool)
            .await
            .map_err(classify_write)
    }
}

/// The count, the reset and the sweep, in one statement.
///
/// Read the module header before changing a character of it. The three
/// properties that make it correct — `id <> $1`, `FOR UPDATE SKIP LOCKED`
/// and the `LIMIT` — are each load-bearing and none of them is obvious from
/// the text, which is why
/// [`tests::the_statement_keeps_the_three_properties_that_make_it_safe`]
/// asserts each by name.
///
/// A `const &'static str`, so no `AssertSqlSafe` and no `format!`: there is
/// nothing here for [`crate::sql_audit`] to audit, which is the shape that
/// module's header asks for.
const COUNT_ATTEMPT: &str = "WITH swept AS ( \
                 DELETE FROM rate_limit_windows \
                  WHERE id IN ( \
                        SELECT id FROM rate_limit_windows \
                         WHERE id <> $1 AND window_started_at <= $4 \
                         ORDER BY window_started_at \
                         LIMIT $5 \
                         FOR UPDATE SKIP LOCKED \
                  ) \
                 RETURNING 1 \
             ) \
             INSERT INTO rate_limit_windows (id, scope, window_started_at, attempts, updated_at) \
             VALUES ($1, $2, $3, 1, $3) \
             ON CONFLICT (id) DO UPDATE SET \
                 window_started_at = CASE \
                     WHEN rate_limit_windows.window_started_at <= $4 THEN $3 \
                     ELSE rate_limit_windows.window_started_at END, \
                 attempts = CASE \
                     WHEN rate_limit_windows.window_started_at <= $4 THEN 1 \
                     ELSE rate_limit_windows.attempts + 1 END, \
                 updated_at = $3 \
             RETURNING attempts";

#[cfg(test)]
mod tests {
    use super::*;

    /// `model RateLimitWindow` carries **no `@@allow` arm**, and this pins the
    /// absence.
    ///
    /// Pinned in this direction for `crate::schema`'s money-model reason,
    /// which applies here with one extra turn of the screw: nothing in vpay
    /// reads or writes this table through the generated layer, so an arm
    /// appearing would be a standing permission over a table with no caller
    /// — and, this table being the one an unauthenticated caller populates,
    /// a permission over rows an attacker chose the keys of.
    ///
    /// If the increment ever becomes expressible by a generated builder (see
    /// the module header), move this assertion with the query rather than
    /// deleting it.
    #[test]
    fn the_rate_limit_window_model_answers_no_rows_to_every_action() {
        use crate::schema::cratestack_schema::models::RATE_LIMIT_WINDOW_MODEL as descriptor;

        for (action, policies) in [
            ("read", descriptor.read_allow_policies),
            ("create", descriptor.create_allow_policies),
            ("update", descriptor.update_allow_policies),
            ("delete", descriptor.delete_allow_policies),
        ] {
            assert!(
                policies.is_empty(),
                "model RateLimitWindow grew an @@allow(\"{action}\", …) arm. Nothing queries \
                 this model — `count_attempt` is one hand-written statement, because the \
                 increment is an expression over the row's own column and an Update…Input \
                 carries values. A permission with no caller is one nobody reviewed for a \
                 caller; if a query moved, move this assertion with it"
            );
        }
    }

    /// The three properties that make [`COUNT_ATTEMPT`] safe, each asserted
    /// by name, plus the increment that makes it a counter.
    ///
    /// A test over the statement's *text* because there is no type that
    /// expresses any of them — the same argument [`crate::sql_audit`]'s
    /// header makes for scanning source. Each assertion is a mutation
    /// somebody could plausibly make while tidying:
    ///
    /// * deleting `id <> $1` lets the sweep name the row the `INSERT` is
    ///   about, in one statement, through one snapshot;
    /// * deleting `SKIP LOCKED` lets two concurrent sign-in attempts take the
    ///   same row locks in different orders, which is a deadlock and a `500`
    ///   for a person typing a password;
    /// * deleting the `LIMIT` makes the first attempt after an idle period
    ///   pay for every row that accumulated during it;
    /// * turning `attempts + 1` into a literal makes the limiter count to one
    ///   forever and admit everything.
    #[test]
    fn the_statement_keeps_the_three_properties_that_make_it_safe() {
        for (fragment, why) in [
            (
                "id <> $1",
                "the sweep must never name the row the INSERT is about",
            ),
            (
                "FOR UPDATE SKIP LOCKED",
                "without it two concurrent counts can deadlock on the sweep",
            ),
            (
                "LIMIT $5",
                "the work per attempt must be bounded whatever the table's size",
            ),
            (
                "attempts + 1",
                "a counter that does not increment admits every attempt",
            ),
            (
                "ON CONFLICT (id) DO UPDATE",
                "the row lock this takes is what makes two replicas share one budget",
            ),
            (
                "RETURNING attempts",
                "the caller's verdict is the position of the attempt just counted",
            ),
        ] {
            assert!(
                COUNT_ATTEMPT.contains(fragment),
                "`{fragment}` is gone from the count statement: {why}"
            );
        }
    }
}
