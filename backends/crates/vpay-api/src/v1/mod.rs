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

use axum::Router;
use axum::extract::FromRequestParts;
use axum::http::Method;
use axum::http::request::Parts;
use axum::routing::{MethodRouter, get, post};
use vpay_config::Config;
use vpay_core::Currency;
use vpay_provider::ProviderConfig;

use crate::error::ApiError;

/// Boot step 4's derivation — the seeds both binaries hand
/// `vpay_db::config_reconcile::reconcile`. Not a request path, and here
/// rather than in either `main.rs` because both binaries need the identical
/// answer; see the module's own doc comment.
pub mod boot;
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
/// Deliberately **not** including `/v1/balance` or `/v1/events`: both SDKs
/// can call them and vpay implements neither, so they answer the honest 404
/// from the nest's fallback rather than a route that would have to invent a
/// body. See `docs/status.md`.
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
/// and one `clone` of the small maps, rather than re-deriving a callback URL
/// and re-copying credentials for every rail on every request.
#[derive(Debug, Clone)]
pub struct RailConfig {
    /// `providers.code`.
    code: String,
    /// Whether new intents may name this rail — [`vpay_config::ProviderHost::enabled`].
    enabled: bool,
    base_url: String,
    callback_url: String,
    settings: BTreeMap<String, String>,
    credentials: BTreeMap<String, String>,
}

impl RailConfig {
    /// This rail's code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The [`ProviderConfig`] handed to `ProviderAdapter::submit` for a
    /// charge in `currency`.
    ///
    /// `currency` is a parameter rather than a stored field because a rail
    /// is not tied to one currency — the charge carries the intent's
    /// currency verbatim, with no conversion and no per-rail currency check
    /// (Step 2's D2). Storing one on the rail would make that a property of
    /// the deployment and quietly reject the second currency a rail
    /// supports.
    #[must_use]
    pub fn provider_config(&self, currency: Currency) -> ProviderConfig {
        ProviderConfig {
            base_url: self.base_url.clone(),
            callback_url: self.callback_url.clone(),
            currency,
            settings: self.settings.clone(),
            credentials: self.credentials.clone(),
        }
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
}

impl ResourceConfig {
    /// Projects a loaded, validated [`Config`].
    ///
    /// Takes `&Config` rather than the pieces so that a field added to the
    /// projection is one edit, and so both binaries build it identically —
    /// a server and a worker disagreeing about which currencies exist would
    /// be a defect with no visible symptom until a charge failed.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
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
        let base = config.deployment.public_base_url.trim_end_matches('/');
        let rails = config
            .providers
            .iter()
            .map(|provider| {
                let rail = RailConfig {
                    code: provider.code.clone(),
                    enabled: provider.enabled,
                    base_url: provider.host.url.clone(),
                    // `docs/api/README.md`'s `/provider/{code}/callback`.
                    // Derived rather than configured: the rail is told where
                    // to call back, and a value an operator could mistype
                    // would point a live rail at nothing.
                    callback_url: format!("{base}/provider/{}/callback", provider.code),
                    settings: provider.settings.clone(),
                    credentials: provider.credentials.clone(),
                };
                (provider.code.clone(), rail)
            })
            .collect();

        Self {
            livemode: config.deployment.livemode,
            merchant_id_by_client_id,
            currency_codes,
            rails,
        }
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

    /// Whether `code` (upper-case ISO 4217) is one this deployment admits.
    ///
    /// Two gates, not one: [`Currency`] says the *system* knows the code,
    /// this says the *deployment* configured it. A deployment that lists
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
        let resource_config = ResourceConfig::from_config(&config());
        assert_eq!(
            resource_config.merchant_id_for("acme-cameroon"),
            Some("acme-cameroon-tenant"),
            "the tenant is the configured merchant_id, never the client_id"
        );
        assert_eq!(resource_config.merchant_id_for("someone-else"), None);
    }

    #[test]
    fn a_disabled_rail_is_configured_but_not_offered() {
        let resource_config = ResourceConfig::from_config(&config());
        assert!(resource_config.rail("orange_money").is_some());
        assert!(
            resource_config.enabled_rail("orange_money").is_none(),
            "a disabled rail must not be selectable for a new charge"
        );
        assert!(resource_config.enabled_rail("mtn_momo").is_some());
    }

    /// The callback URL is derived from `deployment.public_base_url`, and the
    /// trailing slash a config might carry must not survive into it.
    #[test]
    fn the_callback_url_is_derived_from_the_public_base_url() {
        let resource_config = ResourceConfig::from_config(&config());
        let rail = resource_config.rail("mtn_momo").expect("mtn is configured");
        let provider_config = rail.provider_config(Currency::Xaf);
        assert_eq!(
            provider_config.callback_url,
            "https://api.vpay.test/provider/mtn_momo/callback"
        );
        assert_eq!(provider_config.base_url, "https://mtn.example");
        assert_eq!(provider_config.currency, Currency::Xaf);
        assert_eq!(
            provider_config
                .credentials
                .get("api_key")
                .map(String::as_str),
            Some("secret")
        );
    }

    #[test]
    fn a_currency_the_deployment_did_not_configure_is_not_admitted() {
        let resource_config = ResourceConfig::from_config(&config());
        assert!(resource_config.admits_currency("XAF"));
        // A currency `vpay_core::Currency` knows, that this deployment did
        // not list. Both gates have to pass — see `admits_currency`.
        assert!(!resource_config.admits_currency("EUR"));
    }
}
