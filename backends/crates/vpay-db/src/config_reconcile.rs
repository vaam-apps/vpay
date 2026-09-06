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
//! # Both upserts run through CrateStack; the one statement that does not is
//! the disable pass
//!
//! Since 2026-09-06 the currency pass is `find_unique(code).for_update()`
//! followed by `upsert(...)`, and since later the same day the provider pass
//! is `upsert(...)` — all of them `run_in_tx` on the transaction this
//! function opens and all of them after the advisory lock below. CrateStack
//! joins vpay's transaction; it never opens one of its own.
//!
//! The provider pass was a hand-written `INSERT ... ON CONFLICT` until then,
//! and for a measured reason rather than a pending one: CrateStack's
//! generated upsert input cannot carry a `@default(...)` column, and five of
//! `providers`' eight columns had one. Migration 0033 dropped those five
//! column defaults and `schemas/vpay.cstack` dropped the five `@default(...)`
//! in the same commit — the two halves are one change, because either alone
//! puts five `default value differs` lines in the drift report. Why a code
//! generator's input-shaping rule got to decide vpay's DDL is argued in
//! migration 0033's header and in
//! [docs/reference/vpay-db.md § CrateStack](../../../../docs/reference/vpay-db.md#cratestack).
//!
//! The disable pass — `UPDATE providers SET enabled = false WHERE code <>
//! ALL($1)` — is still hand-written, because it addresses rows by their
//! *absence* from a list and no generated builder expresses that.
//!
//! # `# Errors` moved, and callers should notice
//!
//! The currency and provider statements now fail as [`DbError::Persistence`]
//! rather than [`DbError::Query`], so a caller matching the *variant* would
//! have silently stopped matching. Boot only ever reads the category — and
//! **two categories moved with the variant**, which this paragraph said they
//! did not until the review pass of 2026-09-06 checked it.
//!
//! An integrity violation `vpay-db` had already given its own variant is
//! genuinely unchanged: `23505` and `23503` classify identically on both
//! paths, asserted against each other by
//! `persistence::tests::a_duplicate_key_classifies_the_same_through_cratestack_as_through_sqlx`,
//! and a pool timeout is `Category::Storage` either way. A `23514` is **not**
//! in that set and never was. `error::classify_write` deliberately leaves it
//! in the unclassified [`DbError::Query`] bucket → `Category::Storage` → exit
//! `69`; `persistence::classify_cratestack` gives it
//! `PersistenceError::Check` → `Category::Internal` → exit `1`. Nothing
//! asserts those two against each other, because they disagree.
//!
//! What that means here, concretely: the `partial_refunds_imply_refunds`
//! CHECK is the only `23514` boot step 4 can raise, and until the provider
//! pass moved it reached a supervisor as `69` ("wait for Postgres") for an
//! adapter whose declared `Capabilities` are incoherent. It is `1`
//! ("page someone") now. That is arguably the better answer — nothing about
//! the database is wrong and `Capabilities::is_coherent` is not checked at
//! boot, so it is vpay's own bug — but it is a change, it was not asked for,
//! and whether boot should instead refuse an incoherent adapter as
//! `Category::Configuration` (exit `78`, like the flow label below) is a
//! maintainer's call, recorded in docs/status.md rather than taken here.
//! `a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
//! pins the category so the next move is deliberate.
//!
//! One classification changed on purpose rather than as a consequence, and it
//! is the flow label: see [`DbError::ProviderFlowUnknown`].
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
use crate::persistence::{classify_cratestack, system_context};

/// The `.cstack` model the currency half of [`ConfigReconcile::reconcile`]
/// reads and writes through, named once so the error a denial produces and
/// the query that produced it cannot drift apart.
const CURRENCY_MODEL: &str = "Currency";

