//! The `disabled_clients` kill-switch query layer (`backends/migrations/
//! 0012_create-disabled-clients.sql`).
//!
//! YAML stays authoritative for client *identity* (does this client exist,
//! what is its key — ADR-0003, ADR-0010); this table only ever *subtracts*
//! access, letting an operator revoke a compromised client instantly,
//! without a deploy. A correct "may this client authenticate right now"
//! answer needs both sources, per ADR-0010's Consequences section — this
//! module answers only the database half.
//!
//! # Caching — deliberately none, argued
//!
//! [`is_client_disabled`] is a plain `SELECT` on every call, with no cache in
//! front of it. That is a considered choice, not an oversight:
//!
//! - **A per-request `SELECT` is simplest and always correct.** It reads the
//!   one row that matters, by primary key, every time — there is no window
//!   in which the answer can be stale.
//! - **A cache directly undoes the one thing this table exists to provide.**
//!   The entire point of `disabled_clients` (this module's own header, and
//!   ADR-0010) is that revocation is *instant* — "an operator flips a client
//!   to disabled and it takes effect immediately, no deploy required." Any
//!   cache with a nonzero TTL reintroduces exactly the deploy-speed problem
//!   this table was built to avoid: a compromised merchant key gets disabled
//!   at 2am, and every replica that already cached "not disabled" keeps
//!   accepting it for the rest of the TTL. A revocation mechanism that is
//!   merely "faster than a deploy" is a much weaker claim than "instant,"
//!   and this repository does not get to silently narrow that claim.
//! - **The query this replaces is genuinely cheap.** `client_id` is the
//!   table's primary key, so this is an index-only point lookup against a
//!   table that — per this table's own migration header — holds one row per
//!   *disabled* client only, not one per registered client; in the common
//!   case (an operator has disabled nothing) the table is empty and Postgres
//!   answers from a couple of cached pages. It is called on the same request
//!   path as `private_key_jwt` signature verification (RSA/EC), which costs
//!   far more CPU per call than an indexed point lookup costs in network
//!   round-trip time — the kill-switch check is not the bottleneck a cache
//!   would be solving for.
//!
//! If a future load test shows this lookup actually dominates token-issuance
//! latency, the answer is a *short*, explicitly-bounded cache (seconds, not
//! minutes) built and justified at that point against a measured cost — not
//! a change made speculatively here.

use sqlx::PgPool;

use crate::error::DbError;

/// Reports whether `client_id` currently has a `disabled_clients` row.
///
/// See the module doc comment for why this is a plain, uncached `SELECT` on
/// every call rather than a cached lookup.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the lookup itself fails.
pub async fn is_client_disabled(pool: &PgPool, client_id: &str) -> Result<bool, DbError> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM disabled_clients WHERE client_id = $1)",
    )
    .bind(client_id)
    .fetch_one(pool)
    .await
    .map_err(DbError::Query)
}

/// Disables `client_id`, with an optional free-text operator note.
///
/// Idempotent at this layer via `INSERT ... ON CONFLICT (client_id) DO
/// UPDATE`: a raw duplicate `INSERT` is rejected by the database's own
/// primary key (proven by
/// `backends/tests/integration/tests/postgres_smoke.rs`'s
/// `a_duplicate_disabled_client_id_is_rejected_by_the_database`), but an
/// operator re-running "disable this client" — the actual caller of this
/// function — should not have to first check whether it is already
/// disabled. A second call updates the recorded `reason` (e.g. a fuller
/// explanation added after the fact) and leaves the original `disabled_at`
/// untouched, so "when was this client first disabled" stays accurate across
/// repeated calls.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails.
pub async fn disable_client(
    pool: &PgPool,
    client_id: &str,
    reason: Option<&str>,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO disabled_clients (client_id, reason) VALUES ($1, $2) \
         ON CONFLICT (client_id) DO UPDATE SET reason = EXCLUDED.reason",
    )
    .bind(client_id)
    .bind(reason)
    .execute(pool)
    .await
    .map_err(DbError::Query)?;

    Ok(())
}

/// Re-enables `client_id` by removing its `disabled_clients` row.
///
/// A no-op, not an error, if `client_id` was not disabled to begin with —
/// "make sure this client is enabled" is naturally idempotent as a `DELETE`.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the write fails.
pub async fn enable_client(pool: &PgPool, client_id: &str) -> Result<(), DbError> {
    sqlx::query("DELETE FROM disabled_clients WHERE client_id = $1")
        .bind(client_id)
        .execute(pool)
        .await
        .map_err(DbError::Query)?;

    Ok(())
}
