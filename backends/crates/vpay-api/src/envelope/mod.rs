//! The signed transport, server half, for `/v1` (VP-3) and for the management
//! surface, `/dash/v1` and `/svc/v1` (VP-6a).
//!
//! Every `/v1` request is a COSE_Sign1 message from a merchant's registered
//! Ed25519 key, and every answer to one is a COSE_Sign1 message from this
//! process's key. **On a deployment that permits `keypair` alone** (`auth.methods`,
//! ADR-0037) there is no bearer token on `/v1`: an `Authorization` header is
//! not read, because the layer that opens the request is in front of
//! everything that could read one. A deployment that also permits
//! `machine_tokens` runs the layer in `Optional` mode and lets [`scope`] read a
//! bearer token from a request that was not signed ([`SurfaceMode`]); one that
//! permits it alone runs the layer `Off`.
//!
//! The management surface is the same mechanism with different keys, three
//! layers in all, each its own resolver and its own contract table:
//!
//! | Surface | Whose key | Where it is registered | Resolver |
//! |---|---|---|---|
//! | `/v1` | a merchant's backend (Ed25519) | `merchant_clients[].cose_keys` | [`MerchantKeyResolver`] |
//! | `/dash/v1`, `/dash/v1/$procs/*` | a person's console (P-256) | `console_keys`, in the database | [`console::ConsoleKeyResolver`] |
//! | `/svc/v1` | a service, vaam (Ed25519) | `service_clients[].cose_keys` | [`ServiceKeyResolver`] |
//!
//! A key of one kind is no key at another: each resolver reads its own registry
//! and nothing else, so a merchant's key at `/svc/v1`, or a console's at `/v1`,
//! is the coarse unsigned `401` an unknown key gets. All three seal their answers
//! with the one response key (`VPAY_ENVELOPE_SIGNING_KEY_FILE`).
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
//! Under `keypair` alone the mode is `Required`: an unsigned request to a `/v1`
//! operation is the layer's coarse `401`, whatever it carries. Loosening it is
//! the operator's explicit choice in `auth.methods`, never a fallback the code
//! takes by itself, and `Optional` still refuses a signed request that fails to
//! verify rather than reading it as unsigned. A missing server key or merchant
//! key registry is not a degraded mode either; the process refuses to boot
//! (`vpay-server`, exit `78`).

pub mod binding;
pub mod console;
pub mod keys;
pub mod nonces;
pub(crate) mod scope;
pub(crate) mod service;

use std::sync::Arc;

use cratestack_axum::envelope_layer::{EnvelopeLayer, EnvelopeMode, PolicyRequest};
use cratestack_core::AcceptedContracts;
use cratestack_cose::{CoseEnvelope, CoseMode, CoseVerifierResolver};
use vpay_config::{AuthMethod, Config, ConfigError};
use vpay_core::{Category, Classify};
use vpay_db::Repositories;
use vpay_wire::{DASH_CONTRACTS, DASH_MOUNT, SVC_CONTRACTS, SVC_MOUNT, V1_CONTRACTS, V1_MOUNT};

pub use binding::TableBinding;
pub use console::ConsoleKeyResolver;
pub use keys::{EnvelopeKeyError, EnvelopeKeys};
pub use nonces::PgNonceStore;

use crate::merchant_keys::{
    MerchantKeyRegistry, MerchantKeyResolver, ServiceKeyRegistry, ServiceKeyResolver,
};
use crate::resource_auth::JwtValidator;
use crate::v1::ResourceConfig;

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
/// The management surface uses the same bound.
pub const ENVELOPE_BODY_LIMIT_BYTES: usize = crate::V1_BODY_LIMIT_BYTES + ENVELOPE_OVERHEAD_BYTES;

/// The audience every signed request binds and this process answers to: the
/// deployment's public base URL, without a trailing slash.
///
/// A configured logical name and never the `Host` header (ADR 0006 §4), so a
/// request signed for one deployment cannot be replayed at another that shares
/// a key. It is `issuer_for`'s base, for the same reason that one is derived in
/// one place: a client and this server compare the string byte for byte and the
/// only symptom of a mismatch is the coarse `401`. A merchant that reaches vpay
/// at an internal address (`http://vpay:8080`) sets the SDK's `audience` to this
/// value, as vaam-apps does.
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

