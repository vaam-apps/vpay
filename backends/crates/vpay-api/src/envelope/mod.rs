//! The signed transport, server half, for `/v1` (VP-3).
//!
//! Every `/v1` request is a COSE_Sign1 message from a merchant's registered
//! Ed25519 key, and every answer to one is a COSE_Sign1 message from this
//! process's key. There is no bearer token on `/v1`: an `Authorization` header
//! is not read, because the layer that opens the request is in front of
//! everything that could read one.
//!
//! ```text
//!   client ──COSE_Sign1(form body)──▶ track_http_metrics
//!                                       └▶ EnvelopeLayer          opens, verifies, records the cti
//!                                            └▶ RequestBodyLimit  bounds the OPENED payload
//!                                                 └▶ merchant_scope   signer → tenant + scopes
//!                                                      └▶ the handler  (a plain form request)
//!   client ◀─COSE_Sign1(JSON body)── …sealed on the way back out, errors included
//! ```
//!
//! * [`keys`] — the Ed25519 seeds this process seals with (`current`, and
//!   `next` for rotation).
//! * [`nonces`] — the replay cache, shared by every replica through Postgres.
//! * [`binding`] — which `/v1` operation a request addresses, over
//!   `V1_ROUTES`, and the form-in / JSON-out payload types.
//! * [`scope`] — the signer's merchant, its tenant and its scopes.
//!
//! The layer proves who holds the key; [`scope`] says what that is worth. That
//! split is cratestack's (ADR 0006) and is kept: the resolver
//! ([`MerchantKeyResolver`]) answers a signature's `kid`, never an operation.
//!
//! # Placement, and what the order buys
//!
//! * The layer is applied to the `/v1` router **before** it is nested
//!   (`Router::layer`, so `MatchedPath` is set and the resolver can bind the
//!   operation) with `.mount_prefix("/v1")`. Wrapped around the whole app it
//!   would see no matched route, let plain traffic through unprotected and
//!   refuse every COSE body with a `415`.
//! * `track_http_metrics` stays **outside** it: it reads the route pattern and
//!   the response status, never a body or a header, and a sealed response keeps
//!   its real HTTP status, so the counters still say `401`/`404`/`409` rather
//!   than a wall of `200`s.
//! * The body limit is **inside** it. The layer buffers at most
//!   [`ENVELOPE_BODY_LIMIT_BYTES`] of COSE message (the payload's bound plus the
//!   header, the signature and the base framing), and the limit then bounds
//!   what the handler would read: the opened payload.
//! * Nothing reads `Authorization`, the body or `Content-Type` ahead of the
//!   layer. vaam-apps found two middlewares that did (a PII-read audit keyed on
//!   `Authorization`, an edge limiter that decoded `/rpc/batch` frames) and had
//!   to move them inside the envelope; `/v1` has neither, and
//!   `a_bearer_token_buys_nothing_on_v1` pins that for the one place
//!   one could be added without noticing.
//!
//! # Fail closed
//!
//! A mode of `Required`, no optional or compat envelope: an unsigned request
//! to a `/v1` operation is the layer's coarse `401`, whatever it carries. A
//! missing server key or merchant key registry is not a degraded mode either;
//! the process refuses to boot (`vpay-server`, exit `78`).

pub mod binding;
pub mod keys;
pub mod nonces;
pub(crate) mod scope;

use std::sync::Arc;

use cratestack_axum::envelope_layer::{EnvelopeLayer, EnvelopeMode};
use cratestack_cose::{CoseEnvelope, CoseMode};
use vpay_config::{Config, ConfigError};
use vpay_core::{Category, Classify};
use vpay_db::Repositories;
use vpay_wire::{V1_CONTRACTS, V1_MOUNT};

pub use binding::V1Binding;
pub use keys::{EnvelopeKeyError, EnvelopeKeys};
pub use nonces::PgNonceStore;

use crate::merchant_keys::{MerchantKeyRegistry, MerchantKeyResolver};

/// How much more than the `/v1` payload bound the layer will buffer of the
/// COSE message that wraps it: protected header, claims, the 64-byte signature
/// and the CBOR framing. Sixteen KiB is cratestack's own default slack
/// (`DEFAULT_MAX_BODY_BYTES`), and a message is nowhere near it.
const ENVELOPE_OVERHEAD_BYTES: usize = 16 * 1024;

/// The most COSE-message bytes the layer reads for one request:
/// [`crate::V1_BODY_LIMIT_BYTES`] of payload plus the envelope's own framing.
///
/// The payload's exact bound is enforced separately, inside the layer, on the
/// opened bytes — this one only keeps an anonymous caller from making the
/// process buffer more than a bounded message before the signature is checked.
pub const ENVELOPE_BODY_LIMIT_BYTES: usize = crate::V1_BODY_LIMIT_BYTES + ENVELOPE_OVERHEAD_BYTES;

