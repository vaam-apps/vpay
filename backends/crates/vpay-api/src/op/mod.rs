//! The merchant-facing OAuth2 provider behind `/v1/oauth` (ADR-0010,
//! [docs/flows/merchant-auth.md](../../../../../docs/flows/merchant-auth.md)).
//!
//! Five pieces, each its own module so it can be tested on its own:
//! [`clients`] (the `ClientStore`, YAML minus the kill switch), [`keys`] (the
//! RS256 signing key, loaded from a file at boot and never persisted),
//! [`jwks`] (the database-backed `/jwks.json`, which publishes every key still
//! in its rotation window rather than the one this process holds), [`token`]
//! (the two handlers vpay writes itself), and [`refusing_stores`] (the three
//! `OpStore` slots `/v1` must fill and no `/v1` grant can reach).
//!
//! [`MerchantOp`] is the assembly. Nothing here serves the dashboard surface.
//!
//! Why vpay writes its own handlers instead of mounting `authkestra-axum`, and
//! what it deliberately does *not* re-implement:
//! [docs/reference/vpay-api.md § the merchant OP](../../../../../docs/reference/vpay-api.md#the-merchant-op-op).

use std::collections::BTreeMap;
use std::sync::Arc;

use authkestra_engine::token::TokenManager;
use authkestra_op::OpStore;
use authkestra_op::config::OpConfig;
use authkestra_op::store::CompositeOpStore;
use vpay_config::Config;
use vpay_db::Repositories;

use crate::op::clients::YamlClientStore;
use crate::op::keys::LoadedSigningKey;
use crate::op::refusing_stores::{
    RefusingAuthorizationCodeStore, RefusingDeviceCodeStore, RefusingRefreshTokenStore,
};

pub mod clients;
pub mod jwks;
pub mod keys;
pub mod refusing_stores;
pub mod token;

/// The access-token lifetime `/v1` mints, in seconds.
///
/// **15 minutes is the plan's default, not a decision that has been made.**
/// `docs/roadmap.md` lists the merchant access-token TTL as an open
/// question; no ADR or flow doc fixes a number, and this constant does not
/// close that question — it is the value this code uses until a maintainer
/// settles it.
///
/// The two constraints it does have to satisfy, both of which 900 s meets
/// comfortably:
///
/// - It must be shorter than [`keys::ROTATION_OVERLAP`] (24 h) by a wide
///   margin, or a token signed just before a rotation could outlive the
///   window in which its key is still published. That constant's own doc
///   comment does this arithmetic against this number.
/// - It must be long enough that a merchant is not spending a
///   `client_assertion` `jti` on every request. `sdks/rust` caches the token
///   until `expires_in` minus a 30 s margin, so at 900 s a busy client mints
///   roughly four assertions an hour.
///
/// Not configurable per deployment on purpose: a TTL that varies by YAML is
/// one more thing that can differ between the sandbox a merchant integrates
/// against and the production they go live on.
pub const ACCESS_TOKEN_TTL_SECS: u64 = 900;

/// The one grant `/v1` offers.
///
/// `client_credentials` and nothing else, matching ADR-0010 and
/// `vpay_config::ConfigError::DisallowedMerchantGrant`, which refuses to
/// boot a merchant registration declaring anything else. Machine-to-machine
/// only: there is no browser leg, no user consent, no refresh token and no
/// device flow on this surface.
///
/// This list is what the discovery document advertises. It is *not* what
/// enforces the restriction at runtime — `authkestra_op`'s `handle_token`
/// dispatches on `grant_type` without consulting
/// `OpConfig::grant_types_supported` at all (read
/// `authkestra-op-0.7.1/src/handlers/token.rs`, the `match
/// req.grant_type.as_str()` block). What actually refuses every other grant
/// is each grant handler's own `client.allows_grant_type(..)` check against
/// the registration [`clients::registration_for`] built from YAML, which
/// for a merchant can only ever contain `client_credentials`. The two agree
/// because config validation makes them agree, not because this constant is
/// consulted.
pub const OP_GRANT_TYPES: [&str; 1] = ["client_credentials"];

