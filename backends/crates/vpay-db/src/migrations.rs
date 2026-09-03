//! Embedded migrations and the `run_migrations` entry point.

use crate::error::DbError;

#[async_trait::async_trait]
pub trait Migrations: Send + Sync {
    /// Applies every migration in `backends/migrations` that has not already
    /// run, in order, inside `sqlx`'s own migrations-tracking table.
    ///
    /// Idempotent by construction: `sqlx::migrate!` records each applied
    /// migration's checksum in the `_sqlx_migrations` table it manages, so
    /// calling this twice against the same database is a no-op the second time
    /// (proven by `tests/postgres.rs`), not a re-apply or an error.
    ///
    /// The path given to `sqlx::migrate!` is resolved relative to *this crate's*
    /// `Cargo.toml` (`backends/crates/vpay-db`), not the workspace root — two
    /// `../..` steps land on `backends/migrations`, mirroring the identical
    /// `sqlx::migrate!("../../migrations")` already used from
    /// `backends/tests/integration` (same directory depth from its own crate
    /// root). A workspace-root-relative path like `"backends/migrations"` would
    /// resolve to a directory that does not exist from here and fail to compile.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Migrate`] if any migration fails to apply — including
    /// a checksum mismatch against a migration already recorded as applied.
    async fn run_migrations(&self) -> Result<(), DbError>;
}

#[async_trait::async_trait]
impl Migrations for crate::repository::PgRepositories {
    async fn run_migrations(&self) -> Result<(), DbError> {
        sqlx::migrate!("../../migrations")
            .run(&self.pool)
            .await
            .map_err(DbError::Migrate)
    }
}
