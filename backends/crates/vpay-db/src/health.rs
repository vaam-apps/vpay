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

    /// How full this process's connection pool is right now. No I/O: the
    /// pool answers from its own counters, so it is safe to call from a
    /// metrics sampler on a database that is down — which is when a pool
    /// reading matters most.
    fn pool_stats(&self) -> PoolStats;
}

/// A reading of one connection pool, as `sqlx` reports it.
///
/// `size` counts every open connection (checked out or not), so the
/// checked-out ones are `size - idle`. Taken without a lock the pool holds
/// across the two reads, so a sampler can see them a moment apart; `idle` is
/// clamped to `size` so a skewed pair never reports a negative `in_use`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolStats {
    /// Open connections, idle or not.
    pub size: u32,
    /// Open connections waiting in the pool.
    pub idle: u32,
    /// The configured upper bound.
    pub max: u32,
}

impl PoolStats {
    /// Connections checked out by a request, a job or a listener.
    #[must_use]
    pub const fn in_use(&self) -> u32 {
        self.size.saturating_sub(self.idle)
    }
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

    fn pool_stats(&self) -> PoolStats {
        let size = self.pool.size();
        let idle = u32::try_from(self.pool.num_idle()).unwrap_or(u32::MAX);
        PoolStats {
            size,
            idle: idle.min(size),
            max: self.pool.options().get_max_connections(),
        }
    }
}