/// The client-authentication method `/v1` accepts, and the only one.
///
/// `private_key_jwt` (RFC 7523 §2.2). vpay stores no merchant secret in any
/// form (`vpay_config::ConfigError::ClientSecretPresent` refuses to boot a
/// config that supplies one), and every merchant registration carries
/// `token_endpoint_auth_method: Some(PrivateKeyJwt)`, which
/// `authkestra_op::handlers::token::authenticate_client` treats as an
/// exclusive binding rather than a preference — a `client_secret_basic`
/// credential presented by such a client is a failure, never a fallback.
pub const OP_TOKEN_ENDPOINT_AUTH_METHOD: &str = "private_key_jwt";

/// The JWS algorithms a `client_assertion` may be signed with.
///
/// Transcribed from `authkestra-op-0.7.1/src/client_assertion.rs`'s private
/// `assertion_algorithms`, which derives the accepted set **from the
/// registered JWK's key type**, not from any configuration vpay controls:
/// an RSA key admits `RS*`/`PS*`, a P-256 key `ES256`, a P-384 key `ES384`,
/// an OKP key `EdDSA`. So this is the union across key types — the honest
/// answer to "what could this endpoint accept", not a promise that any one
/// client may use any one of them.
///
/// RFC 8414 §2 makes `token_endpoint_auth_signing_alg_values_supported`
/// REQUIRED once `private_key_jwt` is advertised, which is why the list is
/// published rather than omitted. It is a transcription and can drift: any
/// `authkestra-op` bump must re-diff it against that function (the same
/// re-diff discipline the root `Cargo.toml` already demands for
/// `SqlxOpStore::migrate`).
pub const OP_ASSERTION_SIGNING_ALGS: [&str; 9] = [
    "RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256", "ES384", "EdDSA",
];

/// The assembled `/v1` OAuth2 provider: one config, one store, one signer.
///
/// Built once in `main` and shared by every request through an `Arc` in
/// router state. Holds no per-request state of its own, so nothing here
/// needs a lock.
///
/// Deliberately not `Clone`: the store is a trait object behind an `Arc`
/// already, and handing out clones of the whole struct would make it easy
/// to end up with two `OpConfig`s that could disagree about the issuer.
/// Callers share it as `Arc<MerchantOp>`.
pub struct MerchantOp {
    config: OpConfig,
    store: Arc<dyn OpStore>,
    tokens: Arc<TokenManager>,
    default_scopes: BTreeMap<String, String>,
}

