//! [`DbError`], the one error type this crate returns, and
//! [`classify_write`] — the single place a Postgres integrity violation is
//! turned into one of its variants.

/// Everything that can go wrong talking to Postgres, from this crate's own
/// responsibilities: building a pool, running migrations, checking
/// connectivity, and the repository queries the `/v1` handlers run.
///
/// Deliberately not a wrapper around a single `sqlx::Error` — the variants
/// let a caller (`main.rs` in either binary, `/healthz`, or a `/v1` handler)
/// log and react differently: a `Connect` failure at startup should abort
/// the process loudly (`AGENTS.md`: a payment process that hangs or boots
/// half-working is worse than one that fails fast); a `Healthcheck` failure
/// at request time should return `503`, not crash the server; a
/// [`DbError::UniqueViolation`] on `one_charge_per_intent` is a `409` a
/// merchant can act on, not a `503` telling them vpay is broken.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The connection pool could not be built — most commonly a dead host,
    /// a refused connection, or bad credentials. Surfaced by [`crate::connect`].
    #[error("failed to connect to Postgres: {0}")]
    Connect(#[source] sqlx::Error),

    /// A migration failed to apply — a broken migration file, a conflicting
    /// schema already present, or a lost connection mid-run. Surfaced by
    /// [`crate::Migrations::run_migrations`].
    #[error("failed to run database migrations: {0}")]
    Migrate(#[source] sqlx::migrate::MigrateError),

    /// The liveness query itself failed — the pool exists but the database
    /// is not answering right now. Surfaced by [`crate::Health::check_connection`].
    #[error("database healthcheck query failed: {0}")]
    Healthcheck(#[source] sqlx::Error),

    /// A repository query failed — the disabled-clients kill-switch lookup,
    /// or the signing-key repository's read/rotate queries. Kept as one
    /// opaque variant across both, mirroring `Healthcheck`/`Connect`'s own
    /// granularity: callers of these repositories (a token-issuance path, an
    /// operator endpoint, a JWKS handler) all want the same thing on
    /// failure — log the real cause and fail the request — not a different
    /// branch per query.
    #[error("database query failed: {0}")]
    Query(#[source] sqlx::Error),

    /// A signing key this process was told to activate is already in
    /// `oauth_signing_keys` and has been **retired** — the shape a deploy
    /// rollback to an older Secret mount takes.
    ///
    /// Its own variant rather than the `Query` a duplicate-key violation
    /// would produce, for two reasons that both matter at 3am. It is not a
    /// storage failure: retrying it against a healthy database fails
    /// identically, so classifying it as `Storage` would tell a supervisor
    /// to wait for Postgres (exit `69`) when the actual fix is a deploy
    /// (exit `78`) — see the `Classify` impl below. And its `Display` is the
    /// whole remediation: an operator reading
    /// `error: duplicate key value violates unique constraint
    /// "oauth_signing_keys_pkey"` in a crash loop has to know this table's
    /// schema to work out what happened, whereas this sentence names the
    /// key, when it was retired, and the two ways out.
    ///
    /// Re-activating the retired row instead is deliberately **not** what
    /// this crate does — that needs a policy decision about `expires_at` and
    /// about whether a key retired on purpose may publish again, and the
    /// rotation policy is an open maintainer question (`docs/roadmap.md`,
    /// "Open — signing-key rotation overlap window"). Refusing loudly is the
    /// honest behaviour until it is settled; this variant only makes the
    /// refusal legible.
    #[error(
        "signing key {kid} was retired at {retired_at}; rolling back to a retired key is refused \
         — generate a new key or restore the current one"
    )]
    SigningKeyRetired {
        /// The `kid` that was asked for and is present-but-retired.
        kid: String,
        /// When the row stopped being the active key (`updated_at`, which
        /// the retiring `UPDATE` sets in the same statement that clears
        /// `active`). Not `expires_at`, which is a *future* instant — the
        /// end of the key's publish-only overlap window — and would read as
        /// a retirement date that has not happened yet.
        retired_at: time::OffsetDateTime,
    },

    /// A write lost a race against a database uniqueness rule (SQLSTATE
    /// `23505`) — canonically `one_charge_per_intent`, the plain unique
    /// index that makes "one charge per intent, forever" (`AGENTS.md`) a
    /// property of the schema rather than of whichever handler happened to
    /// check first.
    ///
    /// Its own variant rather than [`DbError::Query`] because the two
    /// belong to different *people*. A duplicate charge is the merchant's
    /// answer to give — confirm the same intent twice and the second call
    /// is a `409`, which is actionable — whereas `Query` classifies as
    /// `Category::Storage` and would tell that merchant vpay is
    /// temporarily unavailable and to retry, i.e. to try submitting the
    /// duplicate charge again. Getting this wrong is not cosmetic: retry
    /// advice on a *payment* is advice to charge a payer twice.
    ///
    /// The constraint name is carried so a caller can tell which rule it
    /// hit without re-deriving it from a `sqlx::Error`'s text, and so a
    /// test can assert the specific rule rather than "some write failed".
    #[error("write violates the unique constraint {constraint}: {source}")]
    UniqueViolation {
        /// The Postgres constraint or index name, e.g.
        /// `one_charge_per_intent`. `<unnamed>` if the driver reported none
        /// — never a guess.
        constraint: String,
        /// The underlying driver error, kept whole for operator logs.
        #[source]
        source: sqlx::Error,
    },

    /// A write referenced a row that does not exist (SQLSTATE `23503`) —
    /// an unknown `currency_code`, a `provider_code` that was never seeded,
    /// a charge pointing at a missing intent.
    ///
    /// `Category::InvalidRequest`, not `Storage`: every foreign key in this
    /// schema points at *reference* data (currencies, providers) or at an
    /// object the caller named, so the database refusing the reference
    /// means the request named something that is not there. The database is
    /// working exactly as intended, so telling the caller to retry would be
    /// telling them to re-send a request that can never succeed.
    #[error("write violates the foreign key constraint {constraint}: {source}")]
    ForeignKeyViolation {
        /// The Postgres constraint name, e.g.
        /// `payment_intents_currency_code_fkey`.
        constraint: String,
        /// The underlying driver error, kept whole for operator logs.
        #[source]
        source: sqlx::Error,
    },

    /// A write that must affect exactly one row affected none: either the
    /// row named does not exist, or it was no longer in the state the write
    /// required.
    ///
    /// Raised by the two compare-and-swap writes that have nothing to
    /// return — `idempotency::store` (`WHERE ... AND claim_id = $3 AND
    /// state = 'in_flight'`) and `provider_requests::record_response`. Both
    /// are called by a process that just created the row they address, so
    /// zero rows matched means an invariant this crate's callers guarantee
    /// has been broken: a key completed twice, or a provider request
    /// answered twice. Hence `Category::Internal` — it pages, rather than
    /// being reported to a merchant as their mistake.
    ///
    /// `idempotency::store` deliberately does **not** raise this when the
    /// row has been reclaimed by a later request or swept: that is a
    /// legitimate consequence of the 24-hour window rather than a bug, and
    /// it is reported as `IdempotencyStoreOutcome::StaleClaim` instead. See
    /// that function for how the two are told apart.
    ///
    /// The alternative was to return `Ok(())` and log, which is precisely
    /// the "plausible-looking success" `CLAUDE.md` forbids: the caller
    /// would go on to answer `200` having stored nothing, and the next
    /// replay of that key would re-execute the payment.
    #[error("no row in {table} matched {key}, or it was no longer in the required state")]
    WriteMatchedNoRow {
        /// The table the write addressed, for the operator reading the log.
        table: &'static str,
        /// The key that matched nothing, rendered by the caller. Never a
        /// secret — an idempotency key or a numeric row id.
        key: String,
    },

    /// A `currencies` row already exists with a different `exponent` than
    /// the boot-time seed claims — e.g. the deployment says `XAF` has two
    /// decimal places while the database recorded zero.
    ///
    /// Refused rather than upserted, and this is the whole reason
    /// `config_reconcile` reads the stored exponent — with
    /// `find_unique(code).for_update()` since 2026-09-06 — before it upserts,
    /// instead of letting `DO UPDATE SET exponent = EXCLUDED.exponent` land.
    /// That statement is exactly what CrateStack's `upsert` renders, which is
    /// why the read is not an optimisation there but the guard itself. The exponent is not a
    /// per-deployment setting; it is a property of the currency itself
    /// (`docs/flows/money.md`, and migration 0001's own comment), and every
    /// amount already stored is an integer count of minor units *at the
    /// recorded exponent*. Silently changing it would multiply or divide
    /// every historical amount in that currency by ten with no write to any
    /// of those rows — an entire ledger misread, with nothing in the audit
    /// trail to show it happened.
    ///
    /// `Category::Configuration` (exit `78`) because the fix is a corrected
    /// deployment, never a retry.
    #[error(
        "currency {code} is recorded with exponent {stored} but this deployment seeds exponent \
         {seeded}; refusing to change it — every amount already stored in {code} is a count of \
         minor units at exponent {stored}"
    )]
    CurrencyExponentConflict {
        /// The ISO-4217 code whose exponent disagrees.
        code: String,
        /// What the database already holds, and what existing amounts mean.
        ///
        /// `i64`, not `i32`, since migration 0032 widened
        /// `currencies.exponent` to `BIGINT` — `Int` in a `.cstack` model
        /// always emits `int8`, and an `int4` column is not a narrower `Int`
        /// to `cratestack`, it is a column its introspector refuses to map at
        /// all. The value is still bounded to `0..=4` two layers upstream
        /// (`vpay_config`'s `Config::validate_all`, then
        /// `currencies_exponent_range_check`); the width is about which
        /// integer the column *is*, not about how large an exponent vpay
        /// accepts.
        stored: i64,
        /// What this deployment's configuration asked for.
        seeded: i64,
    },

    /// A query that ran through CrateStack's data layer rather than through
    /// a hand-written `sqlx` statement failed.
    ///
    /// `transparent`, and its classification is *delegated* rather than
    /// re-decided (ADR-0011, ADR-0016 standard 1): [`crate::PersistenceError`]
    /// is a leaf that has already made every decision this variant could
    /// make, and a second opinion here is how the two persistence paths would
    /// start disagreeing about whether a duplicate charge is a `409` or a
    /// `503`. `cargo xtask verify-errors` fails if the `Classify` impl below
    /// ever answers for this variant with a wildcard instead of naming it.
    ///
    /// One variant rather than one per CrateStack failure for the same
    /// reason [`Self::Query`] is one variant across every hand-written read:
    /// the interesting distinctions are inside the leaf, and a caller that
    /// wants them matches on it.
    #[error(transparent)]
    Persistence(#[from] crate::PersistenceError),
}

