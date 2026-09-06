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
//! # Half of this module runs through CrateStack, and the half that does not
//! is the interesting half
//!
//! Since 2026-09-06 the currency pass is `find_unique(code).for_update()`
//! followed by `upsert(...)`, both `run_in_tx` on the transaction this
//! function opens and both after the advisory lock below — CrateStack joins
//! vpay's transaction, it never opens one of its own. The provider pass is
//! still a hand-written `INSERT ... ON CONFLICT`, and the reason is measured
//! rather than pending: CrateStack's generated upsert input cannot carry a
//! `@default(...)` column, and five of `providers`' eight columns have one.
//! See the comment on that loop, and
//! [docs/reference/vpay-db.md § CrateStack](../../../../docs/reference/vpay-db.md#cratestack).
//!
//! # `# Errors` moved, and callers should notice
//!
//! The currency statements now fail as [`DbError::Persistence`] rather than
//! [`DbError::Query`]. The *classification* is unchanged — a `23514` is
//! still `Category::Internal`, a pool timeout still `Category::Storage`,
//! because `persistence::classify_cratestack` and `error::classify_write`
//! are asserted against each other — so a caller branching on
//! `Classify::category` sees nothing; a caller matching the variant would
//! have silently stopped matching. Boot only ever reads the category.
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
    /// Written into a `TEXT` column guarded by
    /// `providers_flow_enum_check`, so any other string is still a
    /// [`DbError::Query`] at boot rather than a silently stored typo — the
    /// refusal moved from the native `provider_flow` enum type to a CHECK
    /// constraint in migration 0032, which is a change of *mechanism* and
    /// not of behaviour. It had to move: `cratestack`'s generated row
    /// decoders read an enum column with `try_get::<String>()`, so a native
    /// enum column fails to decode on every read through that layer.
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
    /// currency. [`DbError::Persistence`] if either currency statement fails,
    /// including when a model policy refuses one: every CrateStack call here
    /// runs as the system principal, so a refusal means
    /// `schemas/vpay.cstack` and this module disagree, and it classifies
    /// `Category::Internal` rather than `Forbidden`.
    /// [`DbError::Query`] if a `flow` is not `push` or `redirect` (the
    /// `providers_flow_enum_check` CHECK, which replaced the
    /// `provider_flow` enum type in migration 0032), if
    /// `supports_partial_refunds` is set without `supports_refunds` (the
    /// `partial_refunds_imply_refunds` CHECK), or if any provider statement
    /// or the commit fails. All of them roll the whole transaction back.
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
            // Two pooled connections on the conflict branch, not one:
            // `upsert_resolve.rs::gate_update_policy` runs its update-policy
            // probe on `runtime.pool()` while this transaction still holds a
            // connection of its own. That probe is a plain `SELECT 1 ... AND
            // (<policy>)` with no `FOR UPDATE`, so it does **not** block on
            // the row this transaction just locked — Postgres's MVCC reads
            // are not blocked by writers — and boot cannot deadlock against
            // itself. It does mean two of `pool.rs`'s `MAX_CONNECTIONS = 10`
            // per in-flight reconcile, which is comfortable for a boot step
            // the advisory lock already admits one at a time.
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
            // Still a hand-written statement, and NOT because nobody got to
            // it. `CreateProviderInput` — the input CrateStack's `upsert`
            // takes — carries only `code`, `display_name` and `flow`.
            // `cratestack-macros`' `model/inputs.rs::create_input_fields` and
            // `model/descriptor/columns.rs`'s `upsert_update_columns` both
            // drop every field with a `@default(...)`, and `model Provider`
            // in `schemas/vpay.cstack` carries one on all five capability
            // booleans because the live table does
            // (`DEFAULT FALSE` / `DEFAULT TRUE`, migration 0002).
            //
            // So a CrateStack upsert here would insert a rail with
            // `supports_refunds = false, supports_partial_refunds = false,
            // delivers_callbacks = false, requires_ip_allowlist = false,
            // enabled = true` **whatever the deployment configured**, and
            // would never carry a capability change to an existing row. A
            // rail an operator had just disabled would come back enabled.
            // That is a plausible-looking success writing the wrong value,
            // which is the one thing AGENTS.md rule 2 is about.
            //
            // The rendered statement is pinned by
            // `the_provider_upsert_cannot_carry_the_capability_columns`
            // below, so this paragraph is a test rather than a claim, and so
            // that an upstream release which fixes it turns that test red —
            // which is the signal to move this statement. What it would take
            // on vpay's side instead is a maintainer's call and is written up
            // in docs/reference/vpay-db.md § CrateStack.
            //
            // What did change in migration 0032: `$3` is no longer cast to
            // `::provider_flow`. That type no longer exists — `flow` is TEXT
            // guarded by `providers_flow_enum_check` — and a bind of an
            // unknown flow is refused by the CHECK exactly as the enum
            // refused it, as
            // `an_unknown_provider_flow_is_refused_by_the_check_that_replaced_the_enum_type`
            // in `backends/tests/integration/tests/postgres_smoke.rs`
            // proves. (This named `an_unknown_provider_flow_is_refused_by_
            // the_database` until 2026-09-06, which is not a test that has
            // ever existed. Nothing gates a citation to a test name —
            // `verify-status` lexes `NotImplemented` tokens, not these — so
            // the only thing that catches one is a reader trying to open it.)
            sqlx::query(
                "INSERT INTO providers \
                 (code, display_name, flow, supports_refunds, supports_partial_refunds, \
                  delivers_callbacks, requires_ip_allowlist, enabled) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
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

    /// Why `reconcile`'s **provider** pass is still a hand-written
    /// `INSERT ... ON CONFLICT` — measured, and pinned so that the day it
    /// stops being true, this test says so.
    ///
    /// `cratestack-macros` drops every `@default(...)` field from both
    /// `Create{Model}Input` and `upsert_update_columns`
    /// (`model/inputs.rs::create_input_fields`,
    /// `model/descriptor/columns.rs`), and `model Provider` carries a
    /// `@default(...)` on all five capability booleans because the live
    /// table does. A CrateStack upsert here would therefore write
    /// `supports_refunds = false … enabled = true` whatever the deployment
    /// configured, and would never carry a capability change to an existing
    /// row.
    ///
    /// That is the failure this repository's second rule is about: a
    /// plausible-looking success storing the wrong value. `docs/status.md`
    /// carries the row, and `schemas/vpay.cstack`'s `Provider` GAP note
    /// carries what it would take to unblock it — which is a schema decision
    /// (drop five DB defaults) rather than a code change, and is left to a
    /// maintainer.
    #[tokio::test]
    async fn the_provider_upsert_cannot_carry_the_capability_columns() {
        let cs = lazy_cratestack();
        let input = cratestack_schema::CreateProviderInput {
            code: "mtn_momo".to_owned(),
            display_name: "MTN MoMo".to_owned(),
            flow: cratestack_schema::ProviderFlow::push,
        };

        let sql = cs.provider().upsert(input).preview_sql();

        assert!(
            sql.starts_with(
                "INSERT INTO providers (code, display_name, flow) VALUES ($1, $2, $3) \
                 ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, \
                 flow = EXCLUDED.flow"
            ),
            "{sql}"
        );
        // Written out rather than built with `format!`: `sql_audit.rs` scans
        // this crate's sources for interpolation into anything that looks
        // like a statement, and it is right to — a `format!` next to a SQL
        // fragment is the shape it exists to catch. These are assertion
        // needles, and spelling them in full costs five lines and keeps that
        // audit free of an exception.
        for assignment in [
            "supports_refunds = EXCLUDED.supports_refunds",
            "supports_partial_refunds = EXCLUDED.supports_partial_refunds",
            "delivers_callbacks = EXCLUDED.delivers_callbacks",
            "requires_ip_allowlist = EXCLUDED.requires_ip_allowlist",
            "enabled = EXCLUDED.enabled",
        ] {
            assert!(
                !sql.contains(assignment),
                "`{assignment}` is now rendered by the generated upsert. If this failed after a \
                 cratestack bump, `reconcile`'s provider loop can move off hand-written SQL — \
                 read the comment on that loop first: {sql}"
            );
        }
        assert!(
            !sql.contains("(code, display_name, flow, supports_refunds"),
            "the five capability columns must be absent from the INSERT list too, or this test \
             is asserting only half of the problem: {sql}"
        );
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

        // `model Provider` deliberately has NO create/update/delete arm:
        // nothing writes it through CrateStack (see
        // `the_provider_upsert_cannot_carry_the_capability_columns`), and an
        // arm no call site uses is a permission nothing can measure. If this
        // fails, the write moved — check that it moved deliberately.
        assert!(
            provider.create_allow_policies.is_empty()
                && provider.update_allow_policies.is_empty()
                && provider.delete_allow_policies.is_empty(),
            "`model Provider` grew a write arm. `reconcile`'s provider pass is hand-written SQL \
             and needs none; grant one in the commit that moves the write, not before it"
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
