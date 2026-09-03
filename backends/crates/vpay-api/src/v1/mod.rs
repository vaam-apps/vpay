//! The merchant `/v1` resource surface — everything behind the bearer-token
//! boundary [`crate::router`] mounts.
//!
//! Three things live here rather than in a handler file, because all three
//! are properties of the *surface* and not of any one resource:
//!
//! 1. [`V1_ROUTES`], the single list of what is mounted, which
//!    `routes` (private, below) folds into the actual [`Router`]. The
//!    table is not
//!    documentation of the router — it *is* the router's source, so a route
//!    cannot exist without appearing in it, and a test that walks it (e.g.
//!    `every_registered_v1_path_answers_401_without_a_token` in
//!    `backends/tests/integration/tests/payment_intents.rs`) is walking the
//!    real surface rather than a hand-maintained copy of it.
//! 2. [`ResourceConfig`], the slice of the YAML deployment configuration a
//!    request path needs: which tenant a token's `client_id` acts for,
//!    which currencies and rails this deployment admits, and the per-rail
//!    material an adapter is handed. Resolved once at boot and shared by
//!    `Arc`, because re-reading YAML per request would make a config file
//!    edit take effect at an unpredictable moment.
//! 3. [`MerchantScope`], the resolved tenant the authentication middleware
//!    puts on every `/v1` request (D3) and every handler filters by.
//!
//! # Why the tenant is resolved before a handler runs
//!
//! `merchant_id` is the whole tenancy boundary: there is no `merchants`
//! table and therefore no foreign key that would catch a query missing its
//! filter (`backends/migrations/0003_create-payment-intents.sql`). Resolving
//! it in one place — the middleware — and handing handlers a
//! [`MerchantScope`] they cannot construct themselves means a handler that
//! forgot to scope a query does not compile, rather than reading another
//! merchant's rows.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use axum::Router;
use axum::extract::FromRequestParts;
use axum::http::Method;
use axum::http::request::Parts;
use axum::routing::{MethodRouter, get, post};
use vpay_config::Config;
use vpay_provider::ProviderConfig;

use crate::error::ApiError;

/// Boot step 4's derivation — the seeds both binaries hand
/// `vpay_db::config_reconcile::reconcile`. Not a request path, and here
/// rather than in either `main.rs` because both binaries need the identical
/// answer; see the module's own doc comment.
pub mod boot;
pub mod events;
/// Cursor paging, shared by [`events`] and [`payment_intents`].
///
/// `pub(crate)` unlike its two neighbours: every item in it is an
/// implementation detail of a handler, and there is no `ListPage` a caller
/// outside this crate could usefully build (the repositories take
/// `vpay_db::ListPage` directly). Keeping it crate-private is also what lets
/// its module docs link to the private functions they are about.
pub(crate) mod paging;
pub mod payment_intents;

/// The scope a token must carry to *change* anything under `/v1`.
///
/// This constant is the vocabulary's only definition. It is the string an
/// operator writes in a merchant registration's `scopes:`
/// (`config/application.yml`, `vpay_config::oauth::MerchantClient::scopes`),
/// the string the OP puts in the token's `scope` claim, and the string
/// [`crate::require_merchant_token`] checks — three places that fail
/// *silently* if they disagree: a token minted with a scope nothing checks
/// authorises everything, and a check for a scope nothing mints refuses
/// everything. There is no schema tying them together, so a shared constant
/// is what ties them.
pub const SCOPE_PAYMENTS_WRITE: &str = "payments:write";

/// The scope a token must carry to *read* under `/v1`.
///
/// Separate from [`SCOPE_PAYMENTS_WRITE`] so a merchant can issue a
/// credential to something that must observe payments without being able to
/// take them — a reconciliation job, a support tool. Write implies read (see
/// [`required_scopes`]): a credential that may create a payment intent can
/// obviously see the one it created, and requiring both strings on every
/// registration would be a trap with no security value.
pub const SCOPE_PAYMENTS_READ: &str = "payments:read";