/// Maps a failed *write* onto the variant that says whose problem it is:
/// SQLSTATE `23505` → [`DbError::UniqueViolation`], `23503` →
/// [`DbError::ForeignKeyViolation`], anything else → [`DbError::Query`].
///
/// Every write in this crate goes through this one function rather than
/// `map_err(DbError::Query)`, so a new repository cannot quietly reclassify
/// a duplicate charge as a storage outage. It is the *only* place SQLSTATE
/// strings appear.
///
/// Reads (`SELECT`) deliberately keep `map_err(DbError::Query)`: neither
/// integrity violation can arise from one, so routing them through here
/// would suggest a distinction that cannot occur.
///
/// Not `pub`: callers of this crate consume [`DbError`], never raw
/// `sqlx::Error`s — nothing outside these repositories has an
/// `sqlx::Error` to classify in the first place.
pub(crate) fn classify_write(error: sqlx::Error) -> DbError {
    // `constraint()` is `None` for violations Postgres reports without one
    // (and for every non-database error). `<unnamed>` is a placeholder that
    // reads as one in a log; inventing a plausible constraint name here
    // would make a test asserting the name pass against the wrong rule.
    let Some(db_error) = error.as_database_error() else {
        return DbError::Query(error);
    };
    let code = db_error.code().unwrap_or_default().into_owned();
    let constraint = db_error.constraint().unwrap_or("<unnamed>").to_owned();

    match code.as_str() {
        "23505" => DbError::UniqueViolation {
            constraint,
            source: error,
        },
        "23503" => DbError::ForeignKeyViolation {
            constraint,
            source: error,
        },
        // Includes `23514` (a CHECK violation) on purpose: those guard
        // invariants the application is supposed to have enforced before
        // writing — a metadata blob that is not an object, an amount below
        // zero — so hitting one is a vpay bug reaching Postgres, not a
        // caller error to hand back with a `param`. `Query` is where the
        // unclassified live until one of them earns its own variant.
        _ => DbError::Query(error),
    }
}

