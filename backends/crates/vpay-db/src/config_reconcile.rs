//! Boot-time reconciliation of the two reference tables — `currencies`
//! (migration 0001) and `providers` (migration 0002) — from the running
//! deployment's own configuration.
//!
//! This is step 4 of both binaries' boot sequence, between `run_migrations`
//! and `ensure_active_signing_key`. Until it existed, `providers` was
//! documented as "reconciliation from YAML is not implemented yet"
//! (migration 0002's own `COMMENT ON TABLE`) and every row in both tables
//! had to be inserted by hand.
//!
//! # Configuration is the authority; the database is the mirror
//!
//! [`ConfigReconcile::reconcile`] takes the deployment's view and makes the tables match it,
//! in one transaction: a provider present in the seed is inserted or
//! updated, and a provider **absent** from the seed is set
//! `enabled = false` — not deleted. Deleting would break every `charges`
//! and `provider_requests` row that references it (and the foreign keys
//! would refuse anyway); disabling keeps the history readable while making
//! the rail unusable for new charges. A rail that has ever taken money must
//! stay nameable forever.
//!
//! # No adapter dependency, deliberately
//!
//! The seeds are plain structs, and this module links no adapter crate. The
//! join between "what the YAML configures" and "what this binary actually
//! has an adapter for" happens in each binary's `main.rs`, where both are
//! in scope — a YAML provider code with no linked adapter is a
//! configuration error there (exit 78), not something this layer can see.
//! Depending on the adapters from here would also drag them into `vpay-db`,
//! which is exactly the coupling ADR-0002's port exists to prevent.
//!
//! # One transaction, and one writer at a time
//!
//! Every statement below runs in one transaction, so a failure part-way
//! through leaves the tables exactly as they were. A half-reconciled
//! `providers` table — new rails inserted, the disable pass never run — is
//! a deployment where a rail an operator just removed is still accepting
//! charges.
//!
//! That transaction opens by taking [`lock_keys::CONFIG_RECONCILE`], because
//! boot step 4 runs in *both* binaries and in every replica of each: a
//! rollout that restarts four processes runs four of these concurrently
//! against one database. See that constant for what interleaving them costs.
//! The seeds are also iterated in `code` order (sorted here, not left to the
//! YAML), which removes the ordering half of the same hazard independently
//! of the lock — two guards, because the sort only binds writers that share
//! this function and the lock binds any writer of these tables.

use crate::error::{DbError, classify_write};
use crate::lock_keys;

/// One rail as this deployment describes it: the identity and capabilities
/// from a linked adapter, plus whether the YAML enables it.
///
/// A plain data struct rather than `vpay_provider::Capabilities` so
/// `vpay-db` stays free of the provider port — see the module comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSeed {
    /// Stable rail code, e.g. `mtn_momo`. Primary key of `providers`, and
    /// what `payment_method_types` on an intent refers to.
    pub code: String,
    /// Human-readable name for dashboards and operator tooling.
    pub display_name: String,
    /// `push` or `redirect`, the wire form of `vpay_core::ProviderFlow`.
    /// Written into the `provider_flow` enum column, so any other string is
    /// a [`DbError::Query`] at boot rather than a silently stored typo.
    pub flow: String,
    /// Whether the rail can refund at all.
    pub supports_refunds: bool,
    /// Whether it can refund part of a charge. Implies `supports_refunds`,
    /// enforced by the `partial_refunds_imply_refunds` CHECK.
    pub supports_partial_refunds: bool,
    /// Whether the rail posts callbacks. Callbacks are hints either way
    /// (`AGENTS.md`) — this only says whether to expect them.
    pub delivers_callbacks: bool,
    /// Whether the rail requires vpay's egress IPs to be allow-listed.
    pub requires_ip_allowlist: bool,
    /// Whether new charges may use this rail right now.
    pub enabled: bool,
}

/// One currency this deployment accepts.
///
/// The exponent is *not* a per-deployment setting — see
/// [`DbError::CurrencyExponentConflict`], and migration 0001's comment: it
/// is a property of the currency itself, and every amount already stored is
/// a count of minor units at the recorded value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencySeed {
    /// ISO-4217 alphabetic code, uppercase (`code_is_iso4217_shape`).
    pub code: String,
    /// Minor units per major unit as a power of ten: 0 for XAF, 2 for EUR.
    pub exponent: i32,
}

#[async_trait::async_trait]
pub trait ConfigReconcile: Send + Sync {
    /// Makes `currencies` and `providers` match this deployment, in one
    /// transaction, with at most one such transaction in flight per database.
    ///
    /// Idempotent: running it twice with the same seeds changes nothing an
    /// observer can see (proven by
    /// `reconcile_is_idempotent_and_disables_a_dropped_provider_code`), which
    /// is what makes it safe on every replica's boot rather than a migration
    /// that must run once.
    ///
    /// **Serialised, not merely idempotent.** Idempotence is about *repeating*
    /// this function; it says nothing about two runs *overlapping*, which is the
    /// normal case (both binaries do this at boot, and a rollout restarts them
    /// together). The `pg_advisory_xact_lock` below is what makes the overlap
    /// safe, and `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released`
    /// in `tests/repositories.rs` is what proves the lock is actually taken.
    ///
    /// # Errors
    ///
    /// [`DbError::CurrencyExponentConflict`] if a currency is already recorded
    /// with a different exponent — refused rather than overwritten, because
    /// changing it would reinterpret every amount already stored in that
    /// currency. [`DbError::Query`] if a `flow` is not a member of the
    /// `provider_flow` enum, if `supports_partial_refunds` is set without
    /// `supports_refunds` (the `partial_refunds_imply_refunds` CHECK), or if
    /// any statement or the commit fails. Both roll the whole transaction back.
    async fn reconcile(
        &self,
        currencies: &[CurrencySeed],
        providers: &[ProviderSeed],
    ) -> Result<(), DbError>;
}