/// Which scopes satisfy a request by its method: *any one* of the returned
/// scopes is enough.
///
/// The rule is per-method rather than per-route because it has to be decided
/// in the authentication middleware, which runs before axum matches a route
/// — and that is the right place for it anyway: "may this credential write?"
/// is a property of the credential and the verb, not of which resource is
/// being written.
///
/// Anything that is not a read method requires write, including methods no
/// route answers. That is deliberate fail-closed ordering: an unmatched
/// method reaches this function before it reaches the router's `405`, and
/// the answer to "may an unknown verb through on a read scope?" is no.
#[must_use]
pub fn required_scopes(method: &Method) -> &'static [&'static str] {
    match *method {
        // `HEAD` is here because axum answers it from the same `get(..)`
        // handler; refusing it on a read scope would refuse a request the
        // router is about to serve as a read.
        Method::GET | Method::HEAD => &[SCOPE_PAYMENTS_READ, SCOPE_PAYMENTS_WRITE],
        _ => &[SCOPE_PAYMENTS_WRITE],
    }
}

/// One mounted `/v1` route: the path axum matches, the methods it answers,
/// and the `MethodRouter` that serves them.
///
/// `methods` is carried alongside the router rather than derived from it
/// because axum 0.8 exposes no way to enumerate a built `Router`'s paths or
/// a `MethodRouter`'s methods. Keeping both halves in one entry is what
/// stops them drifting: a route added to `mount` without its method listed
/// is a route the boundary test would exercise with the wrong verb, which
/// is visible, rather than a route the test never sees at all.
#[derive(Debug)]
pub struct V1Route {
    /// The axum path pattern, e.g. `/payment_intents/{id}`, relative to the
    /// `/v1` nest.
    pub path: &'static str,
    /// Every HTTP method this path answers, upper-case.
    pub methods: &'static [&'static str],
    /// Builds the handlers. A function pointer rather than a value because
    /// `MethodRouter` is not `const`-constructible.
    pub(crate) mount: fn() -> MethodRouter<crate::AppState>,
}

/// Every route mounted under `/v1`, and the only place they are listed.
///
/// Deliberately **not** including `/v1/balance`: an SDK can call it and vpay
/// has no ledger read path, so it answers the honest 404 from the nest's
/// fallback rather than a route that would have to invent a body. See
/// `docs/status.md`.
///
/// `/v1/events` **was** on that list until 2026-09-03 and is now served
/// (Step 5): the same renderer the webhook deliverer signs is what it
/// returns, because a merchant who missed a webhook is told to re-read the
/// event here, and two renderers would let the fallback answer a different
/// question from the one the webhook asked. `?type=` is documented in
/// `docs/api/README.md` and deliberately **not** implemented — see
/// [`events`]'s module docs.
pub const V1_ROUTES: &[V1Route] = &[
    V1Route {
        path: "/payment_intents",
        methods: &["POST", "GET"],
        mount: || post(payment_intents::create).get(payment_intents::list),
    },
    V1Route {
        path: "/payment_intents/{id}",
        methods: &["GET"],
        mount: || get(payment_intents::retrieve),
    },
    V1Route {
        path: "/payment_intents/{id}/confirm",
        methods: &["POST"],
        mount: || post(payment_intents::confirm),
    },
    V1Route {
        path: "/payment_intents/{id}/cancel",
        methods: &["POST"],
        mount: || post(payment_intents::cancel),
    },
    V1Route {
        path: "/events",
        methods: &["GET"],
        mount: || get(events::list),
    },
    V1Route {
        path: "/events/{id}",
        methods: &["GET"],
        mount: || get(events::retrieve),
    },
];

/// The `/v1` resource router, built by folding [`V1_ROUTES`].
///
/// Returns a `Router` that still needs state and the authentication layer —
/// [`crate::router`] adds both, so this function cannot accidentally be
/// mounted unauthenticated.
pub(crate) fn routes() -> Router<crate::AppState> {
    V1_ROUTES
        .iter()
        .fold(Router::new(), |router, route| {
            router.route(route.path, (route.mount)())
        })
        // Inside the nest, so an unmatched `/v1/...` path is this crate's
        // envelope rather than axum's empty 404 body.
        .fallback(crate::not_found)
}

/// The tenant a request acts for, resolved from the token's `client_id` by
/// the authentication middleware and read from request extensions here.
///
/// Constructed only by [`crate::require_merchant_token`]: the field is
/// `pub(crate)` and the extractor below is the only public way to obtain
/// one, so a handler cannot invent a tenant to query by. That is the point —
/// see this module's docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerchantScope {
    pub(crate) merchant_id: String,
}

impl MerchantScope {
    /// The value every `/v1` query filters by.
    #[must_use]
    pub fn merchant_id(&self) -> &str {
        &self.merchant_id
    }

