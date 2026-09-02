//! Database connectivity for vpay: a typed connection pool, embedded schema
//! migrations, and a cheap liveness check — the three things that were
//! previously entirely absent from every shipping binary (`docs/status.md`,
//! "Database schema / migrations (core)": eight migrations existed and were
//! proven against real Postgres by `backends/tests/integration`, but nothing
//! in the application ever opened a connection).
//!
//! # `rustls::crypto::CryptoProvider::install_default()` — investigated, not
//! needed here
//!
//! The root `Cargo.toml`'s own comment on the `authkestra-*` dependencies
//! documents a real requirement: those crates build `reqwest` clients with
//! `rustls-no-provider`, which means the *first* one constructed panics
//! unless a process-wide default `CryptoProvider` was already installed.
//! `sqlx` is configured with the `tls-rustls-ring` feature, which vendors
//! Mozilla's CA bundle via `webpki-roots` (see the root `Cargo.toml`'s own
//! comment on that dependency: the runtime image is `FROM scratch` per
//! ADR-0004, so there is no OS trust store for the `-native-roots`
//! alternative to read). That looks like the same hazard — it is not.
//! Reading `sqlx-core` 0.8.6's own TLS
//! setup (`sqlx-core-0.8.6/src/net/tls/tls_rustls.rs`) shows it never calls
//! `rustls::crypto::CryptoProvider::get_default()` (the call that panics
//! without an installed default). Instead it builds its own provider inline
//! and passes it explicitly:
//!
//! ```text
//! let provider = Arc::new(rustls::crypto::ring::default_provider());
//! let config = ClientConfig::builder_with_provider(provider.clone())...
//! ```
//!
//! `builder_with_provider` never consults the process-wide default, so a
//! `sqlx` Postgres connection negotiating TLS cannot hit the "no default
//! installed" panic regardless of whether `install_default()` was ever
//! called anywhere in the process. **Conclusion: this crate does not call
//! `install_default()`, deliberately.** The requirement documented in the
//! root `Cargo.toml` is real but belongs to the dashboard-auth work
//! (`authkestra-op`/`authkestra-engine`'s `reqwest` clients), not to this
//! crate — see that comment block for the call site it still needs to land
//! at, once that work starts.
//!
//! # Pool sizing
//!
//! See the constants and their doc comments in `pool` for the numbers and
//! the reasoning behind each one.

// The repository modules are `pub` rather than flattened into re-exported
// free functions, unlike the three older ones below. Their names carry the
// meaning: `payment_intents::insert` says what a bare `insert` would not,
// and `charges::insert_for_intent` and `idempotency::claim` would collide
// or read as nonsense at the crate root. The row and seed *types* are
// re-exported below anyway, so a caller spells a type once and reaches a
// function through the table it belongs to.
pub mod charges;
pub mod config_reconcile;
pub mod idempotency;
// `pub` for the same reason the repository modules are, plus one of its own:
// a test that wants to prove a writer actually takes its lock has to be able
// to *hold* that lock from outside (see
// `reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released` in
// `tests/repositories.rs`), and an operator reading `pg_locks` needs the
// values to be findable from a crate doc rather than by grepping for a hex
// literal.
pub mod lock_keys;
pub mod payment_intents;
pub mod provider_requests;

mod client_assertion;
mod disabled_clients;
mod error;
mod health;
mod migrations;
mod pool;
mod signing_keys;

pub use charges::{ChargeRow, NewCharge};
pub use client_assertion::{SqlClientAssertionStore, delete_expired_client_assertion_jtis};
pub use config_reconcile::{CurrencySeed, ProviderSeed};
pub use disabled_clients::{disable_client, enable_client, is_client_disabled};
pub use error::DbError;
pub use health::check_connection;
pub use idempotency::{IdempotencyClaim, IdempotencyRecord, IdempotencyStoreOutcome};
pub use migrations::run_migrations;
pub use payment_intents::{ListPage, NewPaymentIntent, PaymentIntentRow};
pub use pool::connect;
pub use signing_keys::{
    ActivationOutcome, SigningKey, active_signing_key_kid, ensure_active_signing_key,
    publishable_signing_keys, rotate_signing_key,
};

// Re-exported so callers (both binaries' `main.rs`, `vpay-api`'s router
// state) can name the pool type without also depending on `sqlx` directly
// just to spell it.
pub use sqlx::PgPool;
