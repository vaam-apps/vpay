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
            Self::Migrate(_) => Category::Configuration,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Connect(_) => "database_unreachable",
            Self::Healthcheck(_) => "database_unhealthy",
            Self::Query(_) => "database_query_failed",
            Self::Migrate(_) => "database_migration_failed",
        }
    }
}