    /// The **one** other way a scope comes into existence: minted by
    /// [`crate::browser::PayerScope::as_merchant_scope`] once a publishable
    /// key and a payment intent's own `client_secret` have both been verified
    /// (Step 5c).
    ///
    /// `pub(crate)` and named for its caller rather than a general `new`,
    /// because the type's whole value is that a handler cannot invent one.
    /// A constructor called `new` would be an invitation; this one cannot be
    /// reached from outside the crate at all, and inside it there is exactly
    /// one call site, which the name points at.
    ///
    /// What it means is unchanged: "queries may be filtered by this tenant".
    /// It carries no OAuth scope claim and never did — `/v1`'s scope check
    /// happens in `crate::require_merchant_token`, before a `MerchantScope`
    /// exists, and the browser surface needs none because it mounts two
    /// routes and both address one already-authorised intent.
    pub(crate) fn for_payer(merchant_id: String) -> Self {
        Self { merchant_id }
    }
}

impl<S> FromRequestParts<S> for MerchantScope
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    /// Fails closed with [`ApiError::Internal`] (500, paged) rather than
    /// falling back to any other tenant, for the same reason
    /// [`crate::resource_auth::AuthenticatedMerchant`] does: reaching a
    /// handler with no scope on the request means the middleware is not
    /// mounted, and the safe answer to "which merchant's rows may I read"
    /// is none of them.
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<MerchantScope>()
            .cloned()
            .ok_or_else(|| {
                ApiError::Internal(
                    "a /v1 handler ran with no MerchantScope on the request: the merchant \
                 authentication middleware is not mounted in front of this route"
                        .to_owned(),
                )
            })
    }
}

/// One rail's deployment-specific material, as an adapter needs it.
///
/// Split out of [`ResourceConfig`] so the per-request work is a map lookup
/// and one `clone` of an already-projected value, rather than re-deriving a
/// callback URL and re-parsing a currency for every rail on every request.
#[derive(Debug, Clone)]
pub struct RailConfig {
    /// `providers.code`.
    code: String,
    /// Whether new intents may name this rail — [`vpay_config::ProviderHost::enabled`].
    enabled: bool,
    /// Projected once at boot by
    /// [`vpay_config::ProviderHost::to_provider_config`] — see
    /// [`RailConfig::provider_config`] for why the projection lives there and
    /// not here.
    provider_config: ProviderConfig,
}

impl RailConfig {
    /// This rail's code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The currency this rail settles in, from `providers[].currency`.
    ///
    /// Exposed separately from [`RailConfig::provider_config`] because the
    /// one caller that needs it — the confirm path's charge-vs-rail check —
    /// needs to *compare* it, and cloning a whole `ProviderConfig`
    /// (credentials included) per request to read one field would put rail
    /// secrets on the stack of a validation function for no reason.
    #[must_use]
    pub fn currency(&self) -> vpay_core::Currency {
        self.provider_config.currency
    }

    /// The [`ProviderConfig`] handed to `ProviderAdapter::submit`.
    ///
    /// # What changed in Step 3, and what it means
    ///
    /// This used to take the *charge's* currency (Step 2's D2: the charge
    /// carries the intent's currency verbatim). It no longer does: the
    /// currency an adapter is told to transact in is now
    /// `providers[].currency` from the YAML, because it is a property of the
    /// rail's *profile* — MTN's sandbox rejects XAF and accepts EUR only
    /// (`docs/flows/money.md`), which is a deployment fact and must never be
    /// a code branch (ADR-0003).
    ///
    /// The charge is unaffected: `charges.currency_code` is still the
    /// intent's own, and `Money` still carries it, so
    /// `Money::to_provider_string` still renders with the *charge's*
    /// exponent.
    ///
    /// **The two are now reconciled at confirm, not left to the rail.**
    /// `vpay_api::v1::payment_intents`' `currencies_agree` refuses a confirm
    /// whose intent currency is not this one, with a `400` naming
    /// `payment_method_data[type]`, before any charge row is written. The
    /// alternative — submitting a XAF amount under a EUR profile because
    /// "amounts against a EUR profile are notional" (`docs/flows/money.md`)
    /// — is a payer charged the wrong unit by a rail that simply believes
    /// the number. Refusing at *boot* was the other candidate and is
    /// deliberately not done: a deployment may legitimately offer several
    /// rails in several currencies, and which pair is illegal is a property
    /// of the request, not of the file.
    ///
    /// Built at boot rather than per call because the projection can fail
    /// (an unknown currency) and a request path is the wrong place to
    /// discover a configuration defect.
    #[must_use]
    pub fn provider_config(&self) -> ProviderConfig {
        self.provider_config.clone()
    }
}