/// Shows the configuration (public metadata, all of it published at
/// `/.well-known/openid-configuration`) and nothing else. The store holds a
/// database pool and the `TokenManager` holds the private signing key;
/// neither belongs in a `{:?}`, and neither has a `Debug` impl of its own.
impl std::fmt::Debug for MerchantOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MerchantOp")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl MerchantOp {
    /// Assembles the OP from a validated [`Config`], the signing key this
    /// process loaded, and the database pool.
    ///
    /// `key` is taken by value: the caller has no further use for it after
    /// [`keys::LoadedSigningKey::ensure_active_in_database`] has run, and
    /// moving it makes "the OP signs with the key this process announced"
    /// hard to get wrong.
    ///
    /// # The issuer
    ///
    /// `{public_base_url}/v1/oauth`, with any trailing slash on
    /// `public_base_url` removed first — [`OpConfig::issuer`]'s own contract
    /// is "no trailing slash", and a `public_base_url` of
    /// `https://api.example/` would otherwise produce
    /// `https://api.example//v1/oauth`, which is a *different string* to
    /// every consumer that compares it: the `iss` claim the resource
    /// validator pins, the `aud` a merchant's `client_assertion` must
    /// carry, and the `issuer` in the discovery document the SDK reads.
    ///
    /// This value has to be the same string in three places or `/v1` fails
    /// closed with no diagnostic: here, in
    /// [`keys::LoadedSigningKey::from_file`]'s `issuer` argument (what gets
    /// stamped into every token), and in
    /// [`crate::resource_auth::JwtValidator::new`]'s `issuer` (what every
    /// token is checked against). `main` derives all three from
    /// [`Self::issuer`] for exactly that reason.
    ///
    /// # The store
    ///
    /// Clients come from [`YamlClientStore`] (YAML for identity, the
    /// `disabled_clients` table for revocation). Spent `client_assertion`
    /// `jti`s go to [`vpay_db::client_assertion_store`], which is what turns a
    /// captured assertion from a replayable bearer credential into a
    /// single-use one — `authkestra_op`'s `OpStore` refuses every assertion
    /// unless one is wired (`NoClientAssertionStore` fails closed).
    ///
    /// **The other three slots serve no `/v1` grant, and now say so.**
    /// `OpStore` is a supertrait of `AuthorizationCodeStore`,
    /// `RefreshTokenStore` and `DeviceCodeStore`, so a value implementing it
    /// must supply all three whether or not any grant reaches them — and
    /// none does here: every grant handler other than `client_credentials`
    /// refuses the request at its own `client.allows_grant_type(..)` check,
    /// before it touches a store, because a merchant registration can only
    /// ever declare `client_credentials`
    /// (`vpay_config::ConfigError::DisallowedMerchantGrant`); and
    /// `handle_client_credentials`, the one handler that does run, is not
    /// even passed the store. They are filled by
    /// [`refusing_stores`]' three fail-closed types.
    ///
    /// **Changed 2026-09-05.** Until then all three held
    /// `authkestra_op::sqlx_store::SqlxOpStore<sqlx::Postgres>` over the pool
    /// below. That type is behind `authkestra-op`'s `sqlx-postgres` feature,
    /// which pins `sqlx ^0.8`, and it was the only reverse dependency holding
    /// the whole workspace on that major — three warm slots nothing calls, in
    /// exchange for a compiler-visible constraint on every other crate. The
    /// replacement is not a stub or a double (AGENTS.md rule 1): a double
    /// pretends to succeed, and every method of these three returns `Err`
    /// naming the grant. See [`refusing_stores`] for the full argument, and
    /// `docs/status.md` for what was given up with it.
    #[must_use]
    pub fn new(
        config: &Config,
        key: LoadedSigningKey,
        repositories: Arc<dyn Repositories>,
    ) -> Self {
        // The one place `vpay_db` still hands out a raw pool, and the reason
        // it does: vpay-db's client-assertion store is a *foreign* trait
        // implementation over a pool (ADR-0010), whose queries vpay does not
        // own and cannot express as repository methods. Step 7's decision (9)
        // — see `docs/status.md`. It used to serve `SqlxOpStore` as well;
        // that consumer is gone, and this one is what keeps the exemption
        // alive.
        let pool = repositories.op_store_pool();
        let store = CompositeOpStore::new(
            YamlClientStore::new(&config.merchant_clients, repositories),
            RefusingAuthorizationCodeStore,
            RefusingRefreshTokenStore,
            RefusingDeviceCodeStore,
        )
        .with_client_assertion_store(vpay_db::client_assertion_store(pool));

        Self {
            default_scopes: default_scopes(config),
            config: OpConfig {
                issuer: issuer_for(config),
                scopes_supported: scopes_supported(config),
                // Empty, and that is the accurate answer rather than a
                // placeholder: `response_types_supported` describes what
                // `/authorize` will return, and this OP has no
                // authorization endpoint at all. RFC 8414 §2 lists the
                // field as REQUIRED, so it is published as an empty array
                // instead of omitted.
                response_types_supported: Vec::new(),
                grant_types_supported: OP_GRANT_TYPES.map(str::to_owned).to_vec(),
                // The algorithm an ID token would be signed with. `/v1`
                // issues none — `client_credentials` has no end user to
                // describe, and `handle_client_credentials` hardcodes
                // `id_token: None` — but the field is not optional on
                // `OpConfig`, and "RS256" is at least the truth about what
                // this deployment's key can sign (`LoadedSigningKey` refuses
                // anything that is not RSA). Not advertised in the discovery
                // document; see `token::discovery_document`.
                id_token_signing_alg: "RS256".to_owned(),
                // Inert: reached only by `default_handle_authorization_code`,
                // which no merchant client can ever get to (see "The store"
                // above). 60 s is RFC-003 §7's recommendation, so if a
                // future step does mount the grant the starting value is the
                // conservative one rather than something that had to be
                // noticed first.
                authorization_code_ttl_secs: 60,
                access_token_ttl_secs: ACCESS_TOKEN_TTL_SECS,
                // Inert for the same reason: there is no device
                // authorization endpoint on this surface. 600 s is the value
                // authkestra's own examples use.
                device_code_ttl_secs: 600,
                // RFC 8693 delegation, off. Nothing in vpay exchanges one
                // token for another, and `default_handle_token_exchange`
                // checks this flag *before* the per-client grant check — so
                // this is one of the few `OpConfig` fields that does gate
                // behaviour at runtime, and it gates it closed.
                token_exchange_enabled: false,
            },
            tokens: key.token_manager(),
            store: Arc::new(store),
        }
    }

    /// The `iss` claim of every token this OP mints, and the base for every
    /// endpoint URL below. See [`Self::new`] for why the same string has to
    /// reach the signer and the validator.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.config.issuer
    }

    /// `{issuer}/token` — where a merchant exchanges a `client_assertion`
    /// for an access token.
    ///
    /// Also the `aud` a merchant's assertion may carry:
    /// `authkestra_op::handlers::token::authenticate_client` accepts either
    /// this or [`Self::issuer`] (RFC 7523 §3 allows the token endpoint URL,
    /// OIDC Core §9 the issuer identifier). `sdks/rust` signs with this one.
    #[must_use]
    pub fn token_endpoint(&self) -> String {
        self.config.token_endpoint()
    }

    /// `{issuer}/jwks.json` — the **public** URL of the key set, as
    /// published in the discovery document.
    ///
    /// Not what this process's own resource-server validator fetches: a pod
    /// is not guaranteed to be able to reach its own public hostname (split
    /// DNS, an ingress that terminates elsewhere, an egress policy that
    /// forbids hairpinning), so `main` points the validator at a loopback
    /// URL on the port it actually bound. See `vpay-server`'s
    /// `loopback_jwks_url`.
    #[must_use]
    pub fn jwks_url(&self) -> String {
        self.config.jwks_url()
    }

    /// The OP configuration `handle_token` reads. `pub(crate)` because the
    /// only legitimate consumer is [`token`]'s handlers in this crate —
    /// exposing it publicly would let a caller build a *second* `OpConfig`
    /// and serve a token endpoint whose issuer disagrees with this one.
    pub(crate) fn config(&self) -> &OpConfig {
        &self.config
    }

    /// The store `handle_token` resolves clients and spends `jti`s through.
    /// `pub(crate)` for the reason given on [`Self::config`].
    pub(crate) fn store(&self) -> &dyn OpStore {
        self.store.as_ref()
    }

    /// The scope a token request from `client_id` is granted when it asks
    /// for none — RFC 6749 §3.3's "locally defined default", which the RFC
    /// requires an authorization server to have if it does not fail such a
    /// request outright.
    ///
    /// vpay's default is the client's own registered `scopes:`, space-joined.
    /// See [`default_scopes`] for why that, and not "nothing".
    pub(crate) fn default_scope_for(&self, client_id: &str) -> Option<&str> {
        self.default_scopes.get(client_id).map(String::as_str)
    }

    /// The signer `handle_token` mints with. `pub(crate)` for the reason
    /// given on [`Self::config`] — and additionally because this is the
    /// private key: a public accessor would let any crate in the workspace
    /// sign a token that `/v1` would then accept.
    pub(crate) fn tokens(&self) -> &TokenManager {
        self.tokens.as_ref()
    }
}

