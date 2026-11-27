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
pub mod privacy;
pub mod routing;
pub mod settlement;
pub mod state;

pub use error::{Category, Classify, Retry, Severity};
pub use failure::FailureCode;
pub use money::{Currency, Money, MoneyError};
pub use settlement::{Settlement, StatusKind, contradiction, settle};
pub use state::{
    ChargeState, IntentStatus, InvoiceStatus, ProviderFlow, RefundStatus, Transition, next_status,
};

/// The reserved `charges.provider_account_id` used in RFC-0009 Phase A: a
/// deployment holds one account per rail at rail level, and every charge
/// records `default` so Phase B never has to guess which account an old charge
/// used.
pub const DEFAULT_PROVIDER_ACCOUNT_ID: &str = "default";