/// Why an envelope could not be assembled at boot.
#[derive(Debug, thiserror::Error)]
pub enum V1EnvelopeError {
    /// A key registry could not be read out of the configuration.
    /// Unreachable for a `Config` that came from a file (`validate_all` ran
    /// the same code); kept typed so a hand-built one is refused too.
    #[error("a key registry could not be built: {0}")]
    Registry(#[from] ConfigError),

    /// cratestack refused the layer's configuration: an empty audience, or a
    /// payload-type or contract setup it cannot serve.
    #[error("an envelope layer was refused: {0}")]
    Layer(String),
}

impl Classify for V1EnvelopeError {
    /// A deployment that must be fixed, exit `78`.
    fn category(&self) -> Category {
        Category::Configuration
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Registry(_) => "key_registry_unusable",
            Self::Layer(_) => "envelope_layer_refused",
        }
    }
}

/// The COSE server envelope every surface shares in shape: sealing with this
/// process's response key, verifying under `resolver`, replay-guarded by the
/// Postgres nonce store.
fn cose_envelope(
    keys: &EnvelopeKeys,
    resolver: Arc<dyn CoseVerifierResolver>,
    repositories: &Arc<dyn Repositories>,
) -> Result<CoseEnvelope, V1EnvelopeError> {
    cose_envelope_over(
        keys,
        resolver,
        Arc::new(PgNonceStore::new(repositories.clone())),
    )
}

/// [`cose_envelope`] over a replay cache the caller supplies.
fn cose_envelope_over(
    keys: &EnvelopeKeys,
    resolver: Arc<dyn CoseVerifierResolver>,
    nonces: Arc<dyn cratestack_core::NonceStore>,
) -> Result<CoseEnvelope, V1EnvelopeError> {
    CoseEnvelope::server(CoseMode::Sign1, keys.signer(), resolver, nonces)
        .build()
        .map_err(layer_error)
}

fn layer_error(error: cratestack_core::CratestackError) -> V1EnvelopeError {
    V1EnvelopeError::Layer(error.to_string())
}

/// One hand-written surface's layer in `mode`, bound by `binding` over
/// `contracts`, mounted at `mount`, sealing in the types the binding declares.
///
/// `mode` is `Required` for every surface but `/v1`, whose mode is the
/// deployment's choice ([`SurfaceMode`]).
fn surface_layer(
    config: &Config,
    envelope: CoseEnvelope,
    contracts: AcceptedContracts,
    binding: TableBinding,
    mount: &str,
    mode: SurfaceMode,
) -> Result<EnvelopeLayer, V1EnvelopeError> {
    let (request_types, response_types) = binding.payload_types();
    let builder = EnvelopeLayer::builder(
        envelope,
        audience_for(&config.deployment.public_base_url),
        contracts,
    )
    .binding_resolver(binding)
    .mount_prefix(mount);
    let builder = match mode {
        SurfaceMode::Required => builder.policy(EnvelopeMode::Required),
        // A closure rather than the bare mode, and `unresolved_mode`
        // pinned back to `Required`: a bare `Optional` answers
        // `unresolved_mode` with itself, so a route the resolver could not
        // bind would pass through unsigned instead of failing closed with the
        // `500` (cratestack decision S2). Only the ops' own mode is loosened.
        SurfaceMode::Optional => builder
            .policy(|_request: &PolicyRequest<'_>| EnvelopeMode::Optional)
            .unresolved_mode(EnvelopeMode::Required),
        SurfaceMode::Off => builder.policy(EnvelopeMode::Off),
    };
    builder
        .payload_media_types(
            request_types.iter().copied(),
            response_types.iter().copied(),
        )
        .max_body_bytes(ENVELOPE_BODY_LIMIT_BYTES)
        .build()
        .map_err(layer_error)
}

/// The envelope mode `/v1` runs in, decided by `auth.methods` (ADR-0037).
///
/// | `keypair` | `machine_tokens` | Mode | A request with no signature |
/// |---|---|---|---|
/// | yes | no | `Required` | the layer's coarse unsigned `401` |
/// | yes | yes | `Optional` | runs; the scope step wants a bearer token |
/// | no | yes | `Off` | runs; a COSE body is refused (`415`) |
///
/// The fourth row, neither, is refused at boot (`ConfigError::NoWayIn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMode {
    /// Every request is signed (VP-3).
    Required,
    /// A signed request is opened and sealed; an unsigned one is let through
    /// to the scope step, which accepts a bearer token.
    Optional,
    /// The envelope is off: a legacy-only deployment, which seals nothing.
    Off,
}

