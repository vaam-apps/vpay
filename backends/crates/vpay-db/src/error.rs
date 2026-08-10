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
}
