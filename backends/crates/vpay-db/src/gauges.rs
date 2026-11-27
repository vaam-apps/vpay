//! The samplers behind the database-shaped gauges: this process's pool and the
//! webhook delivery backlog.
//!
//! They are *reads written into the `metrics` facade*, kept here beside the
//! queries so a binary calls one function and the metric names stay in
//! `vpay_core::metrics`. Neither installs a recorder (`vpay_core::metrics`'s
//! header), and with none installed both are no-ops.
//!
//! **A failed read writes nothing.** The gauge keeps its last value (a stale
//! series, which a dashboard shows as a flat line and an alert on staleness
//! catches), and the caller logs the error; a zero written for "could not
//! read" would read as a healthy, empty backlog.

use vpay_core::metrics::{
    DB_POOL_CONNECTIONS, DB_POOL_MAX_CONNECTIONS, WEBHOOK_DELIVERIES_PENDING,
    WEBHOOK_OLDEST_PENDING_OVERDUE_SECONDS, db_pool_state,
};

use crate::error::DbError;
use crate::health::{Health, PoolStats};
use crate::webhook_deliveries::{WebhookBacklog, WebhookDeliveries};

/// Writes one reading of the pool: connections idle and in use, and the
/// configured maximum.
pub fn record_pool(stats: &PoolStats) {
    metrics::gauge!(DB_POOL_CONNECTIONS, "state" => db_pool_state::IDLE).set(f64::from(stats.idle));
    metrics::gauge!(DB_POOL_CONNECTIONS, "state" => db_pool_state::IN_USE)
        .set(f64::from(stats.in_use()));
    metrics::gauge!(DB_POOL_MAX_CONNECTIONS).set(f64::from(stats.max));
}

/// Writes one reading of the webhook delivery backlog.
pub fn record_webhook_backlog(backlog: &WebhookBacklog) {
    // A count as an `f64` gauge without a lossy cast: clamped at `i32::MAX`, a
    // backlog that no healthy deployment is anywhere near, and which a
    // clamp reports as "enormous" rather than as something smaller.
    let pending = i32::try_from(backlog.pending).map_or(f64::from(i32::MAX), f64::from);
    metrics::gauge!(WEBHOOK_DELIVERIES_PENDING).set(pending);
    metrics::gauge!(WEBHOOK_OLDEST_PENDING_OVERDUE_SECONDS).set(backlog.oldest_overdue_seconds);
}

/// Samples the pool (no I/O) and, when the read succeeds, the webhook backlog.
///
/// # Errors
///
/// The backlog read's [`DbError`]; the pool gauges are written first and
/// regardless, because they need no database.
pub async fn sample<R>(repositories: &R) -> Result<(), DbError>
where
    R: Health + WebhookDeliveries + ?Sized,
{
    record_pool(&repositories.pool_stats());
    let backlog = repositories.backlog().await?;
    record_webhook_backlog(&backlog);
    Ok(())
}