/// One merchant webhook endpoint, projected out of
/// `vpay_config::oauth::WebhookEndpoint` at boot.
///
/// # Why this type exists at all, rather than re-exporting the config type
///
/// [`ResourceConfig`] is a *projection*: what is in it is exactly what a
/// request path — and, since Step 5, the worker's binary — is allowed to
/// depend on. Carrying the YAML type would carry whatever else that type
/// grows next, and the whole point of the projection is that a handler which
/// could see `merchant_clients` could see a JWK set.
///
/// # Why the worker does not get `vpay_worker::webhooks::Endpoint` from here
///
/// It cannot: `vpay-worker` depends on `vpay-api` (it renders the delivered
/// body through [`crate::model::EventObject`]), so an edge back would be a
/// cycle. The worker **binary** — which links both — converts these into
/// `vpay_worker::webhooks::Endpoint` and calls `EndpointRegistry::from_pairs`
/// on the result of [`ResourceConfig::webhook_endpoints`]. That is the one
/// place the two shapes meet, and it is a binary, where linking everything is
/// the job.
#[derive(Clone, PartialEq, Eq)]
pub struct WebhookEndpointConfig {
    id: String,
    url: String,
    secrets: Vec<String>,
}

/// Redacts [`WebhookEndpointConfig::secrets`] down to a count.
///
/// [`ResourceConfig`] derives `Debug` and lives in the server's `AppState`
/// for the whole life of the process, so anything it holds is one `{:?}`
/// away from a log. A webhook secret in a log is a forged webhook: whoever
/// holds it can sign a `payment_intent.succeeded` the merchant's handler
/// will believe. Mirrors `vpay_config::oauth::WebhookEndpoint`'s own impl —
/// see that one for why the count, the id and the URL stay visible.
impl fmt::Debug for WebhookEndpointConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookEndpointConfig")
            .field("id", &self.id)
            .field("url", &self.url)
            .field(
                "secrets",
                &format_args!("[{} redacted]", self.secrets.len()),
            )
            .finish()
    }
}

impl WebhookEndpointConfig {
    /// The operator-authored id, stored verbatim on every
    /// `webhook_deliveries` row (migration 0022).
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Where the signed event body is POSTed. Validated at **boot** by
    /// `vpay_config`'s `validate_host`, never at delivery time.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The signing secrets, in configuration order — one `v1=` each.
    #[must_use]
    pub fn secrets(&self) -> &[String] {
        &self.secrets
    }
}

/// The parts of the validated YAML configuration a `/v1` request needs.
///
/// Built once at boot from a [`Config`] and shared by `Arc`. It is a
/// *projection*, not a copy of the whole `Config`: a handler that could see
/// `merchant_clients` could see a JWK set, and a handler that could see
/// `deployment` could branch on the deployment name — which ADR-0003
/// forbids outright. What is here is exactly what a request path is allowed
/// to depend on.
#[derive(Debug, Clone)]
pub struct ResourceConfig {
    livemode: bool,
    merchant_id_by_client_id: BTreeMap<String, String>,
    currency_codes: BTreeSet<String>,
    rails: BTreeMap<String, RailConfig>,
    /// Keyed on `events.merchant_id`, **not** on `client_id` like
    /// [`Self::merchant_id_by_client_id`]: the fan-out key is the event's
    /// merchant, one merchant may hold several credentials, and a map keyed
    /// the other way would fan out to the endpoints of whichever client
    /// happened to be looked up (`docs/plans/2026-09-03-step5-webhooks.md`,
    /// S5). Two clients naming one tenant is refused at boot today
    /// (`ConfigError::DuplicateMerchantId`), so the merge below cannot
    /// currently lose an endpoint — it is written to merge anyway, because
    /// the day that rule is relaxed the failure would be silent.
    endpoints_by_merchant_id: BTreeMap<String, Vec<WebhookEndpointConfig>>,
    /// Which tenant a `pk_test_…`/`pk_live_…` names — the first step of
    /// [`crate::browser::authenticate`] (Step 5c D1).
    ///
    /// Keyed on the **tenant** like [`Self::endpoints_by_merchant_id`] and
    /// unlike [`Self::merchant_id_by_client_id`], but for a different reason:
    /// there is no credential in the picture at all on the browser surface.
    /// A publishable key names who the intent must belong to, and the
    /// intent's own `client_secret` is what authorises the request.
    ///
    /// `Config::validate_all` refuses a key claimed by two merchants
    /// (`ConfigError::DuplicatePublishableKey`), so this map cannot silently
    /// lose one — it is built with a plain `collect` rather than a merge for
    /// exactly that reason, and the day the rule is relaxed the collision
    /// would have to be decided here rather than discovered.
    merchant_id_by_publishable_key: BTreeMap<String, String>,
}

