//! [`client_assertion_store`] — Postgres-backed replay protection for
//! `private_key_jwt` client assertions (RFC 7523 §3 point 7), against
//! `oauth_client_assertion_jtis` (`backends/migrations/0011_create-oauth-
//! client-assertion-jtis.sql`).
//!
//! `authkestra-op` ships exactly two implementations of
//! `authkestra_op::client_assertion::ClientAssertionStore`, and neither fits
//! vpay's deployment: `NoClientAssertionStore` is the crate's own fail-closed
//! default, refusing every assertion outright; `MemoryClientAssertionStore`
//! is a single-process `Mutex<HashMap>` whose own doc comment names exactly
//! vpay's situation as the case it does not cover — "a multi-node deployment
//! gets one accepted replay per node; such a deployment must supply a store
//! backed by something shared (Redis `SET NX`, a SQL unique index) instead."
//! vpay runs multiple replicas on Kubernetes, so this is that SQL-unique-
//! index store.

use async_trait::async_trait;
use authkestra_op::client_assertion::ClientAssertionStore;
use authkestra_op::error::OpError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::time::Duration;
use time::OffsetDateTime;

use crate::error::DbError;

/// A Postgres-backed [`ClientAssertionStore`] over `pool`, durable and shared
/// across every vpay replica.
///
/// Returns `impl ClientAssertionStore` rather than the concrete type, and that
/// is the whole point of the function existing: ADR-0016 standard 5 says a
/// repository implementation is private to this crate and reached only through
/// its trait, and until 2026-09-05 this one was the exception — `vpay-api`
/// wrote `SqlClientAssertionStore::new(pool)` by name. A caller now gets the
/// behaviour and has no way to spell the type, which is what `connect`
/// already does for [`crate::Repositories`].
///
/// A free function rather than a method on [`crate::Repositories`]: this store
/// implements a *foreign* trait (`authkestra_op`'s), and putting an
/// authkestra type in vpay's own umbrella trait would make every consumer of
/// `Repositories` — the worker included, which mints no tokens — depend on the
/// OP's vocabulary.
///
/// `PgPool` is a cheap `Arc`-backed handle (per `sqlx`'s own docs), so this
/// opens no connection; it clones the handle in.
#[must_use]
pub fn client_assertion_store(pool: PgPool) -> impl ClientAssertionStore {
    SqlClientAssertionStore { pool }
}

/// The only [`ClientAssertionStore`] in the workspace.
///
/// `pub(crate)`: [`client_assertion_store`] is the whole public surface, so
/// no caller outside this crate can name the implementation or substitute
/// another one for it (ADR-0016 standard 5, ADR-0006).
#[derive(Debug, Clone)]
pub(crate) struct SqlClientAssertionStore {
    pool: PgPool,
}

/// Converts `authkestra-op`'s `chrono::DateTime<Utc>` into the `time::
/// OffsetDateTime` vpay's own convention (and every other TIMESTAMPTZ bind
/// in this crate) uses, so this is the one place that boundary is crossed.
///
/// Exact for every representable instant: both types model the same instant
/// in UTC, so the conversion is a lossless reinterpretation via the Unix
/// timestamp, not an approximation — `time::OffsetDateTime::UNIX_EPOCH` and
/// `chrono`'s epoch are the same instant, and both crates count elapsed
/// seconds/nanoseconds from it. The two failure paths below are not really
/// about precision loss:
///
/// - `from_unix_timestamp` rejects a `chrono::DateTime<Utc>` so far outside
///   `time::OffsetDateTime`'s representable range that it cannot happen from
///   a legitimate assertion — `authkestra_op`'s own
///   `MAX_CLIENT_ASSERTION_LIFETIME_SECS` (300s) already bounds `expires_at`
///   to a few minutes from now before this function is ever reached.
/// - `replace_nanosecond` can only fail on a value in chrono's leap-second
///   range (`1_000_000_000..=1_999_999_999`, chrono's own documented
///   representation for `:60` — `time` has no leap-second slot at all). The
///   clamp below maps that to the last representable nanosecond of the same
///   second rather than failing replay protection over a leap second, which
///   changes nothing about whether the jti has been spent.
fn chrono_to_offset_date_time(dt: DateTime<Utc>) -> Result<OffsetDateTime, OpError> {
    let without_nanos = OffsetDateTime::from_unix_timestamp(dt.timestamp()).map_err(|error| {
        tracing::error!(
            %error,
            "client assertion expires_at is out of range for time::OffsetDateTime"
        );
        OpError::Storage
    })?;

    let nanos = dt.timestamp_subsec_nanos().min(999_999_999);
    without_nanos.replace_nanosecond(nanos).map_err(|error| {
        tracing::error!(
            %error,
            "client assertion expires_at nanosecond component is invalid"
        );
        OpError::Storage
    })
}