impl SurfaceMode {
    /// The mode `config`'s `auth.methods` selects.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoWayIn`] when neither `keypair` nor `machine_tokens`
    /// is permitted: unreachable for a `Config` that came from a file.
    pub fn from_config(config: &Config) -> Result<Self, ConfigError> {
        let methods = config.auth_methods()?;
        match (
            methods.permits(AuthMethod::Keypair),
            methods.permits(AuthMethod::MachineTokens),
        ) {
            (true, false) => Ok(Self::Required),
            (true, true) => Ok(Self::Optional),
            (false, true) => Ok(Self::Off),
            (false, false) => Err(ConfigError::NoWayIn),
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
    mode: SurfaceMode,
    /// Whether `auth.methods` permits `machine_tokens`, and the validator the
    /// scope step checks a bearer token with. `None` with the method permitted
    /// is a router assembled without one: every unsigned request is then a
    /// `500`, never an open door.
    machine_tokens: bool,
    validator: Option<JwtValidator>,
}

impl std::fmt::Debug for V1Envelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V1Envelope")
            .field("layer", &self.layer)
            .field("merchants", &self.merchants)
            .field("mode", &self.mode)
            .field("machine_tokens", &self.machine_tokens)
            .finish_non_exhaustive()
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
        keys: &EnvelopeKeys,
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
        keys: &EnvelopeKeys,
        repositories: Arc<dyn Repositories>,
        merchants: Arc<MerchantKeyResolver>,
    ) -> Result<Self, V1EnvelopeError> {
        let envelope = cose_envelope(keys, merchants.clone(), &repositories)?;
        Self::assemble(config, envelope, merchants)
    }

    /// [`Self::with_resolver`] with an in-memory replay cache, for the unit
    /// tests that open a real sealed request without a database. The cache is
    /// the only difference: the layer, the mode and the scope step are the ones
    /// that ship.
    ///
    /// # Errors
    ///
    /// [`V1EnvelopeError::Layer`].
    #[cfg(test)]
    pub(crate) fn with_replay_cache(
        config: &Config,
        keys: &EnvelopeKeys,
        merchants: Arc<MerchantKeyResolver>,
        nonces: Arc<dyn cratestack_core::NonceStore>,
    ) -> Result<Self, V1EnvelopeError> {
        let envelope = cose_envelope_over(keys, merchants.clone(), nonces)?;
        Self::assemble(config, envelope, merchants)
    }

    fn assemble(
        config: &Config,
        envelope: CoseEnvelope,
        merchants: Arc<MerchantKeyResolver>,
    ) -> Result<Self, V1EnvelopeError> {
        let mode = SurfaceMode::from_config(config)?;
        let layer = surface_layer(
            config,
            envelope,
            *V1_CONTRACTS,
            TableBinding::v1(),
            V1_MOUNT,
            mode,
        )?;
        Ok(Self {
            layer,
            merchants,
            mode,
            machine_tokens: config.auth_methods()?.permits(AuthMethod::MachineTokens),
            validator: None,
        })
    }