impl ResourceConfig {
    /// Projects a loaded, validated [`Config`].
    ///
    /// Takes `&Config` rather than the pieces so that a field added to the
    /// projection is one edit, and so both binaries build it identically —
    /// a server and a worker disagreeing about which currencies exist would
    /// be a defect with no visible symptom until a charge failed.
    ///
    /// # Errors
    ///
    /// Whatever [`vpay_config::ProviderHost::to_provider_config`] returns —
    /// today, a `providers[].currency` outside
    /// [`vpay_core::Currency`]. `Config::load` has already refused to boot on
    /// that, so this is unreachable for a `Config` that came from a file;
    /// it is `Result` rather than a silent skip because a rail dropped from
    /// this map would make every charge on it answer "unknown rail", which
    /// looks like a merchant's typo rather than our configuration defect.
    pub fn from_config(config: &Config) -> Result<Self, vpay_config::ConfigError> {
        let merchant_id_by_client_id = config
            .merchant_clients
            .iter()
            .map(|client| (client.client_id.clone(), client.merchant_id.clone()))
            .collect();
        let currency_codes = config
            .currencies
            .iter()
            .map(|entry| entry.code.to_ascii_uppercase())
            .collect();
        // The callback-URL derivation and the currency parse live in
        // `vpay-config`, on the side that owns the YAML shape, so the server
        // and the worker cannot derive a rail's callback URL differently —
        // and so this crate never learns what a config file looks like.
        let rails = config
            .providers
            .iter()
            .map(|provider| {
                let rail = RailConfig {
                    code: provider.code.clone(),
                    enabled: provider.enabled,
                    provider_config: provider.to_provider_config(&config.deployment)?,
                };
                Ok((provider.code.clone(), rail))
            })
            .collect::<Result<BTreeMap<_, _>, vpay_config::ConfigError>>()?;

        let mut endpoints_by_merchant_id: BTreeMap<String, Vec<WebhookEndpointConfig>> =
            BTreeMap::new();
        for client in &config.merchant_clients {
            endpoints_by_merchant_id
                .entry(client.merchant_id.clone())
                .or_default()
                .extend(
                    client
                        .webhooks
                        .iter()
                        .map(|endpoint| WebhookEndpointConfig {
                            id: endpoint.id.clone(),
                            url: endpoint.url.clone(),
                            secrets: endpoint.secrets.clone(),
                        }),
                );
        }

        let merchant_id_by_publishable_key = config
            .merchant_clients
            .iter()
            .flat_map(|client| {
                client
                    .publishable_keys
                    .iter()
                    .map(|key| (key.clone(), client.merchant_id.clone()))
            })
            .collect();

        Ok(Self {
            livemode: config.deployment.livemode,
            merchant_id_by_client_id,
            currency_codes,
            rails,
            endpoints_by_merchant_id,
            merchant_id_by_publishable_key,
        })
    }

    /// `deployment.livemode`, stamped on every object this deployment
    /// returns. A label, never a branch (ADR-0003).
    #[must_use]
    pub fn livemode(&self) -> bool {
        self.livemode
    }

    /// The tenant a token's `client_id` acts for, or `None` if this
    /// deployment has no registration for it.
    ///
    /// `None` is reachable in one real situation: a token minted before a
    /// config change that removed the client, presented inside its
    /// remaining TTL. The middleware answers 403 for it — the credential is
    /// genuine, and there is no longer a tenant it may act on.
    #[must_use]
    pub fn merchant_id_for(&self, client_id: &str) -> Option<&str> {
        self.merchant_id_by_client_id
            .get(client_id)
            .map(String::as_str)
    }

    /// The tenant a publishable key names, or `None` if this deployment has
    /// no registration carrying it.
    ///
    /// **`None` is the common case and is not an error.** Most deployments
    /// register no publishable keys at all, and a payer's browser is an
    /// unauthenticated caller that can send anything — so this answering
    /// `None` is what a mistyped key, a retired key, or a key from another
    /// deployment all look like, and [`crate::browser::authenticate`] turns
    /// every one of them into the same 404 as a wrong `client_secret`.
    ///
    /// Deliberately **not** a `merchant_id_for` overload: the two resolve
    /// different things (a credential vs. a browser-side tenant label) from
    /// different namespaces, and one function taking either would be one
    /// `client_id` typo away from letting a merchant's own id act as a
    /// publishable key.
    #[must_use]
    pub fn merchant_id_for_publishable_key(&self, key: &str) -> Option<&str> {
        self.merchant_id_by_publishable_key
            .get(key)
            .map(String::as_str)
    }