/// The `/v1` OP's issuer identifier, derived from a validated [`Config`].
///
/// **The one derivation of this string in the workspace**, and it is a free
/// function rather than a method because `vpay-server` needs it *before* a
/// [`MerchantOp`] can exist: the signing key is loaded — and stamped with
/// this `iss` — ahead of the database connection, so there is no OP to ask
/// yet. Every other consumer goes through [`MerchantOp::issuer`], which
/// returns what this produced.
///
/// Three parties compare this string byte for byte, and a mismatch between
/// any two of them has no symptom other than a bare 401 on every `/v1` call:
/// the signer (stamps `iss` on every token), the resource validator (pins
/// `iss`), and a merchant's SDK (signs its `client_assertion` with either
/// this or `{issuer}/token` as `aud` — `authkestra_op`'s
/// `authenticate_client` accepts both). Duplicating the `format!` in a
/// caller is exactly how those three drift apart, so callers do not get to.
///
/// The trailing slash is trimmed because [`OpConfig::issuer`]'s own contract
/// is "no trailing slash", and `https://api.example/` would otherwise yield
/// `https://api.example//v1/oauth` — a different string to every one of
/// those comparisons.
///
/// `/v1/oauth` and not something configurable: it is what `sdks/rust`
/// defaults to (`{base_url}/v1/oauth`), what `sdks/nodejs` defaults to, and
/// what `docs/flows/merchant-auth.md`'s endpoint table documents. A
/// deployment that moved it would silently break every merchant who took the
/// default.
#[must_use]
pub fn issuer_for(config: &Config) -> String {
    format!(
        "{}/v1/oauth",
        config.deployment.public_base_url.trim_end_matches('/')
    )
}