/// The audience every `/v1` request binds and this process answers to: the
/// deployment's public base URL, without a trailing slash.
///
/// A configured logical name and never the `Host` header (ADR 0006 §4), so a
/// request signed for one deployment cannot be replayed at another that shares
/// a merchant key. It is `issuer_for`'s base, for the same reason that one is
/// derived in one place: a merchant's client and this server compare the string
/// byte for byte and the only symptom of a mismatch is the coarse `401`. A
/// merchant that reaches vpay at an internal address (`http://vpay:8080`) sets
/// the SDK's `audience` to this value, as vaam-apps does.
///
/// ```
/// use vpay_api::envelope::audience_for;
///
/// assert_eq!(audience_for("https://api.example.test/"), "https://api.example.test");
/// assert_eq!(audience_for("http://localhost:8080"), "http://localhost:8080");
/// ```
#[must_use]
pub fn audience_for(public_base_url: &str) -> String {
    public_base_url.trim_end_matches('/').to_owned()
}

/// Why the `/v1` envelope could not be assembled at boot.
#[derive(Debug, thiserror::Error)]
pub enum V1EnvelopeError {
    /// The merchant key registry could not be read out of the configuration.
    /// Unreachable for a `Config` that came from a file (`validate_all` ran
    /// the same code); kept typed so a hand-built one is refused too.
    #[error("the merchant key registry could not be built: {0}")]
    Registry(#[from] ConfigError),

    /// cratestack refused the layer's configuration: an empty audience, or a
    /// payload-type or contract setup it cannot serve.
    #[error("the /v1 envelope layer was refused: {0}")]
    Layer(String),
}

impl Classify for V1EnvelopeError {
    /// A deployment that must be fixed, exit `78`.
    fn category(&self) -> Category {
        Category::Configuration
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Registry(_) => "merchant_keys_unusable",
            Self::Layer(_) => "envelope_layer_refused",
        }
    }
}

/// Everything `router()` needs to put the signed transport in front of `/v1`:
/// the built layer, and the merchant resolver [`scope`] asks once a request has
/// verified.
///
/// Built once at boot and cloned into [`crate::RouterDeps`]; cheap to clone
/// (two `Arc`s).
#[derive(Clone)]
pub struct V1Envelope {
    layer: EnvelopeLayer,
    merchants: Arc<MerchantKeyResolver>,
}

impl std::fmt::Debug for V1Envelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V1Envelope")
            .field("layer", &self.layer)
            .field("merchants", &self.merchants)
            .finish()
    }
}

impl V1Envelope {
    /// Builds the layer for `config`'s merchants, sealing with `keys`, with
    /// the replay cache in `repositories`.
    ///
    /// # Errors
    ///
    /// [`V1EnvelopeError`], both `78` at boot.
    pub fn new(
        config: &Config,
        keys: EnvelopeKeys,
        repositories: Arc<dyn Repositories>,
    ) -> Result<Self, V1EnvelopeError> {
        let registry = Arc::new(MerchantKeyRegistry::from_config(config)?);
        let merchants = Arc::new(MerchantKeyResolver::new(registry, repositories.clone()));
        Self::with_resolver(config, keys, repositories, merchants)
    }

    /// [`Self::new`] over a resolver the caller already built (a test that
    /// shares one with its own assertions).
    ///
    /// # Errors
    ///
    /// [`V1EnvelopeError::Layer`].
    pub fn with_resolver(
        config: &Config,
        keys: EnvelopeKeys,
        repositories: Arc<dyn Repositories>,
        merchants: Arc<MerchantKeyResolver>,
    ) -> Result<Self, V1EnvelopeError> {
        let layer_error =
            |error: cratestack_core::CratestackError| V1EnvelopeError::Layer(error.to_string());
        let envelope = CoseEnvelope::server(
            CoseMode::Sign1,
            Arc::new(keys.into_signer()),
            merchants.clone(),
            Arc::new(PgNonceStore::new(repositories)),
        )
        .build()
        .map_err(layer_error)?;
        let layer = EnvelopeLayer::builder(
            envelope,
            audience_for(&config.deployment.public_base_url),
            *V1_CONTRACTS,
        )
        .binding_resolver(V1Binding)
        .mount_prefix(V1_MOUNT)
        .policy(EnvelopeMode::Required)
        .payload_media_types(
            binding::REQUEST_TYPES.iter().copied(),
            binding::RESPONSE_TYPES.iter().copied(),
        )
        .max_body_bytes(ENVELOPE_BODY_LIMIT_BYTES)
        .build()
        .map_err(layer_error)?;
        Ok(Self { layer, merchants })
    }

    pub(crate) fn layer(&self) -> EnvelopeLayer {
        self.layer.clone()
    }

    pub(crate) fn merchants(&self) -> Arc<MerchantKeyResolver> {
        self.merchants.clone()
    }
}
