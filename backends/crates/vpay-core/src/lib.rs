//! Domain types shared by every part of vpay.
//!
//! This crate knows nothing about any payment rail, any HTTP framework, or
//! any database. If a rail-specific name (`msisdn`, `pay_token`,
//! `subscription key`) appears here, that is a defect — see
//! `docs/adr/0002-provider-port.md`.
//!
//! [docs/reference/vpay-core.md](../../../../docs/reference/vpay-core.md) is the
//! narrative half of this crate's documentation: why each of these shapes is
//! the way it is. Each module's own docs say what it is and link into it.

pub mod error;
pub mod failure;
pub mod ids;
pub mod metrics;
pub mod money;
pub mod settlement;
pub mod state;

pub use error::{Category, Classify, Retry, Severity};
pub use failure::FailureCode;
pub use money::{Currency, Money, MoneyError};
pub use settlement::{Settlement, StatusKind, contradiction, settle};
pub use state::{ChargeState, IntentStatus, ProviderFlow, Transition, next_status};
