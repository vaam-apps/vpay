//! The `oauth_signing_keys` repository (`backends/migrations/
//! 0007_create-oauth-signing-keys.sql`, reshaped by `backends/migrations/
//! 0010_reshape-oauth-signing-keys.sql`) — read access for JWKS assembly,
//! and the one-transaction rotate operation the partial unique index
//! `one_active_signing_key` requires.
//!
//! This module is deliberately narrow: no key generation, no rotation
//! *scheduling*, and no PEM handling live here. The private key comes from a
//! Kubernetes Secret at process boot and is parsed once by
//! `authkestra_engine::TokenManager::new_asymmetric` — it is never
//! persisted, and this table never holds it (`0010`'s own migration header).
//! What this module owns is strictly "what does the database currently say
//! about public keys" and "atomically swap the active one."

use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;

use crate::error::DbError;

/// One row of `oauth_signing_keys`, as needed to publish `/jwks.json` or to
/// answer "which key is active."
///
/// Deliberately omits `created_at`/`updated_at`: nothing that reads this
/// repository (JWKS assembly, the active-key lookup) needs them — they exist
/// in the table purely as operational bookkeeping.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct SigningKey {
    /// The JWT header `kid` this key signs/verifies under.
    pub kid: String,
    /// The public half of the key, as a JWK (`kty`/`alg`/`n`/`e`/`kid`). No
    /// private key material is ever stored in this column or this table.
    pub public_jwk: Value,
    /// Whether this is the single currently-active signing key. At most one
    /// row may have `active = true` (`one_active_signing_key`, a partial
    /// unique index).
    pub active: bool,
    /// `NULL` while active; set to the instant this key stops being
    /// publishable once it is retired.
    pub expires_at: Option<OffsetDateTime>,
}

/// Reads every currently-publishable key — the active one, plus any retired
/// key still inside its overlap window — for `/jwks.json` assembly.
///
/// `WHERE active OR expires_at > now()`, per the task brief: a retired key
/// must keep publishing until its own `expires_at` so tokens it already
/// signed keep verifying for their remaining lifetime; a key retired long
/// enough ago that `expires_at` has passed must not appear.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the read fails.
pub async fn publishable_signing_keys(pool: &PgPool) -> Result<Vec<SigningKey>, DbError> {
    sqlx::query_as::<_, SigningKey>(
        "SELECT kid, public_jwk, active, expires_at FROM oauth_signing_keys \
         WHERE active OR expires_at > now() \
         ORDER BY created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::Query)
}

/// Reads the `kid` of the single active signing key, if one exists.
///
/// `one_active_signing_key` guarantees at most one row can ever have
/// `active = true`, so `LIMIT 1` is a belt-and-braces cap on the query
/// itself, not something the result needs to be validated against.
///
/// # Errors
///
/// Returns [`DbError::Query`] if the read fails.
pub async fn active_signing_key_kid(pool: &PgPool) -> Result<Option<String>, DbError> {
    sqlx::query_scalar::<_, String>("SELECT kid FROM oauth_signing_keys WHERE active LIMIT 1")
        .fetch_optional(pool)
        .await
        .map_err(DbError::Query)
}

/// Inserts a new active signing key and retires whatever was active before
/// it, in one transaction.
///
/// Order inside the transaction matters and is not arbitrary: the partial
/// unique index `one_active_signing_key` (`WHERE active`) is checked
/// immediately on each statement, not deferred to `COMMIT`, so inserting the
/// new active row *before* retiring the old one would be rejected outright
/// by the very index this function exists to respect — vpay would observe a
/// unique-violation on what should be a routine rotation. Retiring first
/// (`UPDATE ... WHERE active`) leaves zero active rows for the moment
/// in between, so the subsequent `INSERT ... active = true` never conflicts.
/// If no key is currently active — the very first key this deployment ever
/// writes — the `UPDATE` simply affects zero rows and the `INSERT` becomes
/// an ordinary bootstrap insert; this function needs no separate "is this
/// the first key" branch.
///
/// Wrapping both statements in one transaction is what makes a failure safe:
/// if the `INSERT` fails (e.g. a duplicate `kid`), the `UPDATE` that retired
/// the previous key rolls back with it, and the previous key is still the
/// active one — never a database left with zero active signing keys.
///
/// `retire_previous_at` is supplied by the caller (rotation-scheduling
/// logic, out of scope for this repository layer) as the instant the
/// previous key should stop being published — typically "now plus an
/// overlap window," never computed in here.
///
/// # Errors
///
/// Returns [`DbError::Query`] if either statement, or the commit, fails.
pub async fn rotate_signing_key(
    pool: &PgPool,
    new_kid: &str,
    new_public_jwk: &Value,
    retire_previous_at: OffsetDateTime,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await.map_err(DbError::Query)?;

    sqlx::query(
        "UPDATE oauth_signing_keys \
         SET active = false, expires_at = $1, updated_at = now() \
         WHERE active",
    )
    .bind(retire_previous_at)
    .execute(&mut *tx)
    .await
    .map_err(DbError::Query)?;

    sqlx::query("INSERT INTO oauth_signing_keys (kid, public_jwk, active) VALUES ($1, $2, true)")
        .bind(new_kid)
        .bind(new_public_jwk)
        .execute(&mut *tx)
        .await
        .map_err(DbError::Query)?;

    tx.commit().await.map_err(DbError::Query)?;

    Ok(())
}
