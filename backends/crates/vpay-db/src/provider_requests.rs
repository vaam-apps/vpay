//! The `provider_requests` repository (`backends/migrations/
//! 0016_create-provider-requests.sql`) — one row per attempt to call a rail,
//! written before the call and completed after it.
//!
//! # The ordering is the point
//!
//! [`insert_pending`] runs *before* the network call and
//! [`record_response`] after it, so a process that dies mid-call leaves a
//! row with `status_code IS NULL` behind. That row is not litter: it is the
//! only durable evidence that a rail may have been asked to move money and
//! never answered, which is the exact case `docs/flows/crash-safety.md`
//! says must never be silently dropped. A recovery sweep looks for those
//! rows; there is nothing to look for if the write happens only on success.
//!
//! The same reasoning explains why "failed with no HTTP status" leaves
//! `responded_at` NULL rather than stamping it with the time the failure
//! was noticed — see [`record_response`].
//!
//! # Not yet swept
//!
//! **Nothing reads this table.** The recovery sweep described above does
//! not exist (the worker job loop is not started — `docs/status.md`), so
//! today these rows are written and never looked at. That is a recorded
//! gap, not a claim of crash recovery.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{DbError, classify_write};

/// Records that a call to a rail is about to be made, and returns the row
/// id the answer will be recorded against.
///
/// `attempt` is supplied by the caller rather than derived with a
/// `SELECT max(attempt) + 1` here: that read-then-write would race two
/// concurrent retries into the same number, and the poll ladder — which
/// knows how many times *it* has tried — has the number already.
///
/// # Errors
///
/// [`DbError::ForeignKeyViolation`] if `charge_id` or `provider_code` names
/// something that does not exist, [`DbError::Query`] if `operation` is not
/// one of `submit`/`query_status`/`refund` (a CHECK violation, i.e. a vpay
/// bug) or the write otherwise fails.
pub async fn insert_pending(
    pool: &PgPool,
    charge_id: &str,
    provider_code: &str,
    operation: &str,
    reference: Uuid,
    attempt: i32,
) -> Result<i64, DbError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO provider_requests \
             (charge_id, provider_code, operation, provider_reference_id, attempt) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING id",
    )
    .bind(charge_id)
    .bind(provider_code)
    .bind(operation)
    .bind(reference)
    .bind(attempt)
    .fetch_one(pool)
    .await
    .map_err(classify_write)
}

/// Completes the attempt `id` with whatever came back.
///
/// `responded_at` is set **only** when there is a `status_code`. An attempt
/// that failed without ever getting an HTTP status — a timeout, a TLS
/// failure, or an adapter returning `ProviderError::NotImplemented` before
/// any socket was opened — did not receive a response, and stamping one
/// would make it indistinguishable from a rail that actually answered. The
/// `response_is_paired` CHECK enforces this at the database, so the two
/// facts cannot drift apart even if a future writer forgets: such a row
/// keeps `status_code IS NULL`, `responded_at IS NULL` and carries its
/// `error_kind` as the record of what went wrong.
///
/// This is exactly the row the `501` from a not-implemented `submit`
/// leaves behind, deliberately (this step's design, §4: "the submitting
/// charge row + NULL-status provider_request row are left on purpose").
///
/// # Errors
///
/// [`DbError::WriteMatchedNoRow`] if `id` names no row — the caller just
/// created it, so that is an invariant violation rather than a merchant's
/// mistake — and [`DbError::Query`] if the write fails.
pub async fn record_response(
    pool: &PgPool,
    id: i64,
    status_code: Option<i32>,
    error_kind: Option<&str>,
) -> Result<(), DbError> {
    let affected = sqlx::query(
        "UPDATE provider_requests \
         SET status_code = $2, \
             error_kind = $3, \
             responded_at = CASE WHEN $2::INT IS NULL THEN NULL ELSE now() END \
         WHERE id = $1",
    )
    .bind(id)
    .bind(status_code)
    .bind(error_kind)
    .execute(pool)
    .await
    .map_err(classify_write)?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::WriteMatchedNoRow {
            table: "provider_requests",
            key: id.to_string(),
        });
    }

    Ok(())
}