impl vpay_core::Classify for DbError {
    fn category(&self) -> vpay_core::Category {
        use vpay_core::Category;
        match self {
            // The next request may well succeed; this one must fail rather
            // than guess. Retried after backoff, 503 on the wire, exit 69
            // at startup — all from the category's defaults.
            Self::Connect(_) | Self::Healthcheck(_) | Self::Query(_) => Category::Storage,
            // A migration that fails to apply is a broken deploy, not a
            // transient outage: retrying it against the same schema fails
            // the same way. Classified as Configuration so a supervisor sees
            // exit 78 ("fix the deploy") rather than 69 ("wait for Postgres").
            //
            // A rollback to a retired signing key is the same kind of thing
            // — the deployed Secret names a key this database has already
            // moved past — and the same 78 is what tells a supervisor not to
            // sit in a restart loop waiting for a database that is fine.
            //
            // A currency whose exponent disagrees with the seed is the third
            // shape of the same thing: the database is healthy and the
            // deployment is wrong, and booting on regardless would misread
            // every stored amount in that currency.
            Self::Migrate(_)
            | Self::SigningKeyRetired { .. }
            | Self::CurrencyExponentConflict { .. } => Category::Configuration,
            // The database enforced a rule the request broke. Conflict, not
            // Storage: the answer is `409` and "do not repeat this", never
            // `503` and "retry" — see the variant's own comment for why
            // retry advice on a duplicate charge is dangerous rather than
            // merely unhelpful.
            Self::UniqueViolation { .. } => Category::Conflict,
            // The request named a currency, provider or object that does not
            // exist. Nothing about retrying it unchanged can succeed.
            Self::ForeignKeyViolation { .. } => Category::InvalidRequest,
            // Nobody outside vpay can cause this, and no retry fixes it:
            // a compare-and-swap this crate's own caller was supposed to
            // have set up matched nothing.
            Self::WriteMatchedNoRow { .. } => Category::Internal,
            // Delegated, never re-decided. Named explicitly rather than
            // caught by a wildcard, which is both ADR-0011's rule and what
            // `verify-errors` checks.
            Self::Persistence(error) => error.category(),
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Connect(_) => "database_unreachable",
            Self::Healthcheck(_) => "database_unhealthy",
            Self::Query(_) => "database_query_failed",
            Self::Migrate(_) => "database_migration_failed",
            Self::SigningKeyRetired { .. } => "signing_key_retired",
            // Not `Category::Conflict`'s default `invalid_state`: the
            // object is not in a *state* that forbids the write, it already
            // exists. A merchant branching on this code needs to tell "you
            // already did this" from "this intent cannot be cancelled now".
            Self::UniqueViolation { .. } => "resource_conflict",
            Self::ForeignKeyViolation { .. } => "invalid_reference",
            Self::CurrencyExponentConflict { .. } => "currency_exponent_conflict",
            Self::WriteMatchedNoRow { .. } => "write_matched_no_row",
            Self::Persistence(error) => error.code(),
        }
    }
}