/// The `.cstack` model the provider half of [`ConfigReconcile::reconcile`]
/// writes through, named here for [`CURRENCY_MODEL`]'s reason: the string in
/// a denial's message and the query that produced it must not drift apart.
const PROVIDER_MODEL: &str = "Provider";

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
    ///
    /// A `String` rather than the enum, which is Step 2's D4 (`vpay-db`
    /// binds strings; `vpay-core` parses) and is left standing here: the
    /// producer, `vpay_api::v1::boot::boot_seeds`, already holds a
    /// `ProviderFlow` and renders it with a `match`, so nothing in vpay can
    /// put a third word in this field.
    ///
    /// Any other string is refused rather than stored, and where it is
    /// refused has moved twice. The native `provider_flow` enum type refused
    /// the cast until migration 0032; `providers_flow_enum_check` refused the
    /// row until 2026-09-06; and since the provider pass became a CrateStack
    /// upsert — whose input takes the schema's `ProviderFlow` enum, not a
    /// string — [`ConfigReconcile::reconcile`] refuses the *seed*, as
    /// [`DbError::ProviderFlowUnknown`]. The CHECK is still there and still
    /// fires for a writer that is not `reconcile`; what changed is which
    /// layer answers first, and the advice it gives (exit `78`, not `69`).
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
    /// ISO-4217 alphabetic code, uppercase
    /// (`currencies_code_iso4217_check`, renamed from
    /// `code_is_iso4217_shape` by migration 0032).
    pub code: String,
    /// Minor units per major unit as a power of ten: 0 for XAF, 2 for EUR.
    ///
    /// `i64` since migration 0032 widened the column to `BIGINT`; see
    /// [`DbError::CurrencyExponentConflict`]'s `stored` field for why the
    /// width moved and why the accepted range did not.
    pub exponent: i64,
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
    /// currency. [`DbError::ProviderFlowUnknown`] if a seed's `flow` is
    /// neither `push` nor `redirect`; that one is refused before any
    /// statement runs, and it is the one error here whose category changed
    /// when the provider pass moved (see the variant).
    /// [`DbError::Persistence`] if any currency or provider statement fails
    /// — including `supports_partial_refunds` without `supports_refunds`,
    /// which the `partial_refunds_imply_refunds` CHECK refuses as a `23514`,
    /// and including when a model policy refuses one: every CrateStack call
    /// here runs as the system principal, so a refusal means
    /// `schemas/vpay.cstack` and this module disagree, and it classifies
    /// `Category::Internal` rather than `Forbidden`. [`DbError::Query`] if
    /// the advisory lock, the disable pass or the commit fails. All of them
    /// roll the whole transaction back.
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

        // One context for the whole transaction rather than one per
        // statement: `system_context()` is two `BTreeMap` inserts against a
        // `&'static str`, and building it once here also makes it obvious
        // that every CrateStack call below runs as the *same* principal.
        let ctx = system_context();

        for currency in &currencies {
            // Read first, then write — and the order is the guard, not an
            // optimisation.
            //
            // The statement this replaced was a single `INSERT ... ON
            // CONFLICT (code) DO UPDATE SET exponent = currencies.exponent
            // RETURNING exponent`: a deliberate *no-op* write whose only job
            // was to lock the row and hand back the **stored** exponent, so
            // the comparison could happen in Rust. CrateStack's `upsert`
            // cannot render that statement. It renders `DO UPDATE SET
            // exponent = EXCLUDED.exponent` — measured, not assumed, by
            // `the_currency_upsert_would_overwrite_a_stored_exponent_on_its_own`
            // below — which is the *overwrite* this whole error variant
            // exists to refuse. So the read cannot be folded into the write
            // here; it has to precede it and it has to hold a lock.
            //
            // `.for_update()` is what makes "read, decide, then write" safe
            // against a second writer: `SELECT ... WHERE code = $1 LIMIT 1
            // FOR UPDATE` inside this transaction locks the row for the rest
            // of it, so nothing can change the exponent between the
            // comparison and the upsert. It is deliberately the *second*
            // guard: `pg_advisory_xact_lock` above already serialises every
            // `reconcile` against every other, and
            // `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released`
            // still passes with this `.for_update()` deleted — measured, and
            // recorded in docs/plans/exp17-notes/opus.md, so a reader knows
            // which guard is which. The row lock binds a writer that does
            // *not* go through this function; the advisory lock binds the
            // ones that do.
            //
            // An absent row reads `Ok(None)` rather than an error
            // (`FindUnique::run_in_tx` maps `RowNotFound` to `None`), which
            // is the insert case and needs no comparison at all.
            let stored = self
                .cs
                .currency()
                .find_unique(currency.code.clone())
                .for_update()
                .run_in_tx(&mut tx, &ctx)
                .await
                .map_err(|error| {
                    DbError::from(classify_cratestack(CURRENCY_MODEL, "read", error))
                })?;

            if let Some(row) = &stored
                && row.exponent != currency.exponent
            {
                return Err(DbError::CurrencyExponentConflict {
                    code: currency.code.clone(),
                    stored: row.exponent,
                    seeded: currency.exponent,
                });
            }

            // Reached only when the stored exponent agrees with the seed, or
            // when there is no stored row — so `SET exponent =
            // EXCLUDED.exponent` writes the value that is already there, or
            // inserts. Either way it is the no-op the hand-written statement
            // was, and `CurrencyExponentConflict` above is the only path that
            // can see a disagreement.
            //
            // `run_in_tx`, and NOT `run`. The difference is not only
            // atomicity — it is the second half of the row lock above.
            // `upsert`'s own conflict probe is `SELECT ... FOR UPDATE`
            // (`upsert_sql.rs::select_for_update_by_conflict_target`), so on
            // this transaction it re-takes a lock this transaction already
            // holds (free), and on any other connection it would wait for a
            // transaction that is itself waiting for it. Measured on
            // 2026-09-06 by swapping this one call to `.run(&ctx)`:
            // `reconcile_is_idempotent_and_disables_a_dropped_provider_code`
            // did not fail, it **hung** — `SLOW [>480.000s]` and still going
            // when the run was killed, which in a deployment is a boot that
            // never returns.
            // `a_currency_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
            // in `tests/repositories.rs` is what turns that into a 1.2-second
            // red with a message; see docs/plans/exp17-notes/opus-review.md.
            //
            // Two pooled connections on the conflict branch, not one:
            // `upsert_resolve.rs::gate_update_policy` runs its update-policy
            // probe on `runtime.pool()` while this transaction still holds a
            // connection of its own. *That* probe is a plain `SELECT 1 ...
            // AND (<policy>)` with no `FOR UPDATE` — a different query from
            // the conflict probe above — so it does **not** block on the row
            // this transaction just locked, Postgres's MVCC reads not being
            // blocked by writers. It does mean two of `pool.rs`'s
            // `MAX_CONNECTIONS = 10` per in-flight reconcile, which is
            // comfortable for a boot step the advisory lock already admits
            // one at a time.
            let input = crate::schema::cratestack_schema::CreateCurrencyInput {
                code: currency.code.clone(),
                exponent: currency.exponent,
            };
            self.cs
                .currency()
                .upsert(input)
                .run_in_tx(&mut tx, &ctx)
                .await
                .map(|_row| ())
                .map_err(|error| {
                    DbError::from(classify_cratestack(CURRENCY_MODEL, "upsert", error))
                })?;
        }

        for provider in &providers {
            // Parsed before any statement is built, because
            // `CreateProviderInput::flow` is the schema's `ProviderFlow`
            // enum and the seed's is a `String` (Step 2's D4). Refusing
            // here rather than at `providers_flow_enum_check` is what makes
            // a bad label exit 78 instead of 69 — see
            // [`DbError::ProviderFlowUnknown`], which is where that
            // deliberate change of advice is argued.
            //
            // `map_err` and NOT `unwrap_or_default()`, which is a trap
            // rather than a style choice: `cratestack-macros` marks the
            // FIRST variant of every generated enum `#[default]`
            // (`types/enums.rs::variant_tokens`), and the first variant here
            // is `push`, so a default would record a typo'd rail as a push
            // rail and return `Ok`.
            let flow = provider
                .flow
                .parse()
                .map_err(|_: String| DbError::ProviderFlowUnknown {
                    code: provider.code.clone(),
                    flow: provider.flow.clone(),
                })?;

            // All eight columns, which is the whole of what migration 0033
            // bought: `cratestack-macros` drops every `@default(...)` field
            // from `Create{Model}Input`, so while the five capability
            // booleans carried one this struct could not name them and boot
            // step 4 would have stored the column defaults instead of the
            // deployment's configuration. The argument for the DDL is in
            // that migration's header; the schema half is in
            // `schemas/vpay.cstack`, and neither works without the other.
            //
            // Restore a `@default(...)` on any of the five and this literal
            // stops compiling before any test runs.
            let input = crate::schema::cratestack_schema::CreateProviderInput {
                code: provider.code.clone(),
                display_name: provider.display_name.clone(),
                flow,
                supports_refunds: provider.supports_refunds,
                supports_partial_refunds: provider.supports_partial_refunds,
                delivers_callbacks: provider.delivers_callbacks,
                requires_ip_allowlist: provider.requires_ip_allowlist,
                enabled: provider.enabled,
            };

            // `run_in_tx`, and NOT `run`. A provider written on its own
            // connection survives the rollback a later failure triggers, and
            // it would also commit ahead of the disable pass below — leaving
            // the half-reconciled table this function's transaction exists to
            // prevent.
            // `a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
            // in `tests/repositories.rs` is what turns that into a red
            // assertion.
            //
            // Deliberately NO `find_unique(...).for_update()` ahead of it,
            // which is the asymmetry with the currency pass. There the read
            // IS the guard — a stored exponent must never be overwritten, so
            // it has to be read and compared first. Here every one of the
            // eight columns is owned by configuration and overwriting it is
            // the point, so the read would return a row nothing compares; and
            // the row lock it would take is taken anyway, in this same
            // transaction, by `upsert`'s own conflict probe
            // (`upsert_exec.rs::run_upsert_in_tx` ->
            // `select_for_update_by_conflict_target` on `tx`). One measured
            // consequence, recorded because it inverts the currency finding:
            // with no row lock held here, `.run(&ctx)` fails in 1.2 s instead
            // of deadlocking. See docs/reference/vpay-db.md § CrateStack.
            //
            // What the conflict probe needs in exchange, and it is a real
            // constraint rather than an incidental one: this transaction has
            // to be able to SEE a row another writer committed while it was
            // waiting on the advisory lock above. That is READ COMMITTED,
            // Postgres's default and the one `pool.begin()` opens.
            // `a_reconcile_that_waited_for_the_boot_lock_overwrites_what_the_holder_committed`
            // in `tests/repositories.rs` is what pins it: under `SET
            // TRANSACTION ISOLATION LEVEL REPEATABLE READ` the snapshot is
            // taken when the lock statement starts, the probe cannot see the
            // committed row, and boot fails with `40001` (measured
            // 2026-09-06; no other reconcile case notices).
            self.cs
                .provider()
                .upsert(input)
                .run_in_tx(&mut tx, &ctx)
                .await
                .map(|_row| ())
                .map_err(|error| {
                    DbError::from(classify_cratestack(PROVIDER_MODEL, "upsert", error))
                })?;
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

#[cfg(test)]
mod tests {
    //! Three questions this module could not answer from its call sites
    //! alone, plus the one that needs a database.
    //!
    //! The first three are string and descriptor comparisons with no I/O at
    //! all: what CrateStack's generated upsert *renders*, and whether every
    //! action this module calls has an `@@allow` arm. They exist because
    //! both facts belong to a **pinned external crate** — a `cratestack`
    //! release that changed either would change `reconcile`'s observable
    //! behaviour with no diff in this repository — and because the
    //! `disabled_clients` measurement of 2026-09-06 showed that deleting an
    //! `@@allow` line left `cargo build`, `just clippy`, `just check-schema`
    //! and all ten `just verify` gates green.
    //!
    //! The fourth starts a container, because "the `Provider` model decodes
    //! against the real table" is not a claim a rendered string can make.

    use anyhow::Context as _;
    use sqlx::postgres::PgPoolOptions;

    use crate::error::DbError;
    use crate::migrations::Migrations as _;
    use crate::repository::PgRepositories;
    use crate::schema::cratestack_schema;

    /// A pool that has never opened a connection, and cannot: the port is
    /// unroutable. `connect_lazy` does no I/O and neither does
    /// `preview_sql`. Same shape and same reason as `disabled_clients.rs`'s
    /// helper of the same name; this crate's two CrateStack modules are
    /// separate compilation units for the reader, not for the compiler, and
    /// duplicating four lines is cheaper than a shared test helper module
    /// that has to be `pub(crate)`.
    fn lazy_cratestack() -> cratestack_schema::Cratestack {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("a lazy pool parses its URL and connects to nothing");
        cratestack_schema::Cratestack::builder(pool).build()
    }

    /// The reason `reconcile` reads the currency row before it upserts it,
    /// as an assertion rather than a paragraph.
    ///
    /// The hand-written statement this replaced was `ON CONFLICT (code) DO
    /// UPDATE SET exponent = **currencies**.exponent` — a no-op write whose
    /// `RETURNING` handed back the stored value so Rust could compare it.
    /// CrateStack renders `SET exponent = **EXCLUDED**.exponent`, which is
    /// the overwrite [`DbError::CurrencyExponentConflict`] exists to refuse:
    /// it would silently reinterpret every amount already stored in that
    /// currency, with no write to any of those rows.
    ///
    /// So the `find_unique(...).for_update()` in `reconcile` is not a
    /// prefetch that could be dropped for a round trip — it is the guard.
    /// If a future `cratestack` grows a way to express the no-op form, this
    /// test is where the note that the read could then be folded back in
    /// lives.
    #[tokio::test]
    async fn the_currency_upsert_would_overwrite_a_stored_exponent_on_its_own() {
        let cs = lazy_cratestack();
        let input = cratestack_schema::CreateCurrencyInput {
            code: "XAF".to_owned(),
            exponent: 0,
        };

        let sql = cs.currency().upsert(input).preview_sql();

        assert!(
            sql.starts_with(
                "INSERT INTO currencies (code, exponent) VALUES ($1, $2) \
                 ON CONFLICT (code) DO UPDATE SET exponent = EXCLUDED.exponent"
            ),
            "the generated currency upsert is not the statement `reconcile` reasons about: {sql}"
        );
        assert!(
            !sql.contains("SET exponent = currencies.exponent"),
            "if this ever renders the no-op form, the read-before-write in `reconcile` can be \
             reconsidered — and this test is the note explaining why it was there: {sql}"
        );

        // And the read that makes the write safe really does take a row
        // lock. `FOR UPDATE` is one `.for_update()` call away from absent,
        // and its absence is silent at runtime.
        let read = cs
            .currency()
            .find_unique("XAF".to_owned())
            .for_update()
            .preview_sql();
        assert!(
            read.ends_with("FROM currencies WHERE code = $1 LIMIT 1 FOR UPDATE"),
            "the exponent comparison must read under a row lock: {read}"
        );
    }

    /// The provider upsert carries **all eight** columns — the inverse of the
    /// test that stood here until 2026-09-06, and the assertion migration
    /// 0033 exists to make true.
    ///
    /// `cratestack-macros` drops every `@default(...)` field from both
    /// `Create{Model}Input` and `upsert_update_columns`
    /// (`model/inputs.rs::create_input_fields`,
    /// `model/descriptor/columns.rs`). `model Provider` carried a
    /// `@default(...)` on all five capability booleans, because migration
    /// 0002's table did, so the generated statement was
    ///
    /// ```text
    /// INSERT INTO providers (code, display_name, flow) VALUES ($1, $2, $3)
    /// ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, flow = EXCLUDED.flow
    /// ```
    ///
    /// and boot step 4 would have recorded every rail with the column
    /// defaults rather than the deployment's configuration — a rail an
    /// operator had just disabled coming back enabled. That is why this pass
    /// was hand-written SQL for a day, and why the predecessor of this test
    /// pinned the three-column form.
    ///
    /// **What makes this test worth keeping now that it asserts the happy
    /// shape.** The five `@default(...)` and the five column `DEFAULT`s have
    /// to move together or the drift report grows five lines, so a future
    /// editor who restores one half is stopped by the drift test. But an
    /// editor who restores a `@default(...)` in `schemas/vpay.cstack` alone
    /// is stopped *earlier* and more clearly: the field leaves
    /// `CreateProviderInput` and `reconcile`'s struct literal stops
    /// compiling, with `error[E0560]: struct \`inputs::CreateProviderInput\`
    /// has no field named \`supports_refunds\`` — measured in this
    /// direction. (exp17's review measured `E0063`, "missing field", going
    /// the *other* way, when the fields appeared and the literal did not yet
    /// name them; the two are not interchangeable and naming the wrong one
    /// sends a reader looking for the wrong mistake.) This test is what makes
    /// the *statement* — as opposed to the struct — a checked claim, so the
    /// assertions below are about which columns reach SQL rather than about
    /// which fields exist.
    ///
    /// The `starts_with` is the brittle assertion of the two and deliberately
    /// kept: a harmless upstream change (different identifier quoting, a
    /// reordered `RETURNING`) turns it red too. That is a loud false alarm —
    /// the message prints the SQL — rather than a silent false green, and the
    /// `contains` loop underneath is what pins the finding column by column.
    #[tokio::test]
    async fn the_provider_upsert_carries_all_eight_columns() {
        let cs = lazy_cratestack();
        let input = cratestack_schema::CreateProviderInput {
            code: "mtn_momo".to_owned(),
            display_name: "MTN MoMo".to_owned(),
            flow: cratestack_schema::ProviderFlow::push,
            supports_refunds: true,
            supports_partial_refunds: true,
            delivers_callbacks: true,
            requires_ip_allowlist: true,
            enabled: true,
        };

        let sql = cs.provider().upsert(input).preview_sql();

        assert!(
            sql.starts_with(
                "INSERT INTO providers (code, display_name, flow, supports_refunds, \
                 supports_partial_refunds, delivers_callbacks, requires_ip_allowlist, enabled) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, \
                 flow = EXCLUDED.flow"
            ),
            "the generated provider upsert is not the eight-column statement `reconcile` reasons \
             about. If a `@default(...)` came back to `model Provider`, the crate would not have \
             compiled — so this is an upstream change in `create_input_fields` or in how the \
             INSERT list is rendered: {sql}"
        );
        // Written out rather than built with `format!`: `sql_audit.rs` scans
        // this crate's sources for interpolation into anything that looks
        // like a statement, and it is right to — a `format!` next to a SQL
        // fragment is the shape it exists to catch. These are assertion
        // needles, and spelling them in full costs five lines and keeps that
        // audit free of an exception.
        //
        // The `DO UPDATE SET` half is the one that matters most and is the
        // half a `@default(...)` used to remove on its own: without these
        // five assignments the upsert inserts the configured capabilities on
        // a fresh database and then never carries a *change* to them, which
        // is a bug no first boot can show.
        for assignment in [
            "supports_refunds = EXCLUDED.supports_refunds",
            "supports_partial_refunds = EXCLUDED.supports_partial_refunds",
            "delivers_callbacks = EXCLUDED.delivers_callbacks",
            "requires_ip_allowlist = EXCLUDED.requires_ip_allowlist",
            "enabled = EXCLUDED.enabled",
        ] {
            assert!(
                sql.contains(assignment),
                "`{assignment}` is missing from the generated upsert's update list, so a \
                 capability change would never reach an existing row. Check whether a \
                 `@default(...)` returned to `model Provider` in `schemas/vpay.cstack`, and read \
                 backends/migrations/0033_providers-drop-capability-defaults.sql: {sql}"
            );
        }

        // `enabled` on its own line, because it is the column whose default
        // was `TRUE` rather than `FALSE`: while `@default(true)` was there, a
        // rail the deployment had disabled was inserted as ENABLED. A rail
        // taking money it was configured not to take is the worst of the five
        // and is what `a_rail_the_configuration_disables_is_not_re_enabled_by_reconcile`
        // in `tests/repositories.rs` proves against a real database.
        assert!(
            sql.contains(", enabled)") && sql.contains("$8"),
            "`enabled` must be in the INSERT column list and bound, or boot step 4 records every \
             rail as enabled whatever the deployment says: {sql}"
        );
    }

    /// A flow the schema's enum cannot name is refused by [`reconcile`]
    /// itself, with the category that tells an operator to fix the deploy.
    ///
    /// No database: the parse is upstream of every statement, which is the
    /// whole change this test pins. Before 2026-09-06 the same seed reached
    /// Postgres and came back as a `23514` from
    /// `providers_flow_enum_check` — [`DbError::Query`], `Category::Storage`,
    /// exit `69`, i.e. "wait for Postgres" for a word that will never become
    /// valid by waiting.
    ///
    /// The `unwrap_or_default()` trap this guards is real rather than
    /// theoretical: `cratestack-macros` derives `Default` on every generated
    /// enum with the FIRST variant as the default
    /// (`types/enums.rs::variant_tokens`), and the first variant of
    /// `ProviderFlow` is `push`. A `.unwrap_or_default()` in `reconcile`
    /// would therefore store a typo'd rail as a push rail and return `Ok`.
    ///
    /// [`reconcile`]: ConfigReconcile::reconcile
    #[test]
    fn an_unnameable_flow_is_a_deploy_problem_and_never_reaches_a_statement() {
        use std::str::FromStr as _;
        use vpay_core::{Category, Classify as _, Retry};

        assert!(
            cratestack_schema::ProviderFlow::from_str("redirekt").is_err(),
            "the parse this test is about has to fail for a label the enum cannot name"
        );
        assert_eq!(
            cratestack_schema::ProviderFlow::default(),
            cratestack_schema::ProviderFlow::push,
            "the generated `Default` is the first variant, which is why `reconcile` matches on \
             the parse result instead of calling `unwrap_or_default()`"
        );

        let error = DbError::ProviderFlowUnknown {
            code: "typo_rail".to_owned(),
            flow: "redirekt".to_owned(),
        };

        assert_eq!(error.category(), Category::Configuration);
        assert_eq!(error.category().exit_code(), 78);
        assert_eq!(error.retry(), Retry::Never);
        assert_ne!(
            error.category(),
            DbError::Query(sqlx::Error::RowNotFound).category(),
            "if this ever matches, the flow refusal has gone back to telling a supervisor to \
             wait for a database that is perfectly healthy"
        );

        // The sentence is the remediation: an operator sees this line and
        // nothing else, so it has to name the rail and the bad word.
        let text = error.to_string();
        assert!(text.contains("typo_rail"), "{text}");
        assert!(text.contains("redirekt"), "{text}");
    }

    /// Every action this module calls on `Currency` and `Provider` has an
    /// `@@allow` arm, asserted against the **compiled descriptor** and with
    /// no database.
    ///
    /// The same test `disabled_clients.rs` carries, for the same measured
    /// reason: on 2026-09-06 each `@@allow` line was deleted in turn from
    /// `schemas/vpay.cstack` and every gate stayed green — only a
    /// Postgres-backed test noticed. Two of the arms below are silent at
    /// runtime if they go missing, and they are not the same silence:
    ///
    ///   `Currency` `read`   -> SILENT and DANGEROUS. The policy compiles
    ///                          into the `WHERE`, so `find_unique` answers
    ///                          `None` for a row that exists, `reconcile`
    ///                          concludes "no stored exponent", and the
    ///                          upsert *overwrites* it — the exact
    ///                          reinterpretation of every stored amount that
    ///                          `CurrencyExponentConflict` exists to refuse.
    ///   `Currency` `create` -> LOUD, on every call (`upsert_exec.rs`
    ///                          pre-flights it before any SQL runs).
    ///   `Currency` `update` -> LOUD, but only on the conflict branch: the
    ///                          first boot of a fresh database succeeds and
    ///                          the second fails.
    ///   `Provider` `read`   -> SILENT, and today only a test notices, since
    ///                          no production path reads `providers` through
    ///                          CrateStack yet.
    ///   `Provider` `create` -> LOUD, on every boot, for `Currency`
    ///                          `create`'s reason.
    ///   `Provider` `update` -> LOUD, but only on the conflict branch: a
    ///                          fresh database's first boot succeeds and
    ///                          every later one fails. Measured 2026-09-06.
    ///
    /// The `Currency` `read` case is why this file's container test compares
    /// against a raw sqlx read rather than merely asserting the reconcile
    /// returned `Ok`.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use cratestack_schema::models::{CURRENCY_MODEL as currency, PROVIDER_MODEL as provider};

        assert!(
            !currency.read_allow_policies.is_empty(),
            "`@@allow(\"read\", auth().isSystem())` is missing from `model Currency`: \
             `reconcile` would read `None` for every stored currency and the upsert would \
             overwrite a recorded exponent instead of refusing to"
        );
        assert!(
            !currency.create_allow_policies.is_empty(),
            "`@@allow(\"create\", …)` is missing from `model Currency`: every boot would fail \
             the currency upsert with `Forbidden` -> `PersistenceError::Denied` -> \
             `Category::Internal`"
        );
        assert!(
            !currency.update_allow_policies.is_empty(),
            "`@@allow(\"update\", …)` is missing from `model Currency`: the first boot against a \
             fresh database would succeed and every later one would fail, because only the \
             conflict branch consults this slot"
        );
        assert!(
            !provider.read_allow_policies.is_empty(),
            "`@@allow(\"read\", auth().isSystem())` is missing from `model Provider`: the \
             CrateStack read would return `None` for a row that exists"
        );
        assert!(
            !provider.create_allow_policies.is_empty(),
            "`@@allow(\"create\", …)` is missing from `model Provider`: every boot would fail \
             the provider upsert with `Forbidden` -> `PersistenceError::Denied` -> \
             `Category::Internal`, before any SQL ran"
        );
        assert!(
            !provider.update_allow_policies.is_empty(),
            "`@@allow(\"update\", …)` is missing from `model Provider`: the first boot against a \
             fresh database would succeed and every later one would fail, because only the \
             conflict branch consults this slot — so a deployment could pass CI and crash-loop \
             on its second pod"
        );

        // `model Provider` deliberately has NO delete arm, for `Currency`'s
        // reason: `reconcile` disables a dropped rail rather than removing
        // it, because every `charges` and `provider_requests` row references
        // this table and a rail that has ever taken money must stay nameable.
        assert!(
            provider.delete_allow_policies.is_empty(),
            "`model Provider` grew a `@@allow(\"delete\", …)`; `reconcile` disables a dropped \
             rail and never deletes one, and the foreign keys would refuse anyway"
        );
        // Nothing deletes a currency either. A reference row a `DELETE`
        // could reach is a foreign key waiting to break.
        assert!(
            currency.delete_allow_policies.is_empty(),
            "`model Currency` grew a `@@allow(\"delete\", …)`; every `payment_intents`, \
             `charges` and `ledger_entries` row references this table"
        );

        // A `@@deny` wins over every arm above
        // (`push_action_policy_query` wraps the allow list in
        // `NOT (<deny>) AND (…)`), so an accidental one is worth failing on
        // here rather than in a container.
        for (model, slot, denies) in [
            ("Currency", "read", currency.read_deny_policies),
            ("Currency", "detail", currency.detail_deny_policies),
            ("Currency", "create", currency.create_deny_policies),
            ("Currency", "update", currency.update_deny_policies),
            ("Provider", "read", provider.read_deny_policies),
            ("Provider", "detail", provider.detail_deny_policies),
            ("Provider", "create", provider.create_deny_policies),
            ("Provider", "update", provider.update_deny_policies),
        ] {
            assert!(
                denies.is_empty(),
                "`model {model}` grew a `@@deny(\"{slot}\", …)`; it overrides every `@@allow` \
                 for that action and no call site expects one"
            );
        }
    }

    /// The one claim in migration 0032 that only a real database can settle:
    /// `providers.flow` decodes through CrateStack.
    ///
    /// This is the decisive test for the native-enum conversion, and it is a
    /// *parity* test rather than a "does not error" one for
    /// `disabled_clients.rs`'s reason: a missing `@@allow("read", …)`
    /// compiles into the `WHERE` clause, so the read would succeed and find
    /// nothing. Comparing against a raw `sqlx` read of the same row is what
    /// tells "the model decodes" apart from "the model returns nothing".
    ///
    /// **The mutation this exists for**, run on 2026-09-06 and recorded in
    /// `docs/plans/exp17-notes/opus.md`: delete `ALTER TABLE providers ALTER
    /// COLUMN flow TYPE TEXT` from migration 0032 and this test fails with a
    /// decode error, because `cratestack`'s generated row decoders read an
    /// enum column with `try_get::<String>()` and a native Postgres enum is
    /// not a `String` to sqlx. That is upstream issue #228's finding, pinned
    /// to a test instead of a paragraph.
    ///
    /// It lives here, in a `#[cfg(test)]` module inside the crate, rather
    /// than in `tests/repositories.rs`, because nothing outside `vpay-db`
    /// can reach `PgRepositories::cs` and **nothing should**: `providers`
    /// has no production reader through CrateStack yet, and adding a public
    /// `Providers` trait method purely to give this test a door would be
    /// publishing a capability vpay does not have.
    #[tokio::test]
    async fn a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx()
    -> anyhow::Result<()> {
        let container = vpay_testkit::containers::start_postgres_with_retry()
            .await
            .context("postgres:16-alpine container starts")?;
        let host = container.get_host().await.context("container host")?;
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .context("container port")?;
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = sqlx::PgPool::connect(&url)
            .await
            .context("connecting to the container")?;
        let repositories = PgRepositories {
            pool: pool.clone(),
            cs: cratestack_schema::Cratestack::builder(pool.clone()).build(),
        };
        repositories
            .run_migrations()
            .await
            .context("migrations must apply")?;

        let seed = super::ProviderSeed {
            code: "orange_money".to_owned(),
            display_name: "Orange Money".to_owned(),
            flow: "redirect".to_owned(),
            supports_refunds: true,
            supports_partial_refunds: true,
            delivers_callbacks: false,
            requires_ip_allowlist: true,
            enabled: true,
        };
        super::ConfigReconcile::reconcile(&repositories, &[], std::slice::from_ref(&seed))
            .await
            .context("seeding one rail through boot step 4")?;

        // The raw read, in the shape the rest of this crate uses.
        let (code, display_name, flow, supports_refunds, enabled): (
            String,
            String,
            String,
            bool,
            bool,
        ) = sqlx::query_as(
            "SELECT code, display_name, flow, supports_refunds, enabled FROM providers \
             WHERE code = $1",
        )
        .bind(&seed.code)
        .fetch_one(&pool)
        .await
        .context("the sqlx read must find the row boot step 4 just wrote")?;

        // The same row through CrateStack. `flow` is the field that could
        // not be decoded before migration 0032.
        let row = repositories
            .cs
            .provider()
            .find_unique(seed.code.clone())
            .run(&crate::persistence::system_context())
            .await
            .map_err(|error| anyhow::anyhow!("the CrateStack provider read failed: {error}"))?
            .context(
                "the CrateStack read found no row where sqlx found one. Either \
                 `@@allow(\"read\", auth().isSystem())` is gone from `model Provider` — the \
                 policy compiles into the WHERE clause, so a denied row is indistinguishable \
                 from an absent one — or the model no longer matches the table",
            )?;

        assert_eq!(row.code, code);
        assert_eq!(row.display_name, display_name);
        assert_eq!(
            row.flow.to_string(),
            flow,
            "the enum column must decode to the same label sqlx reads as text"
        );
        assert_eq!(row.flow.to_string(), seed.flow, "and to what was seeded");
        assert_eq!(row.supports_refunds, supports_refunds);
        assert_eq!(row.enabled, enabled);
        // Read straight off the seed as well, because the two reads agreeing
        // proves they agree — not that either is right. These are the five
        // columns CrateStack's own upsert could not have written (see
        // `the_provider_upsert_cannot_carry_the_capability_columns`), so
        // they are the ones worth checking against the deployment's
        // intention rather than against the database's echo.
        assert!(row.supports_refunds, "the seed said this rail refunds");
        assert!(row.supports_partial_refunds);
        assert!(!row.delivers_callbacks);
        assert!(row.requires_ip_allowlist);
        assert!(row.enabled);

        Ok(())
    }
}
