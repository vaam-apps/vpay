//! The typed `PgPool` constructor and the pool-sizing policy behind it.

use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::error::DbError;

/// Maximum number of live Postgres connections the pool will hold.
///
/// `vpay-server` and `vpay-worker-bin` are each a single process with a
/// bounded, small amount of concurrent DB-touching work — there is no
/// `/v1/*` route yet (`docs/status.md`) and the worker's job loop does not
/// exist, so today this is headroom, not a measured need. 10 is chosen as a
/// deliberately small, conservative ceiling rather than a number tuned to
/// load that does not exist yet: Postgres' own default `max_connections` is
/// 100, and a single vpay process should never be able to starve the rest of
/// that budget (other processes, `psql`, a second replica during a rolling
/// deploy) on its own. Raise this only once real concurrent load is
/// measured, not speculatively.
const MAX_CONNECTIONS: u32 = 10;

/// How long a caller waits for a connection to become available from
/// [`sqlx::PgPool::acquire`] before giving up (`PgPoolOptions::acquire_timeout`).
///
/// This is the *only* timeout `sqlx` 0.8's `PoolOptions` exposes on the path
/// to a usable connection — there is no separate `connect_timeout` knob (an
/// older sqlx API had one; `sqlx-core` 0.8.6 removed it). Its own doc
/// comment says explicitly that `acquire_timeout` bounds *all* of: waiting
/// for a pool permit, testing an idle connection's liveness, **and**, when a
/// new connection must be opened, "I/O, handshaking, and initialization
/// commands" — i.e. it already is the connect timeout for a fresh
/// connection, not just a queueing bound. So this one constant does double
/// duty as both settings the task brief asked for ("acquire timeout, and a
/// connect timeout"): there is no second number to pick because sqlx does
/// not expose a second phase to bound separately.
///
/// 5 seconds is the number behind both of those cases: short enough that a
/// dead/unreachable database fails the boot-time `connect` in [`connect`]
/// well before an operator notices a hang ("a payment process that hangs
/// forever on a dead database is worse than one that fails fast" — task
/// brief), comfortably under a typical upstream/gateway timeout (often 30s)
/// so a saturated-pool failure at request time produces a clean 5xx instead
/// of the caller's own timeout firing first and masking the real cause, and
/// generous enough for a same-network Postgres (compose, a VPC) — a real
/// network partition or a wrong hostname fails in milliseconds, not
/// seconds — while still tolerating a slow-starting container locally.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// Builds a `PgPool` against `database_url` with vpay's pool policy.
///
/// This is eager: it returns only once at least one connection has been
/// established (or `ACQUIRE_TIMEOUT` has elapsed and it gives up) — never
/// a lazily-connecting pool that would let a caller believe startup
/// succeeded when the database is actually unreachable.
///
/// # Errors
///
/// Returns [`DbError::Connect`] if no connection could be established within
/// `ACQUIRE_TIMEOUT`.
pub async fn connect(database_url: &str) -> Result<PgPool, DbError> {
    PgPoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect(database_url)
        .await
        .map_err(DbError::Connect)
}
