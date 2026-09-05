//! Database connectivity for vpay: the repository traits every consumer
//! reaches Postgres through, a typed connection pool, embedded schema
//! migrations and a cheap liveness check.
//!
//! `Repositories` is the umbrella trait, `PgRepositories` its one
//! implementation, and `UnitOfWork::transaction` the only way to write two
//! statements atomically. Nothing here takes or returns a `PgPool`.
//!
//! Why the seam has that shape — trait objects over generics, a closure over a
//! transaction handle, and why this crate installs no rustls `CryptoProvider`
//! — is in `docs/reference/vpay-db.md`. Pool sizing is in `pool`'s own
//! constants.

// The table-family modules stay `pub` for the row/seed types and each
// family's own trait, not for free functions. Why, in full: [docs/reference/vpay-db.md
// § what stays pub, and why](../../../../docs/reference/vpay-db.md#what-stays-pub-and-why).
pub mod charges;
// The hosted/embedded checkout object (Step 9). Its own module rather than
// functions on `payment_intents`, because a session is not a property of an
// intent: it is one *attempt* to drive one through vpay's own page, it
// carries two payer credentials of its own, and the one write that belongs
// to neither table alone — the settlement flip — is `pub(crate)` here and
// reachable only from `settlement`.
pub mod checkout_sessions;
pub mod config_reconcile;
pub mod events;
pub mod idempotency;
pub mod jobs;
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
// The settlement transaction is its own module rather than a function on
// `charges` or on `payment_intents`, because it is the one write that
// belongs to *neither* table on its own: it moves both and emits the event
// that tells a merchant about them. A home inside either table's module
// would have made "settle the charge" reachable without the rest.
pub mod settlement;
// The delivery side of the outbox. Its own module rather than functions on
// `events`, because the fan-out transaction spans both tables and the write
// that closes it (`mark_fanned_out_in_tx`) must not be reachable without the
// inserts it commits beside — see that module's own comment.
pub mod webhook_deliveries;

// One trait per table family, `PgRepositories` behind them, and the
// closure-shaped transaction API. Everything a consumer of this crate names
// is re-exported below; nothing here takes a `PgPool`.
mod repository;

mod client_assertion;
mod disabled_clients;
mod error;
mod health;
mod migrations;
mod pool;
mod signing_keys;

pub use charges::{ChargeAsOf, ChargeRow, Charges, NewCharge};
pub use checkout_sessions::{
    CheckoutSessionRow, CheckoutSessions, NewCheckoutSession, SessionListPage,
};
pub use client_assertion::{ClientAssertions, client_assertion_store};
pub use config_reconcile::{ConfigReconcile, CurrencySeed, ProviderSeed};
pub use disabled_clients::DisabledClients;
pub use error::DbError;
pub use events::{EventRow, Events, NewEvent};
pub use health::Health;
pub use idempotency::{Idempotency, IdempotencyClaim, IdempotencyRecord, IdempotencyStoreOutcome};
pub use jobs::{JobRow, Jobs};
pub use migrations::Migrations;
pub use payment_intents::{ListPage, NewPaymentIntent, PaymentIntentRow, PaymentIntents};
pub use pool::{connect, connect_lazy};
pub use provider_requests::ProviderRequests;
pub use repository::{
    PendingTransaction, Repositories, TransactionSource, TxFuture, TxOutcome, TxRepositories,
    UnitOfWork,
};
pub use settlement::{AttemptRow, Settlement};
pub use signing_keys::{ActivationOutcome, SigningKey, SigningKeys};
pub use webhook_deliveries::{DeliveryRow, WebhookDeliveries};