    /// Hands the scope step the validator it checks a bearer token with.
    ///
    /// Separate from [`Self::new`] because the validator fetches this
    /// process's own JWKS over loopback and so needs the bound address, which
    /// exists only after the listener does. Meaningful only when
    /// `auth.methods` permits `machine_tokens`; harmless otherwise, since the
    /// scope step never consults it.
    #[must_use]
    pub fn with_machine_tokens(mut self, validator: JwtValidator) -> Self {
        self.validator = Some(validator);
        self
    }

    /// The mode `/v1` runs in, for the boot log.
    #[must_use]
    pub fn mode(&self) -> SurfaceMode {
        self.mode
    }

    pub(crate) fn layer(&self) -> EnvelopeLayer {
        self.layer.clone()
    }

    /// What the scope step needs, over `resource_config` (which maps a
    /// token's `client_id` to a tenant).
    pub(crate) fn scope_state(&self, resource_config: Arc<ResourceConfig>) -> scope::ScopeState {
        scope::ScopeState {
            merchants: self.merchants.clone(),
            machine_tokens: self.machine_tokens,
            validator: self.validator.clone(),
            resource_config,
        }
    }
}

/// Everything `router()` needs to put the signed transport in front of the
/// management surface (VP-6a): three layers, one per thing mounted, and the
/// service resolver `/svc/v1`'s scope step asks.
///
/// The console's two layers share a key resolver and a response key and differ in
/// what they bind: the hand-written reads ([`DASH_CONTRACTS`], [`TableBinding::dash`])
/// and CrateStack's generated procedure transport (`ACCEPTED_CONTRACTS` and
/// `ROUTE_TRANSPORTS`, from `schemas/vpay.cstack`, built by
/// [`Repositories::dashboard_envelope_layer`] because the generated module is
/// private to `vpay-db`).
#[derive(Clone)]
pub struct ManagementEnvelope {
    dash: EnvelopeLayer,
    procedures: EnvelopeLayer,
    svc: EnvelopeLayer,
    services: Arc<ServiceKeyResolver>,
}

impl std::fmt::Debug for ManagementEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagementEnvelope")
            .field("services", &self.services)
            .finish_non_exhaustive()
    }
}

impl ManagementEnvelope {
    /// Builds the three layers: the console resolver reads `console_keys` through
    /// `repositories`, the service resolver reads `config.service_clients`.
    ///
    /// # Errors
    ///
    /// [`V1EnvelopeError`], both `78` at boot.
    pub fn new(
        config: &Config,
        keys: &EnvelopeKeys,
        repositories: Arc<dyn Repositories>,
    ) -> Result<Self, V1EnvelopeError> {
        let console: Arc<dyn CoseVerifierResolver> =
            Arc::new(ConsoleKeyResolver::new(repositories.clone()));
        let services = Arc::new(ServiceKeyResolver::new(
            Arc::new(ServiceKeyRegistry::from_config(config)?),
            repositories.clone(),
        ));

        let dash = surface_layer(
            config,
            cose_envelope(keys, console.clone(), &repositories)?,
            *DASH_CONTRACTS,
            TableBinding::dash(),
            DASH_MOUNT,
            SurfaceMode::Required,
        )?;
        let procedures = repositories
            .dashboard_envelope_layer(
                Arc::new(cose_envelope(keys, console, &repositories)?),
                &audience_for(&config.deployment.public_base_url),
                DASH_MOUNT,
            )
            .map_err(|error| V1EnvelopeError::Layer(error.to_string()))?;
        let svc = surface_layer(
            config,
            cose_envelope(keys, services.clone(), &repositories)?,
            *SVC_CONTRACTS,
            TableBinding::svc(),
            SVC_MOUNT,
            SurfaceMode::Required,
        )?;
        Ok(Self {
            dash,
            procedures,
            svc,
            services,
        })
    }

    pub(crate) fn dash_layer(&self) -> EnvelopeLayer {
        self.dash.clone()
    }

    pub(crate) fn procedures_layer(&self) -> EnvelopeLayer {
        self.procedures.clone()
    }

    pub(crate) fn svc_layer(&self) -> EnvelopeLayer {
        self.svc.clone()
    }

    pub(crate) fn services(&self) -> Arc<ServiceKeyResolver> {
        self.services.clone()
    }
}
