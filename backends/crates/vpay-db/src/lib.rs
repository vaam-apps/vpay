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
/// The dashboard client's authorization-code store (ADR-0017 decision 3).
pub mod authorization_codes;
pub mod charges;
// The hosted/embedded checkout object (Step 9). Its own module rather than
// functions on `payment_intents`, because a session is not a property of an
// intent: it is one *attempt* to drive one through vpay's own page, it
// carries two payer credentials of its own, and the one write that belongs
// to neither table alone — the settlement flip — is `pub(crate)` here and
// reachable only from `settlement`.
pub mod checkout_sessions;
pub mod config_reconcile;
// The merchant-owned payer record (S4a). Its own module for
// `checkout_sessions`' reason — one table family, one file — and because the
// privacy rules that apply to a table whose whole content is personal data
// apply only here: the hard delete, the redacting `Debug`, and the
// twelve-month retention sweep all belong beside the code they constrain.
pub mod customers;
pub mod events;
// The merchant's bill to a payer, and the lines it is made of (S4b). One
// module for both tables rather than two, because the pair is one aggregate:
// a line has no meaning without its invoice, every write to `invoice_items`
// is guarded on its parent's status, and the invoice's totals are rewritten
// by the same transaction that changes a line. Splitting them would put the
// guard and the thing it guards in different files.
pub mod idempotency;
pub mod invoices;
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
// The merchant's authoritative read of a refund (issue #45). Its own module
// rather than functions on `payment_intents`, although a refund has no
// `merchant_id` of its own and is scoped by joining one: the join is the
// reason it needs a home where that argument can be written down once,
// instead of a `payment_intents` function whose name would not say it reads
// another table.
pub mod refunds;
// The settlement transaction is its own module rather than a function on
// `charges` or on `payment_intents`, because it is the one write that
// belongs to *neither* table on its own: it moves both and emits the event
// that tells a merchant about them. A home inside either table's module
// would have made "settle the charge" reachable without the rest.
/// The durable fixed-window counters every replica's rate limiting spends
/// from (issue #79 item 2, migration 0038). Its own module rather than a
/// function on `staff`, because the table is not about staff: its commonest
/// key is an address with no account, and the second action counted on it is
/// a password change made by somebody already signed in.
pub mod rate_limits;
pub mod settlement;
/// Staff sign-in: the `staff_members` table, its two credentials and the replay
/// guard (ADR-0017).
pub mod staff;
/// Server-side staff sessions: the two bounds, and the revocation that makes
/// signing out mean something (ADR-0017 decision 2).
pub mod staff_sessions;
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
// The CrateStack layer. Both modules are private and neither exports a
// name: `schema` holds the generated `cratestack_schema` module, and
// `persistence` is the only place in this crate a `CratestackError` is read.
// `PersistenceError` itself is `pub` (a `DbError` variant carries it, so a
// caller can match on it), and it is re-exported below from `persistence`
// rather than the module being made `pub`.
mod health;
mod migrations;
mod persistence;
mod pool;
pub use pool::MAX_CONNECTIONS;
mod schema;
mod signing_keys;
// The audit `sqlx::AssertSqlSafe` demands, as a test rather than a comment.
// Test-only: it reads this crate's own sources through `CARGO_MANIFEST_DIR`.
#[cfg(test)]
mod sql_audit;

pub use authorization_codes::{AuthorizationCodeRow, AuthorizationCodes, NewAuthorizationCode};
pub use charges::{ChargeAsOf, ChargeRow, Charges, NewCharge};
pub use checkout_sessions::{
    CheckoutSessionRow, CheckoutSessions, NewCheckoutSession, SessionListPage,
};
pub use client_assertion::{ClientAssertions, client_assertion_store};
pub use config_reconcile::{ConfigReconcile, CurrencySeed, ProviderSeed};
pub use customers::{CustomerListPage, CustomerPatch, CustomerRow, Customers, NewCustomer};
pub use disabled_clients::DisabledClients;
pub use error::DbError;
pub use events::{EventRow, Events, NewEvent};
pub use health::Health;
pub use idempotency::{Idempotency, IdempotencyClaim, IdempotencyRecord, IdempotencyStoreOutcome};
pub use invoices::{
    InvoiceItemPatch, InvoiceItemRow, InvoiceListPage, InvoicePatch, InvoiceRow, Invoices,
    NewInvoice, NewInvoiceItem,
};
pub use jobs::{JobRow, Jobs};
pub use migrations::Migrations;
pub use payment_intents::{
    IntentFilter, ListPage, NewPaymentIntent, PaymentIntentRow, PaymentIntents,
};
// The leaf `DbError::Persistence` carries. `pub` because a caller matching
// on `DbError` can reach it; the module it lives in stays private, and
// `classify_cratestack`/`system_context` stay `pub(crate)` — nothing outside
// this crate has a `CratestackError` to classify or a reason to build a
// system context.
pub use persistence::PersistenceError;
pub use pool::{connect, connect_lazy};
pub use provider_requests::ProviderRequests;
pub use rate_limits::RateLimits;
pub use refunds::{RefundRow, Refunds};
pub use repository::{
    PendingTransaction, Repositories, TransactionSource, TxFuture, TxOutcome, TxRepositories,
    UnitOfWork,
};
pub use settlement::{AttemptRow, InvoicePaidEvent, Settlement};
pub use signing_keys::{ActivationOutcome, SigningKey, SigningKeys};
pub use staff::{NewStaff, Staff, StaffRow, StaffStatus};
pub use staff_sessions::{
    ABSOLUTE_LIFETIME, IDLE_TIMEOUT, NewSession, SessionRow, SessionState, StaffSessions,
};
pub use webhook_deliveries::{DeliveryRow, WebhookDeliveries};