#[cfg(test)]
mod tests {
    use vpay_core::{Category, Classify as _, Retry};

    use super::DbError;

    /// The whole point of the variant: exit `78` ("fix the deploy"), not the
    /// `69` every other `DbError` produces, and never a retry.
    ///
    /// Written against `Category` rather than against the numbers so it
    /// pins the *decision* this crate makes; `vpay-core` pins what the
    /// category then means, and `vpay-server`'s `exit_code_for` reads it
    /// through `find_in_chain::<DbError>`.
    #[test]
    fn a_retired_signing_key_is_a_deploy_problem_not_a_storage_one() {
        let error = DbError::SigningKeyRetired {
            kid: "kid_old".to_owned(),
            retired_at: time::OffsetDateTime::UNIX_EPOCH,
        };

        assert_eq!(error.category(), Category::Configuration);
        assert_eq!(error.category().exit_code(), 78);
        assert_eq!(error.retry(), Retry::Never);
        assert_eq!(error.code(), "signing_key_retired");
        assert_ne!(
            error.category(),
            DbError::Query(sqlx::Error::RowNotFound).category(),
            "if this ever matches, the variant has stopped earning its place"
        );

        // The sentence is the remediation, so it has to name the key and
        // both ways out — an operator reading a crash loop gets this line
        // and nothing else.
        let text = error.to_string();
        assert!(text.contains("kid_old"), "{text}");
        assert!(text.contains("generate a new key"), "{text}");
        assert!(text.contains("restore the current one"), "{text}");
    }