#[async_trait]
impl ClientAssertionStore for SqlClientAssertionStore {
    /// Atomically records `jti` as spent until `expires_at`.
    ///
    /// The `INSERT` itself is the atomic guard (migration `0011`'s own
    /// header comment, and the trait's own doc comment on `record_jti`):
    /// never check-then-insert, since two concurrent presentations of the
    /// same captured assertion would both observe "not yet seen" under that
    /// pattern — precisely the race this store exists to close.
    /// `rows_affected() == 1` after `ON CONFLICT (jti) DO NOTHING` means this
    /// was the row's first insert (fresh, accept); `0` means the conflict
    /// clause fired because the row already existed (replay, reject).
    ///
    /// # Errors
    ///
    /// Returns `OpError::Storage` — opaque by the trait's own design, so a
    /// SQL failure never leaks into an OAuth error response — if the
    /// `expires_at` conversion or the query itself fails. The real cause is
    /// logged via `tracing::error!` before being discarded into the opaque
    /// variant.
    async fn record_jti(&self, jti: &str, expires_at: DateTime<Utc>) -> Result<bool, OpError> {
        let expires_at = chrono_to_offset_date_time(expires_at)?;

        let result = sqlx::query(
            "INSERT INTO oauth_client_assertion_jtis (jti, expires_at) VALUES ($1, $2) \
             ON CONFLICT (jti) DO NOTHING",
        )
        .bind(jti)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|error| {
            tracing::error!(%error, jti, "failed to record client assertion jti");
            OpError::Storage
        })?;

        Ok(result.rows_affected() == 1)
    }
}

#[async_trait::async_trait]
pub trait ClientAssertions: Send + Sync {
    /// Deletes every spent `jti` whose assertion expired more than
    /// `retain_after_exp` ago, **on the database's clock**, returning how many
    /// rows went.
    ///
    /// The statement is `expires_at < now() - retain_after_exp`, with the
    /// duration bound as whole microseconds and multiplied by
    /// `INTERVAL '1 microsecond'` ([`crate::jobs`]'s idiom, for its reason: it
    /// saturates rather than refusing a sub-microsecond value). It takes a
    /// relative [`Duration`] and no instant, so the caller's clock cannot
    /// reach the comparison
    /// ([ADR-0026](../../../../docs/adr/0026-the-database-clock-schedules-jobs.md)
    /// D1).
    ///
    /// # Why a row may not be deleted the moment its `exp` passes
    ///
    /// `expires_at` is the client's own `exp`, stored as it arrived
    /// ([`ClientAssertionStore::record_jti`] is handed it by
    /// `authkestra-op`, and this crate keeps it a *fact*: ADR-0026 D6). It is
    /// **not** the last instant the API will accept the assertion. The
    /// validator (`authkestra-op` 0.7.1, `verify_client_assertion`) leaves
    /// `jsonwebtoken`'s `Validation::leeway` at its 60-second default, which
    /// vpay cannot configure, so an assertion is still accepted while
    /// `api_now_secs <= exp + 60` — `now` truncated to whole seconds, hence
    /// up to 61 seconds after `exp` in real time. A row deleted at
    /// `expires_at < now()` could therefore be gone while the same assertion
    /// still verified, and the next presentation would find the `jti` unspent
    /// and be accepted: one replay of a captured, already-used assertion,
    /// minting a merchant access token. The window is
    /// `max(0, 61 s + db_clock_offset − api_clock_offset)` after `exp`, and it
    /// only matters when a sweep lands inside it.
    ///
    /// The caller therefore passes a `retain_after_exp` of at least that
    /// leeway plus a skew budget between the database's clock and any API
    /// replica's. The worker passes
    /// `vpay_worker::CLIENT_ASSERTION_JTI_RETENTION` (5 minutes), which
    /// derives the number. This method does not default it, because the right
    /// value is a property of the validator and not of the table.
    /// [ADR-0028](../../../../docs/adr/0028-a-spent-jti-outlives-the-validators-leeway.md)
    /// records the decision.
    ///
    /// **What this still assumes:** that the database's clock is not more than
    /// roughly `retain_after_exp − 61 s` ahead of every API replica's (about
    /// four minutes at the production value), and that `authkestra-op`'s
    /// leeway does not grow. The second is pinned by a test that refuses an
    /// assertion older than the production horizon
    /// (`backends/tests/integration/tests/merchant_token_flow.rs`); the first
    /// is an operational property (NTP) nothing here checks.
    ///
    /// Zero is a valid `retain_after_exp` and restores the unsafe behaviour
    /// this parameter exists to remove; production code must not pass it.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the delete fails. The worker's housekeeping
    /// sweep treats that as a failed run and the job is retried; failing to
    /// prune is not a reason to refuse to serve traffic.
    async fn delete_expired_client_assertion_jtis(
        &self,
        retain_after_exp: Duration,
    ) -> Result<u64, DbError>;
}

#[async_trait::async_trait]
impl ClientAssertions for crate::repository::PgRepositories {
    async fn delete_expired_client_assertion_jtis(
        &self,
        retain_after_exp: Duration,
    ) -> Result<u64, DbError> {
        let result = sqlx::query(
            "DELETE FROM oauth_client_assertion_jtis \
             WHERE expires_at < now() - ($1::BIGINT * INTERVAL '1 microsecond')",
        )
        .bind(crate::jobs::as_micros(retain_after_exp))
        .execute(&self.pool)
        .await
        .map_err(DbError::Query)?;

        Ok(result.rows_affected())
    }
}
