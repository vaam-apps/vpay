//! Merchant SDK for vpay's `/v1` API.
//!
//! Implements the wire contract in `docs/flows/merchant-auth.md`:
//! `client_credentials` + `private_key_jwt` authentication (RFC 7523),
//! Stripe-shaped form-encoded resources, and outbound-webhook verification.
//!
//! **Part of the resource half has a server to talk to; part does not.**
//! `vpay-server` mounts `/v1/oauth` (token, discovery, JWKS), the `/v1`
//! bearer-token boundary, and — as of Step 2 (payment intents) and Step 5
//! (events) — [`PaymentIntentsResource`]'s `create`/`retrieve`/`list`/
//! `confirm`/`cancel` and [`EventsResource`]'s `list`/`retrieve`.
//! [`RefundsResource`] and [`BalanceResource`] have no route yet and still
//! reach a 404 there. `backends/tests/integration/tests/merchant_token_flow.rs`
//! drives this crate against the real server; see `docs/status.md` and this
//! crate's own `README.md` "Status" section for exactly what is, and is not,
//! proven by this crate's tests.
//!
//! # Quick start
//!
//! ```no_run
//! use vpay_sdk::{Client, Credentials, RequestOptions};
//! use vpay_sdk::payment_intents::{CreatePaymentIntentParams, PaymentMethodType};
//!
//! # async fn run() -> Result<(), vpay_sdk::Error> {
//! let credentials = Credentials::rsa_pem("merchant_a", "-----BEGIN PRIVATE KEY-----...")?;
//! let client = Client::builder("https://api.vpay.example")
//!     .credentials(credentials)
//!     .build()?;
//!
//! let intent = client
//!     .payment_intents()
//!     .create(
//!         CreatePaymentIntentParams {
//!             amount: 5000,
//!             currency: "xaf".to_string(),
//!             payment_method_types: vec![PaymentMethodType::MtnMomo],
//!             ..Default::default()
//!         },
//!         RequestOptions::new(),
//!     )
//!     .await?;
//! println!("{}", intent.id);
//! # Ok(())
//! # }
//! ```

// Every public item carries a doc comment, and this is what keeps that true:
// a merchant reads this crate's rustdoc as the API reference (there is no
// other one), so an undocumented `pub` item is a hole in the only
// documentation that exists. `AGENTS.md` requires it; without this attribute
// nothing checks it.
#![deny(missing_docs)]

pub mod auth;
mod client;
mod error;
mod form;
mod model;
mod resources;
mod validate;
pub mod webhooks;

pub use auth::Credentials;
pub use client::{Client, ClientBuilder, DEFAULT_AUDIENCE};
pub use error::{ConfigError, Error, WebhookError};
pub use model::{
    Balance, BalanceEntry, Event, EventData, IntentStatus, LastPaymentError, List, NextAction,
    PaymentIntent, PaymentMethodType, RedirectToUrl, Refund, RefundStatus,
};
pub use resources::{
    BalanceResource, ConfirmPaymentIntentParams, CreatePaymentIntentParams, CreateRefundParams,
    EventsResource, ListEventsParams, ListPaymentIntentsParams, PaymentIntentsResource,
    RefundsResource, RequestOptions,
};

/// Re-exports the `payment_intents` params/resource types under a
/// module-shaped path, matching how a merchant is likely to `use` them
/// alongside `client.payment_intents()` — the flat re-export list above is
/// what actually defines the public API; this module only offers a second,
/// more conventional import path onto the same items.
pub mod payment_intents {
    pub use crate::model::PaymentMethodType;
    pub use crate::resources::{
        ConfirmPaymentIntentParams, CreatePaymentIntentParams, ListPaymentIntentsParams,
        PaymentIntentsResource,
    };
}

/// See [`payment_intents`].
pub mod refunds {
    pub use crate::resources::{CreateRefundParams, RefundsResource};
}

/// See [`payment_intents`].
pub mod events {
    pub use crate::resources::{EventsResource, ListEventsParams};
}

/// Compiles the `rust` blocks in this crate's `README.md` as doctests.
///
/// Not decoration: the README carries the usage example a merchant copies,
/// and nothing else in the build ever compiled it — the example survived a
/// change to `ConfirmPaymentIntentParams` that made it uncompilable, and no
/// gate noticed. `#[cfg(doctest)]` keeps the item out of every real build.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