    /// Whether `code` (upper-case ISO 4217) is one this deployment admits.
    ///
    /// Two gates, not one: [`vpay_core::Currency`] says the *system* knows
    /// the code, this says the *deployment* configured it. A deployment that lists
    /// only XAF must not accept an EUR intent it has no rail configured to
    /// charge.
    #[must_use]
    pub fn admits_currency(&self, code: &str) -> bool {
        self.currency_codes.contains(code)
    }

    /// The rail configured under `code`, whether or not it is enabled.
    #[must_use]
    pub fn rail(&self, code: &str) -> Option<&RailConfig> {
        self.rails.get(code)
    }

    /// The rail configured under `code`, if it is enabled for new charges.
    #[must_use]
    pub fn enabled_rail(&self, code: &str) -> Option<&RailConfig> {
        self.rails.get(code).filter(|rail| rail.enabled)
    }

    /// This merchant's webhook endpoints, or an empty slice.
    ///
    /// An empty slice is a *complete* answer and not a missing entry: a
    /// merchant who has configured none still has their events fanned out
    /// (to nothing) and marked `done`, because the alternative is a backlog
    /// index that grows forever (`vpay_worker::webhooks::handle_fan_out`).
    #[must_use]
    pub fn endpoints_for(&self, merchant_id: &str) -> &[WebhookEndpointConfig] {
        self.endpoints_by_merchant_id
            .get(merchant_id)
            .map_or(&[], Vec::as_slice)
    }

