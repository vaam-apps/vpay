//! [`DbError`], the one error type this crate returns.

/// Everything that can go wrong talking to Postgres, from this crate's own
/// three responsibilities: building a pool, running migrations, and checking
/// connectivity.
///
/// Deliberately not a wrapper around a single `sqlx::Error` — the three
/// variants let a caller (`main.rs` in either binary, or `/healthz`) log and
/// react differently: a `Connect` failure at startup should abort the
/// process loudly (`AGENTS.md`: a payment process that hangs or boots
/// half-working is worse than one that fails fast); a `Healthcheck` failure
/// at request time should return `503`, not crash the server.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The connection pool could not be built — most commonly a dead host,
    /// a refused connection, or bad credentials. Surfaced by [`crate::connect`].
    #[error("failed to connect to Postgres: {0}")]
    Connect(#[source] sqlx::Error),

    /// A migration failed to apply — a broken migration file, a conflicting
    /// schema already present, or a lost connection mid-run. Surfaced by
    /// [`crate::run_migrations`].
    #[error("failed to run database migrations: {0}")]
    Migrate(#[source] sqlx::migrate::MigrateError),

    /// The liveness query itself failed — the pool exists but the database
    /// is not answering right now. Surfaced by [`crate::check_connection`].
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
            Self::Migrate(_) | Self::SigningKeyRetired { .. } => Category::Configuration,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Connect(_) => "database_unreachable",
            Self::Healthcheck(_) => "database_unhealthy",
            Self::Query(_) => "database_query_failed",
            Self::Migrate(_) => "database_migration_failed",
            Self::SigningKeyRetired { .. } => "signing_key_retired",
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
}