#[async_trait::async_trait]
impl ConfigReconcile for crate::repository::PgRepositories {
    async fn reconcile(
        &self,
        currencies: &[CurrencySeed],
        providers: &[ProviderSeed],
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;

        // The first statement of the transaction, before anything is read or
        // written: every other reconcile against this database now waits here
        // instead of interleaving its upserts with ours. Transaction-scoped, so
        // `COMMIT`/`ROLLBACK` releases it and there is no unlock path to leak —
        // including on the `CurrencyExponentConflict` early return below, which
        // drops `tx` and rolls back.
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_keys::CONFIG_RECONCILE)
            .execute(&mut *tx)
            .await
            .map_err(DbError::Query)?;

        // Sorted by code rather than iterated in YAML order. Two deployments
        // that list the same rails in different orders would otherwise take the
        // same row locks in opposite orders — a deadlock Postgres resolves by
        // aborting one of them (`40P01`), i.e. by failing a boot. The lock above
        // already prevents that between two `reconcile` calls; this makes the
        // statement order a property of the data rather than of a config file's
        // formatting, so it also holds against any future writer that takes the
        // rows in code order without knowing about the lock. Cloning the slices
        // is the cost of not mutating a caller's input: a handful of seeds at
        // boot.
        let mut currencies = currencies.to_vec();
        currencies.sort_by(|left, right| left.code.cmp(&right.code));
        let mut providers = providers.to_vec();
        providers.sort_by(|left, right| left.code.cmp(&right.code));

        for currency in &currencies {
            // `DO UPDATE SET exponent = currencies.exponent` is a deliberate
            // no-op write: it locks the existing row and makes `RETURNING`
            // yield the *stored* exponent (a bare `DO NOTHING` returns no row
            // at all on conflict, which would need a second query and a race
            // between the two). The comparison then happens in Rust, where the
            // refusal can name both values.
            let stored = sqlx::query_scalar::<_, i32>(
                "INSERT INTO currencies (code, exponent) VALUES ($1, $2) \
             ON CONFLICT (code) DO UPDATE SET exponent = currencies.exponent \
             RETURNING exponent",
            )
            .bind(&currency.code)
            .bind(currency.exponent)
            .fetch_one(&mut *tx)
            .await
            .map_err(classify_write)?;

            if stored != currency.exponent {
                return Err(DbError::CurrencyExponentConflict {
                    code: currency.code.clone(),
                    stored,
                    seeded: currency.exponent,
                });
            }
        }

        for provider in &providers {
            sqlx::query(
                "INSERT INTO providers \
                 (code, display_name, flow, supports_refunds, supports_partial_refunds, \
                  delivers_callbacks, requires_ip_allowlist, enabled) \
             VALUES ($1, $2, $3::provider_flow, $4, $5, $6, $7, $8) \
             ON CONFLICT (code) DO UPDATE SET \
                 display_name = EXCLUDED.display_name, \
                 flow = EXCLUDED.flow, \
                 supports_refunds = EXCLUDED.supports_refunds, \
                 supports_partial_refunds = EXCLUDED.supports_partial_refunds, \
                 delivers_callbacks = EXCLUDED.delivers_callbacks, \
                 requires_ip_allowlist = EXCLUDED.requires_ip_allowlist, \
                 enabled = EXCLUDED.enabled",
            )
            .bind(&provider.code)
            .bind(&provider.display_name)
            .bind(&provider.flow)
            .bind(provider.supports_refunds)
            .bind(provider.supports_partial_refunds)
            .bind(provider.delivers_callbacks)
            .bind(provider.requires_ip_allowlist)
            .bind(provider.enabled)
            .execute(&mut *tx)
            .await
            .map_err(classify_write)?;
        }

        // Anything this deployment no longer configures is disabled, never
        // deleted (module comment). With an empty seed this disables every
        // provider, which is the correct reading of "this deployment has no
        // rails" — and is why a binary must fail on an unknown YAML provider
        // code *before* calling here rather than passing a shortened list.
        let configured: Vec<String> = providers.iter().map(|p| p.code.clone()).collect();
        sqlx::query("UPDATE providers SET enabled = false WHERE code <> ALL($1) AND enabled")
            .bind(&configured)
            .execute(&mut *tx)
            .await
            .map_err(classify_write)?;

        tx.commit().await.map_err(DbError::Query)?;

        Ok(())
    }
}