/// Every client's registered scopes, space-joined, indexed by `client_id` —
/// the default scope [`MerchantOp::default_scope_for`] hands a token request
/// that names none.
///
/// # Why there is a default at all
///
/// `authkestra_op`'s `client_credentials` handler grants exactly what was
/// asked for: no `scope` parameter means a token with **no** `scope` claim.
/// That is one of the two behaviours RFC 6749 §3.3 permits, and it is the
/// wrong one for vpay, because `/v1` now refuses a request whose token
/// carries no scope for it (`vpay_api::require_merchant_token`). Without a
/// default, every merchant that does not explicitly ask — which is every
/// caller of both SDKs' defaults, and `examples/merchant-curl`'s documented
/// `curl` — would get a token that authenticates and then authorises
/// nothing, and the diagnostic would be a `403` on a client whose
/// registration plainly lists the scope it was refused for.
///
/// So the registration is what authorises: what an operator writes in
/// `merchant_clients[].scopes` is what a token for that client carries. A
/// client that asks for a *narrower* scope still gets exactly what it asked
/// for (the request's own `scope` wins), and one that asks for something not
/// in its registration is still `invalid_scope` — this widens nothing. A
/// registration with an empty `scopes:` list gets no default and therefore
/// an unscoped token, which is the honest outcome: it was registered as
/// being allowed to do nothing.
fn default_scopes(config: &Config) -> BTreeMap<String, String> {
    config
        .merchant_clients
        .iter()
        .filter(|client| !client.scopes.is_empty())
        .map(|client| (client.client_id.clone(), client.scopes.join(" ")))
        .collect()
}