    /// Every merchant's endpoints, as `(merchant_id, endpoints)` pairs —
    /// what the **worker binary** feeds `EndpointRegistry::from_pairs`.
    ///
    /// Pairs rather than the map itself because that is the shape
    /// `vpay_worker::webhooks::EndpointRegistry::from_pairs` takes, and
    /// because it is the shape that stays correct if a merchant ever ends up
    /// described by two entries. This crate cannot build the registry itself
    /// — see [`WebhookEndpointConfig`] for the dependency direction that
    /// forbids it.
    pub fn webhook_endpoints(&self) -> impl Iterator<Item = (&str, &[WebhookEndpointConfig])> + '_ {
        self.endpoints_by_merchant_id
            .iter()
            .map(|(merchant_id, endpoints)| (merchant_id.as_str(), endpoints.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use vpay_config::{CurrencyEntry, Deployment, HostEntry, ProviderHost};

    use super::*;

    fn config() -> Config {
        Config {
            deployment: Deployment {
                name: "test".to_owned(),
                livemode: false,
                public_base_url: "https://api.vpay.test/".to_owned(),
            },
            providers: vec![
                ProviderHost {
                    code: "mtn_momo".to_owned(),
                    enabled: true,
                    host: HostEntry {
                        url: "https://mtn.example".to_owned(),
                        label: "mtn".to_owned(),
                    },
                    settings: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
                    callback_url: None,
                    currency: "XAF".to_owned(),
                    credentials: BTreeMap::from([("api_key".to_owned(), "secret".to_owned())]),
                },
                ProviderHost {
                    code: "orange_money".to_owned(),
                    enabled: false,
                    host: HostEntry {
                        url: "https://orange.example".to_owned(),
                        label: "orange".to_owned(),
                    },
                    settings: BTreeMap::new(),
                    callback_url: None,
                    currency: "XAF".to_owned(),
                    credentials: BTreeMap::new(),
                },
            ],
            currencies: vec![CurrencyEntry {
                code: "XAF".to_owned(),
                exponent: 0,
            }],
            merchant_clients: vec![crate::test_fixtures::merchant(
                "acme-cameroon",
                &["payments:write"],
            )],
            dashboard_client: None,
        }
    }

    /// Every method the surface can be reached with, and what it costs.
    ///
    /// The two that matter: a read scope must not authorise a `POST` (a
    /// reconciliation credential must not be able to take a payment), and a
    /// method no route answers must not fall through on a read scope — the
    /// middleware sees it before the router's `405` does.
    #[test]
    fn only_a_write_scope_authorises_a_method_that_is_not_a_read() {
        for method in [Method::GET, Method::HEAD] {
            assert_eq!(
                required_scopes(&method),
                [SCOPE_PAYMENTS_READ, SCOPE_PAYMENTS_WRITE],
                "{method} is a read"
            );
        }
        for method in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ] {
            assert_eq!(
                required_scopes(&method),
                [SCOPE_PAYMENTS_WRITE],
                "{method} must need the write scope"
            );
        }
    }

    /// The scope strings are a wire contract shared with every merchant's
    /// YAML registration and with the `scope` claim the OP mints, so they
    /// are pinned as literals here: a rename that only touched the constants
    /// would compile, pass every other test, and silently refuse every token
    /// already registered against the old spelling.
    #[test]
    fn the_scope_names_are_the_ones_registrations_are_written_against() {
        assert_eq!(SCOPE_PAYMENTS_WRITE, "payments:write");
        assert_eq!(SCOPE_PAYMENTS_READ, "payments:read");
    }

    #[test]
    fn a_clients_tenant_comes_from_config_not_from_the_client_id() {
        let resource_config = ResourceConfig::from_config(&config())
            .expect("the fixture's rails project onto the port");
        assert_eq!(
            resource_config.merchant_id_for("acme-cameroon"),
            Some("acme-cameroon-tenant"),
            "the tenant is the configured merchant_id, never the client_id"
        );
        assert_eq!(resource_config.merchant_id_for("someone-else"), None);
    }

    #[test]
    fn a_disabled_rail_is_configured_but_not_offered() {
        let resource_config = ResourceConfig::from_config(&config())
            .expect("the fixture's rails project onto the port");
        assert!(resource_config.rail("orange_money").is_some());
        assert!(
            resource_config.enabled_rail("orange_money").is_none(),
            "a disabled rail must not be selectable for a new charge"
        );
        assert!(resource_config.enabled_rail("mtn_momo").is_some());
    }

    /// What a rail call is handed: the derived callback URL (with the
    /// config's trailing slash not surviving into it), the rail's *own*
    /// currency from the YAML rather than a charge's, and the credentials
    /// verbatim.
    ///
    /// The derivation itself is `vpay-config`'s and is tested there; this
    /// asserts that the projection reaches a request path intact, which is
    /// the part that would break if `ResourceConfig` ever grew a second
    /// copy of the rule.
    #[test]
    fn the_callback_url_is_derived_from_the_public_base_url() {
        let resource_config = ResourceConfig::from_config(&config())
            .expect("the fixture's rails project onto the port");
        let rail = resource_config.rail("mtn_momo").expect("mtn is configured");
        let provider_config = rail.provider_config();
        assert_eq!(
            provider_config.callback_url,
            "https://api.vpay.test/provider/mtn_momo/callback"
        );
        assert_eq!(provider_config.base_url, "https://mtn.example");
        assert_eq!(provider_config.currency, vpay_core::Currency::Xaf);
        assert_eq!(
            provider_config
                .credentials
                .get("api_key")
                .map(String::as_str),
            Some("secret")
        );
    }

    /// The endpoint table is keyed on the **tenant**, not the credential.
    ///
    /// This is `docs/plans/2026-09-03-step5-webhooks.md`'s S5 in one
    /// assertion: `events.merchant_id` is the fan-out key, and a table keyed
    /// on `client_id` — which is what the projection beside it uses — would
    /// look up the endpoints of whichever credential happened to be asked
    /// about. The fixture's client id and merchant id deliberately differ,
    /// so a projection that confused the two fails here rather than
    /// delivering a merchant's events to nobody.
    #[test]
    fn webhook_endpoints_are_keyed_on_the_tenant_and_not_on_the_credential() {
        let mut config = config();
        let client = config
            .merchant_clients
            .first_mut()
            .expect("the fixture registers one merchant");
        client.webhooks = vec![vpay_config::WebhookEndpoint {
            id: "primary".to_owned(),
            url: "https://hooks.acme.example/vpay".to_owned(),
            secrets: vec!["whsec-never-log-me".to_owned()],
        }];

        let resource_config = ResourceConfig::from_config(&config)
            .expect("the fixture's rails project onto the port");

        let endpoints = resource_config.endpoints_for("acme-cameroon-tenant");
        assert_eq!(endpoints.len(), 1, "the tenant has one endpoint");
        let endpoint = endpoints.first().expect("one endpoint");
        assert_eq!(endpoint.id(), "primary");
        assert_eq!(endpoint.url(), "https://hooks.acme.example/vpay");
        assert_eq!(endpoint.secrets(), ["whsec-never-log-me".to_owned()]);

        // The credential's own id is not a tenant and resolves to nothing.
        assert!(resource_config.endpoints_for("acme-cameroon").is_empty());
        // A merchant nobody registered gets an empty slice, not a panic and
        // not a `None` a caller could mistake for "not configured yet".
        assert!(resource_config.endpoints_for("someone-else").is_empty());

        // The pairs the worker binary feeds `EndpointRegistry::from_pairs`.
        let pairs: Vec<&str> = resource_config
            .webhook_endpoints()
            .map(|(merchant_id, _)| merchant_id)
            .collect();
        assert_eq!(pairs, ["acme-cameroon-tenant"]);
    }

    /// `ResourceConfig` lives in the server's `AppState` for the life of the
    /// process, so anything it holds is one `{:?}` away from a log — and a
    /// webhook secret in a log is a forged webhook.
    ///
    /// The endpoint id and URL stay visible on purpose: they are already in
    /// the delivery rows and in the merchant's own configuration, and an
    /// operator asking "why did this merchant get no webhook?" needs to see
    /// what was actually loaded.
    #[test]
    fn a_resource_configs_debug_output_never_contains_a_webhook_secret() {
        let mut config = config();
        config
            .merchant_clients
            .first_mut()
            .expect("the fixture registers one merchant")
            .webhooks = vec![vpay_config::WebhookEndpoint {
            id: "primary".to_owned(),
            url: "https://hooks.acme.example/vpay".to_owned(),
            secrets: vec!["whsec-never-log-me".to_owned()],
        }];

        let formatted = format!(
            "{:?}",
            ResourceConfig::from_config(&config).expect("the fixture projects onto the port")
        );

        assert!(!formatted.contains("whsec-never-log-me"), "{formatted}");
        assert!(formatted.contains("[1 redacted]"), "{formatted}");
        assert!(formatted.contains("hooks.acme.example"), "{formatted}");
    }

    /// The browser surface's first step: a publishable key resolves to a
    /// tenant, and to nothing else.
    ///
    /// The fixture's `client_id`, `merchant_id` and publishable key are three
    /// different strings on purpose. A projection that keyed this map on the
    /// credential — which is what the map beside it does — would pass every
    /// other test in this file and would then authenticate a payer against
    /// whichever merchant happened to share a name with their key.
    #[test]
    fn a_publishable_key_resolves_to_a_tenant_and_a_client_id_does_not() {
        let mut config = config();
        config
            .merchant_clients
            .first_mut()
            .expect("the fixture registers one merchant")
            .publishable_keys = vec!["pk_test_acmecameroonsandbox01".to_owned()];

        let resource_config = ResourceConfig::from_config(&config)
            .expect("the fixture's rails project onto the port");

        assert_eq!(
            resource_config.merchant_id_for_publishable_key("pk_test_acmecameroonsandbox01"),
            Some("acme-cameroon-tenant")
        );
        // The credential's own id is not a publishable key, and neither is
        // the tenant: the browser namespace is separate from both.
        assert_eq!(
            resource_config.merchant_id_for_publishable_key("acme-cameroon"),
            None
        );
        assert_eq!(
            resource_config.merchant_id_for_publishable_key("acme-cameroon-tenant"),
            None
        );
        // And a key nobody registered — a payer can send anything.
        assert_eq!(
            resource_config.merchant_id_for_publishable_key("pk_test_neverregisteredanywhere"),
            None
        );
    }

    /// The fail-closed default: a deployment that registered no publishable
    /// keys resolves nothing, so its `/v1/browser` surface answers 404 to
    /// every request rather than falling back to a tenant.
    #[test]
    fn a_deployment_with_no_publishable_keys_resolves_nothing() {
        let resource_config = ResourceConfig::from_config(&config())
            .expect("the fixture's rails project onto the port");
        assert_eq!(
            resource_config.merchant_id_for_publishable_key("pk_test_anythingatall000000"),
            None
        );
    }

    #[test]
    fn a_currency_the_deployment_did_not_configure_is_not_admitted() {
        let resource_config = ResourceConfig::from_config(&config())
            .expect("the fixture's rails project onto the port");
        assert!(resource_config.admits_currency("XAF"));
        // A currency `vpay_core::Currency` knows, that this deployment did
        // not list. Both gates have to pass — see `admits_currency`.
        assert!(!resource_config.admits_currency("EUR"));
    }
}
