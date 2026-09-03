//! The connectivity check behind `/healthz`.

use crate::error::DbError;

#[async_trait::async_trait]
pub trait Health: Send + Sync {
    /// Proves the database is actually reachable right now, cheaply enough to be
    /// called on every poll of an HTTP healthcheck.
    ///
    /// `SELECT 1` rather than a real table read: it exercises the full
    /// round-trip — a pooled connection, or a fresh one if the pool is empty,
    /// plus the wire protocol and the server actually answering a query — without
    /// touching any vpay-owned table, an index, or lock contention on one. That
    /// keeps this honest (it is not a fabricated "ok", `backends/crates/vpay-api/
    /// src/lib.rs`'s own module doc) while staying the cheapest query that can
    /// prove it.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Healthcheck`] if the query does not complete — a dead
    /// database, a network partition, or the pool being unable to open a new
    /// connection within its configured timeout (see [`crate::connect`]).
    async fn check_connection(&self) -> Result<(), DbError>;
}

#[async_trait::async_trait]
impl Health for crate::repository::PgRepositories {
    async fn check_connection(&self) -> Result<(), DbError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(DbError::Healthcheck)?;
        Ok(())
    }
}