    /// The two integrity violations must not answer like a storage outage.
    ///
    /// Asserted here as well as in `tests/repositories.rs` (which proves
    /// real Postgres produces these variants at all) because *this* is the
    /// half that decides what a merchant is told: `Storage` would put
    /// "vpay is temporarily unavailable, retry" on a duplicate charge — an
    /// instruction to charge a payer twice — and `Retry::AfterBackoff`
    /// would make the worker do it without asking.
    #[test]
    fn integrity_violations_are_the_callers_problem_not_a_storage_outage() {
        let duplicate = DbError::UniqueViolation {
            constraint: "one_charge_per_intent".to_owned(),
            source: sqlx::Error::RowNotFound,
        };
        assert_eq!(duplicate.category(), Category::Conflict);
        assert_eq!(duplicate.category().http_status(), 409);
        assert_eq!(duplicate.retry(), Retry::Never);
        assert_eq!(duplicate.code(), "resource_conflict");
        assert!(
            duplicate.to_string().contains("one_charge_per_intent"),
            "the operator log has to name the rule that fired: {duplicate}"
        );

        let dangling = DbError::ForeignKeyViolation {
            constraint: "payment_intents_currency_code_fkey".to_owned(),
            source: sqlx::Error::RowNotFound,
        };
        assert_eq!(dangling.category(), Category::InvalidRequest);
        assert_eq!(dangling.category().http_status(), 400);
        assert_eq!(dangling.retry(), Retry::Never);
        assert_eq!(dangling.code(), "invalid_reference");

        // Neither may leak the constraint name — an internal schema
        // identifier — to the merchant reading the envelope.
        for error in [&duplicate, &dangling] {
            let public = error.public_message();
            assert!(
                !public.contains("constraint") && !public.contains("fkey"),
                "public messages name nothing internal: {public}"
            );
        }
    }

    /// A compare-and-swap that matched nothing pages rather than being
    /// blamed on the caller, and a currency-exponent disagreement stops the
    /// boot with exit `78` instead of being upserted away.
    #[test]
    fn the_two_invariant_refusals_classify_as_ours_not_the_callers() {
        let unmatched = DbError::WriteMatchedNoRow {
            table: "idempotency_keys",
            key: "merchant_a/key-1".to_owned(),
        };
        assert_eq!(unmatched.category(), Category::Internal);
        assert_eq!(unmatched.retry(), Retry::Never);

        let exponent = DbError::CurrencyExponentConflict {
            code: "XAF".to_owned(),
            stored: 0,
            seeded: 2,
        };
        assert_eq!(exponent.category(), Category::Configuration);
        assert_eq!(exponent.category().exit_code(), 78);
        // The sentence is the remediation: it has to say what the database
        // holds, what the deployment asked for, and why vpay will not just
        // take the new value.
        let text = exponent.to_string();
        assert!(text.contains("XAF"), "{text}");
        assert!(text.contains("exponent 0"), "{text}");
        assert!(text.contains("exponent 2"), "{text}");
        assert!(text.contains("refusing to change it"), "{text}");
    }
}
