//! Domain types shared by every part of vpay.
//!
//! This crate knows nothing about any payment rail, any HTTP framework, or any
//! database. If a rail-specific name (`msisdn`, `pay_token`, `subscription
//! key`) appears here, that is a defect — see `docs/adr/0002-provider-port.md`.

pub mod error;
pub mod failure;
pub mod ids;
pub mod money;
// The reconciler's half of the state machine, kept apart from `state` on
// purpose: `state::Transition` is the merchant's three verbs and must stay
// unreachable from a rail answer. See the module header.
pub mod settlement;
pub mod state;

pub use error::{Category, Classify, Retry, Severity};
pub use failure::FailureCode;
pub use money::{Currency, Money, MoneyError};
pub use settlement::{Settlement, StatusKind, contradiction, settle};
pub use state::{ChargeState, IntentStatus, ProviderFlow, Transition, next_status};
