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
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;

use crate::error::DbError;

/// The transaction-scoped advisory-lock key every write in this module takes
/// before it reads or changes which key is active.
///
/// Aliased from [`crate::lock_keys`] rather than spelled here: advisory locks
/// share one namespace per database, so the values have to be readable side
/// by side to be checkably distinct — that module owns the value, the reason
/// it exists and the proof that no two subjects share one. Serialising the
/// whole read-decide-write under `pg_advisory_xact_lock` makes the second
/// replica observe the first one's committed row and answer
/// [`ActivationOutcome::AlreadyActive`], which is the honest result. The lock
/// is released by `COMMIT`/`ROLLBACK` — there is no unlock path to leak.
const ROTATION_LOCK_KEY: i64 = crate::lock_keys::SIGNING_KEY_ROTATION;

/// What [`ensure_active_signing_key`] found, and therefore whether it wrote.
///
/// Returned rather than logged-and-discarded so a caller (a binary's
/// `main()`, at boot) can decide what a rotation means for it — page, emit a
/// metric, or simply carry on — without re-querying the table to find out
/// what just happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationOutcome {
    /// The `kid` handed in was already the active one. **No row was written**
    /// — not an idempotent re-write, no write at all — which is what makes
    /// every replica after the first one free of side effects at boot.
    AlreadyActive,
    /// A rotation happened: the given `kid` is now the sole active key.
    Rotated {
        /// The `kid` that was active immediately before, if any. `None` is
        /// the first key this database has ever held (a bootstrap), not an
        /// error.
        previous: Option<String>,
    },
}

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
/// The transaction also takes `ROTATION_LOCK_KEY` before its first write,
/// so it queues behind any concurrent [`ensure_active_signing_key`] rather
/// than interleaving with that function's read-then-write. This changes
/// nothing about the statements below or their order; it only means two
/// rotations in flight at once resolve one after the other.
///
/// # Errors
///
/// Returns [`DbError::SigningKeyRetired`] if `new_kid` names a row that is
/// present but retired (see `refuse_retired_kid`, which both writers share
/// so they cannot drift on it), and [`DbError::Query`] if either statement,
/// or the commit, fails.
pub async fn rotate_signing_key(
    pool: &PgPool,
    new_kid: &str,
    new_public_jwk: &Value,
    retire_previous_at: OffsetDateTime,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await.map_err(DbError::Query)?;

    lock_rotation(&mut tx).await?;
    retire_then_insert(&mut tx, new_kid, new_public_jwk, retire_previous_at).await?;

    tx.commit().await.map_err(DbError::Query)?;

    Ok(())
}

/// Makes `new_kid` the active signing key **only if it is not already**, and
/// reports which of the two happened.
///
/// This is what every replica calls at boot with the key it just loaded from
/// its Secret mount, so it has to be safe to call from N processes at once
/// with the *same* `kid`: exactly one of them may write, and the rest must
/// come back [`ActivationOutcome::AlreadyActive`] without touching the table.
/// The read and the write therefore happen inside one transaction that first
/// takes `ROTATION_LOCK_KEY` — see that constant's own comment for why the
/// obvious single-statement compare-and-swap does not cover this case.
///
/// `retire_previous_at` is the caller's rotation-overlap policy, exactly as
/// in [`rotate_signing_key`]: this layer never invents a window.
///
/// # Rolling *back* to a previously retired key is refused by name
///
/// If `new_kid` exists in the table but is retired — the shape a deploy
/// rollback to an older Secret takes — this returns
/// [`DbError::SigningKeyRetired`], naming the `kid` and when it was retired,
/// and writes nothing. It does **not** re-activate the old row: that needs a
/// policy decision about `expires_at` and about whether publishing a key
/// that was deliberately retired is ever right, and the rotation policy as a
/// whole is still an open maintainer question (`docs/roadmap.md`, "Open —
/// signing-key rotation overlap window"). Failing loudly at boot is the
/// honest behaviour until that is decided; an operator's way out is to roll
/// forward to a new key, or to restore the Secret holding the current one.
///
/// It used to fail as [`DbError::Query`] wrapping a duplicate-key violation
/// on `oauth_signing_keys.kid`, which is the same refusal with two costs:
/// `Category::Storage` told a supervisor to exit `69` and keep restarting
/// (waiting for a database that was never unwell), and the only text an
/// operator got was Postgres's constraint name. See `refuse_retired_kid`,
/// which is where the check now lives, and `DbError::SigningKeyRetired`.
///
/// # Errors
///
/// Returns [`DbError::SigningKeyRetired`] for the rollback case above, and
/// [`DbError::Query`] if the lock, the read, either write, or the commit
/// fails.
pub async fn ensure_active_signing_key(
    pool: &PgPool,
    new_kid: &str,
    new_public_jwk: &Value,
    retire_previous_at: OffsetDateTime,
) -> Result<ActivationOutcome, DbError> {
    let mut tx = pool.begin().await.map_err(DbError::Query)?;

    lock_rotation(&mut tx).await?;

    // Deliberately not a call to `active_signing_key_kid`: that one takes a
    // `&PgPool` and would run on a *different* connection, outside this
    // transaction and outside the lock — which is precisely the race this
    // function exists to close.
    let previous =
        sqlx::query_scalar::<_, String>("SELECT kid FROM oauth_signing_keys WHERE active LIMIT 1")
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::Query)?;

    if previous.as_deref() == Some(new_kid) {
        // Commit rather than roll back: the transaction wrote nothing, and
        // committing releases the advisory lock the same way, without
        // making a routine no-op look like a failed transaction in
        // `pg_stat_database`'s rollback counter.
        tx.commit().await.map_err(DbError::Query)?;
        tracing::info!(
            kid = new_kid,
            "signing key is already the active one; no rotation"
        );
        return Ok(ActivationOutcome::AlreadyActive);
    }

    retire_then_insert(&mut tx, new_kid, new_public_jwk, retire_previous_at).await?;
    tx.commit().await.map_err(DbError::Query)?;

    tracing::info!(
        kid = new_kid,
        previous_kid = previous.as_deref().unwrap_or("<none>"),
        retire_previous_at = %retire_previous_at,
        "rotated the active signing key"
    );

    Ok(ActivationOutcome::Rotated { previous })
}