/// Every scope any configured merchant may request, deduplicated and sorted.
///
/// A union, because `OpConfig::scopes_supported` is a property of the
/// *provider* — "what could this server ever grant" — while what any one
/// caller may actually ask for is `ClientRegistration::scopes`, which
/// `handle_client_credentials` checks per request against the merchant's own
/// list. Publishing the union therefore grants nothing: a merchant asking
/// for a scope that appears here but not in their own registration is
/// refused with `invalid_scope`.
///
/// Sorted and deduplicated so the discovery document is stable across
/// restarts and across the order merchants happen to appear in YAML — a diff
/// between two fetches should mean the configuration changed, not that a
/// `HashMap` iterated differently.
fn scopes_supported(config: &Config) -> Vec<String> {
    let mut scopes: Vec<String> = config
        .merchant_clients
        .iter()
        .flat_map(|client| client.scopes.iter().cloned())
        .collect();
    scopes.sort();
    scopes.dedup();
    scopes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{config_with, lazy_repositories, merchant, signing_key};

    #[tokio::test]
    async fn the_issuer_and_endpoints_are_what_the_sdk_derives_from_a_base_url() {
        // Pinned against `sdks/rust/src/client.rs`'s own defaults —
        // `issuer = {base_url}/v1/oauth`, `token_endpoint = {issuer}/token`
        // — because a merchant that cannot guess these has to be told them
        // out of band, and a mismatch shows up only as an unexplained 401.
        let op = MerchantOp::new(
            &config_with("https://api.vpay.test", vec![]),
            signing_key(),
            lazy_repositories(),
        );

        assert_eq!(op.issuer(), "https://api.vpay.test/v1/oauth");
        assert_eq!(op.token_endpoint(), "https://api.vpay.test/v1/oauth/token");
        assert_eq!(op.jwks_url(), "https://api.vpay.test/v1/oauth/jwks.json");
    }

    /// A trailing slash on `public_base_url` must not produce a *different*
    /// issuer string — the `iss` claim, the assertion `aud` and the
    /// validator's expected issuer are compared byte for byte, so
    /// `https://x//v1/oauth` fails closed everywhere with no diagnostic.
    #[tokio::test]
    async fn a_trailing_slash_on_the_public_base_url_does_not_change_the_issuer() {
        let op = MerchantOp::new(
            &config_with("https://api.vpay.test/", vec![]),
            signing_key(),
            lazy_repositories(),
        );

        assert_eq!(op.issuer(), "https://api.vpay.test/v1/oauth");
    }

    /// `scopes_supported` needs no pool, but every other test in this module
    /// builds a `MerchantOp`, and `PgPool::connect_lazy` spawns sqlx's
    /// connection reaper — which requires a Tokio context even though it
    /// opens nothing. Kept a plain `#[test]` because this one genuinely does
    /// not need a runtime.
    #[test]
    fn scopes_supported_is_the_deduplicated_sorted_union_of_every_merchant() {
        let config = config_with(
            "https://api.vpay.test",
            vec![
                merchant("a", &["payments:write", "refunds:write"]),
                merchant("b", &["payments:write", "balance:read"]),
            ],
        );

        assert_eq!(
            scopes_supported(&config),
            vec![
                "balance:read".to_owned(),
                "payments:write".to_owned(),
                "refunds:write".to_owned()
            ]
        );
    }

    /// The default scope is the registration's own list, and a client
    /// registered for nothing gets no default.
    ///
    /// The empty case is the one worth pinning: `Some("")` would put an
    /// empty `scope` claim on the token instead of leaving the claim off,
    /// and `authkestra_op` would then check "" against the registration and
    /// answer `invalid_scope` — turning "this client may do nothing" into a
    /// failure to obtain a token at all, which is a different (and much
    /// harder to read) answer than the `403` `/v1` gives it.
    #[test]
    fn the_default_scope_is_the_clients_own_registration_and_nothing_wider() {
        let config = config_with(
            "https://api.vpay.test",
            vec![
                merchant("a", &["payments:write", "refunds:write"]),
                merchant("b", &["payments:read"]),
                merchant("nothing", &[]),
            ],
        );
        let defaults = default_scopes(&config);

        assert_eq!(
            defaults.get("a").map(String::as_str),
            Some("payments:write refunds:write"),
            "space-joined, RFC 6749 §3.3's encoding of a scope list"
        );
        assert_eq!(defaults.get("b").map(String::as_str), Some("payments:read"));
        assert_eq!(
            defaults.get("nothing"),
            None,
            "a client registered for no scope must not be handed one"
        );
        assert_eq!(
            defaults.get("never-registered"),
            None,
            "the default comes from the registration, never from the request"
        );
    }

    /// The TTL is the one number every other timing decision in this module
    /// is stated relative to. Asserted as a literal so that changing it is a
    /// deliberate edit to this test too — and against
    /// `keys::ROTATION_OVERLAP`, so a token can never outlive the window in
    /// which its signing key is still published.
    #[tokio::test]
    async fn the_access_token_ttl_fits_inside_the_key_rotation_overlap() {
        let op = MerchantOp::new(
            &config_with("https://api.vpay.test", vec![]),
            signing_key(),
            lazy_repositories(),
        );

        assert_eq!(op.config().access_token_ttl_secs, 900);
        assert!(
            i64::try_from(ACCESS_TOKEN_TTL_SECS).expect("900 fits in an i64")
                < keys::ROTATION_OVERLAP.whole_seconds(),
            "an access token must expire long before the key that signed it stops being published"
        );
    }

    /// `/v1` offers one grant and one client-authentication method. Written
    /// as literals rather than derived from the constants, so widening
    /// either is a deliberate change to this test.
    #[tokio::test]
    async fn the_advertised_grant_and_auth_method_are_the_two_adr_0010_names() {
        let op = MerchantOp::new(
            &config_with("https://api.vpay.test", vec![]),
            signing_key(),
            lazy_repositories(),
        );

        assert_eq!(
            op.config().grant_types_supported,
            vec!["client_credentials".to_owned()]
        );
        assert_eq!(OP_TOKEN_ENDPOINT_AUTH_METHOD, "private_key_jwt");
        assert!(op.config().response_types_supported.is_empty());
        assert!(
            !op.config().token_exchange_enabled,
            "token exchange gates closed at runtime, not merely in the discovery document"
        );
    }
}
