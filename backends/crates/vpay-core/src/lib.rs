//! Domain types shared by every part of vpay.
//!
//! This crate knows nothing about any payment rail, any HTTP framework, or any
//! database. If a rail-specific name (`msisdn`, `pay_token`, `subscription
//! key`) appears here, that is a defect — see `docs/adr/0002-provider-port.md`.

pub mod error;
pub mod failure;
pub mod money;
pub mod state;

pub use error::{Category, Classify, Retry, Severity};
pub use failure::FailureCode;
pub use money::{Currency, Money, MoneyError};
pub use state::{ChargeState, IntentStatus, ProviderFlow};