/// Takes the transaction-scoped rotation lock. Every writer in this module
/// calls this first, so "who may change the active key" is one queue rather
/// than a race — see `ROTATION_LOCK_KEY`.
async fn lock_rotation(tx: &mut PgConnection) -> Result<(), DbError> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ROTATION_LOCK_KEY)
        .execute(tx)
        .await
        .map_err(DbError::Query)?;
    Ok(())
}

/// The ordering-sensitive half of a rotation, factored out so
/// [`rotate_signing_key`] and [`ensure_active_signing_key`] cannot drift on
/// it. Retire first, then insert — [`rotate_signing_key`]'s doc comment
/// explains why that order is the only one `one_active_signing_key` permits.
///
/// Takes a connection rather than a pool precisely so both callers run it
/// inside *their* transaction, holding *their* lock.
async fn retire_then_insert(
    tx: &mut PgConnection,
    new_kid: &str,
    new_public_jwk: &Value,
    retire_previous_at: OffsetDateTime,
) -> Result<(), DbError> {
    refuse_retired_kid(&mut *tx, new_kid).await?;

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

    Ok(())
}

/// Refuses, by name, the one duplicate-`kid` case that is an *operator*
/// mistake rather than a database failure: `new_kid` is already in the table
/// and has been retired.
///
/// Runs inside the caller's transaction, so it reads under the same
/// `ROTATION_LOCK_KEY` the caller took and cannot race a concurrent
/// rotation committing the row between this `SELECT` and the `INSERT` below
/// it.
///
/// # Why a read, not the duplicate-key error
///
/// The `INSERT` that follows would fail anyway — `kid` is the primary key —
/// with SQLSTATE `23505`. Catching *that* would mean matching on a
/// constraint name (`oauth_signing_keys_pkey`) carried inside a
/// `sqlx::Error`, which is a string this code does not own and which a
/// future migration renaming the constraint would silently change; the
/// symptom would be the crash loop coming back, in the one place nobody is
/// watching for it. Reading the row first is one extra `SELECT` on a boot
/// path that already takes an advisory lock and writes twice, and it also
/// yields the `retired_at` the message needs — which the duplicate-key
/// error does not carry at all.
///
/// `AND NOT active` scopes this deliberately narrowly. A `kid` that is
/// *currently active* is not this error: [`ensure_active_signing_key`]
/// answers [`ActivationOutcome::AlreadyActive`] before it ever gets here,
/// and [`rotate_signing_key`] asked to re-insert the live key is a caller
/// bug that keeps its existing duplicate-key [`DbError::Query`] rather than
/// borrowing a message about retirement that would not be true.
async fn refuse_retired_kid(tx: &mut PgConnection, new_kid: &str) -> Result<(), DbError> {
    // `updated_at`, not `expires_at`: see `DbError::SigningKeyRetired`'s own
    // field comment for why the retirement instant is the useful one.
    let retired_at = sqlx::query_scalar::<_, OffsetDateTime>(
        "SELECT updated_at FROM oauth_signing_keys WHERE kid = $1 AND NOT active",
    )
    .bind(new_kid)
    .fetch_optional(&mut *tx)
    .await
    .map_err(DbError::Query)?;

    match retired_at {
        Some(retired_at) => Err(DbError::SigningKeyRetired {
            kid: new_kid.to_owned(),
            retired_at,
        }),
        None => Ok(()),
    }
}
