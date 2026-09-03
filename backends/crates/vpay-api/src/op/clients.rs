//! [`YamlClientStore`] — the merchant registry the `/v1` OP looks clients up
//! in: `merchant_clients` from YAML (ADR-0003, ADR-0010), minus anything an
//! operator has since disabled in `disabled_clients`.
//!
//! Two sources, one answer, and they are not symmetric. YAML is authoritative
//! for *identity* — does this client exist, what is its public JWK — and the
//! database only ever *subtracts* access (ADR-0010's Consequences section, and
//! `vpay_db::disabled_clients`'s own module docs). Nothing here can grant a
//! client the OP would not otherwise have found.
//!
//! # Why the kill switch is enforced in `find_client`, of all places
//!
//! `authkestra_op::store::OpStore` has a grant seam that a deployment can hook
//! — but `client_credentials` does not go through it. Reading the pinned
//! `authkestra-op-0.7.1/src/handlers/token.rs`: `handle_client_credentials`
//! takes the already-resolved `ClientRegistration`, checks
//! `allows_grant_type`, the requested scopes and the requested audience, and
//! then mints straight through `TokenManager` — it consults no store of any
//! kind after client resolution. The single point every token request passes
//! through, for every grant, is step 1 of `handle_token_request`:
//! `op_store.find_client(&client_id)`. So a kill switch that is not enforced
//! *here* is not enforced at all on the one grant `/v1` uses. This is why a
//! disabled client is reported as `Ok(None)` — "no such client" — rather than
//! being filtered somewhere more semantically obvious downstream.
//!
//! `Ok(None)` is also the right *shape* of answer, not merely the convenient
//! one: the OP maps it to `invalid_client` with the same generic "Client
//! authentication failed" description an unknown `client_id` gets, so the
//! token endpoint cannot be used as an oracle for whether a given merchant
//! exists but is suspended. The warn-level log line is where the distinction
//! lives, for the operator who flipped the switch.

use std::collections::HashMap;

use async_trait::async_trait;
use authkestra_op::client::{
    ClientRegistration, ClientStore, GrantType as OpGrantType, TokenEndpointAuthMethod,
};
use authkestra_op::error::OpError;
use vpay_config::oauth::{GrantType as ConfigGrantType, MerchantClient};
use vpay_db::PgPool;

/// Converts one configured merchant into the registration the OP consumes.
///
/// The mapping table in `vpay_config::oauth`'s module docs is the
/// specification for this function; it is deliberately mechanical, and the
/// three fields YAML has no say over are the interesting ones:
///
/// - `token_endpoint_auth_method: Some(PrivateKeyJwt)` — being a merchant
///   client *is* being a `private_key_jwt` client (ADR-0010). `None` here
///   would not be a subtle degradation: `authkestra-op`'s own doc comment on
///   the field says a registration carrying `None` is "**never** accepted via
///   `private_key_jwt`", so every merchant would fail authentication outright.
/// - `client_secret_hash: None` — vpay stores no merchant secret in any form,
///   and `vpay_config::ConfigError::ClientSecretPresent` refuses to boot a
///   config that tries to supply one.
/// - `redirect_uris: vec![]` — `client_credentials` has no browser leg, so
///   there is no URI to redirect to. `ClientRegistration::allows_redirect_uri`
///   is a plain exact-match over this list, so an empty one denies every
///   redirect rather than allowing any.
///
/// `grant_types` is mapped from the config's own closed enum rather than
/// hardcoded, because that enum is what
/// `vpay_config::ConfigError::DisallowedMerchantGrant` validates against —
/// hardcoding `[ClientCredentials]` here would make that boot rule
/// unobservable, which is the "validation that isn't wired into the write
/// path" failure in another costume.
#[must_use]
pub fn registration_for(client: &MerchantClient) -> ClientRegistration {
    ClientRegistration {
        client_id: client.client_id.clone(),
        client_secret_hash: None,
        redirect_uris: Vec::new(),
        grant_types: client
            .grant_types
            .iter()
            .map(|grant| match grant {
                ConfigGrantType::ClientCredentials => OpGrantType::ClientCredentials,
                ConfigGrantType::AuthorizationCode => OpGrantType::AuthorizationCode,
            })
            .collect(),
        scopes: client.scopes.clone(),
        // Deprecated at authkestra-op 0.7.0 and read by no handler (PKCE is
        // unconditional on the authorization-code grant, which a merchant
        // client never uses). Still required to construct the struct, so it
        // is set and `allow`ed rather than worked around — matching
        // `sdks/rust/tests/op_conformance.rs`, which builds the same shape.
        #[allow(deprecated)]
        require_pkce: false,
        allowed_audiences: client.allowed_audiences.clone(),
        token_endpoint_auth_method: Some(TokenEndpointAuthMethod::PrivateKeyJwt),
        jwks: client.jwks.clone(),
    }
}

/// The `/v1` OP's [`ClientStore`]: YAML for identity, `disabled_clients` for
/// revocation.
///
/// Registrations are built once, at construction, rather than per lookup —
/// they are immutable for the process's lifetime (ADR-0003: configuration
/// loads once, at boot, with no hot reload), so converting on every token
/// request would clone the same JWK set over and over for nothing.
///
/// A duplicate `client_id` cannot reach this map: `Config::validate_all`
/// rejects one across merchants *and* the dashboard client before a `Config`
/// exists (`ConfigError::DuplicateClientId`). If one somehow did, the later
/// entry would win — which is why that boot rule, not this constructor, is
/// where uniqueness is enforced.
#[derive(Debug, Clone)]
pub struct YamlClientStore {
    registrations: HashMap<String, ClientRegistration>,
    pool: PgPool,
}

impl YamlClientStore {
    /// Indexes `clients` by `client_id` and keeps `pool` for the kill-switch
    /// lookup.
    ///
    /// `PgPool` is a cheap `Arc`-backed handle (sqlx's own docs), so this
    /// opens no connection — the first query happens on the first
    /// [`ClientStore::find_client`] call that resolves a known client.
    #[must_use]
    pub fn new(clients: &[MerchantClient], pool: PgPool) -> Self {
        Self {
            registrations: clients
                .iter()
                .map(|client| (client.client_id.clone(), registration_for(client)))
                .collect(),
            pool,
        }
    }
}

#[async_trait]
impl ClientStore for YamlClientStore {
    /// Resolves `client_id` against YAML, then against the kill switch.
    ///
    /// That order is deliberate: an unknown `client_id` — the shape every
    /// credential-stuffing attempt against the token endpoint has — is
    /// answered from memory and never reaches Postgres, so an unauthenticated
    /// caller cannot turn the token endpoint into a database load amplifier.
    /// The database is consulted only for a client that YAML already
    /// recognises.
    ///
    /// # Errors
    ///
    /// Returns [`OpError::Storage`] if the `disabled_clients` lookup itself
    /// fails. This fails the token request **closed**: `handle_token_request`
    /// maps any `Err` from `find_client` to a `server_error`
    /// (`authkestra-op-0.7.1/src/handlers/token.rs`, step 1), so a database
    /// outage produces no token rather than a token for a client that may
    /// have been revoked. Mapping the failure to `Ok(None)` would render as
    /// `invalid_client` — indistinguishable from a real revocation, and it
    /// would tell an operator to look at the merchant instead of at
    /// Postgres. `OpError::UnknownClient` is wrong for the same reason and
    /// is additionally not what this trait's contract expects a *missing*
    /// client to be reported as (its own doc comment: return `Ok(None)`,
    /// "not an error"). `Storage` is the crate's own variant for exactly
    /// this — opaque by design, so a SQL error can never leak into an OAuth
    /// error response.
    async fn find_client(&self, client_id: &str) -> Result<Option<ClientRegistration>, OpError> {
        let Some(registration) = self.registrations.get(client_id) else {
            return Ok(None);
        };

        match vpay_db::is_client_disabled(&self.pool, client_id).await {
            Ok(false) => Ok(Some(registration.clone())),
            Ok(true) => {
                tracing::warn!(
                    client_id,
                    "refusing a configured merchant client: disabled via the disabled_clients \
                     kill switch"
                );
                Ok(None)
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    client_id,
                    "disabled_clients lookup failed; refusing client resolution rather than \
                     assuming the client is enabled"
                );
                Err(OpError::Storage)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use authkestra_op::client_assertion::verify_client_assertion;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
    use rsa::traits::PublicKeyParts as _;
    use serde_json::{Value, json};
    use vpay_config::MERCHANT_AUDIENCE;
    use vpay_sdk::Credentials;
    use vpay_sdk::auth::mint_client_assertion;

    use super::*;

    const CLIENT_ID: &str = "acme-cameroon";
    const ISSUER: &str = "https://api.vpay.test/v1/oauth";
    const TOKEN_ENDPOINT: &str = "https://api.vpay.test/v1/oauth/token";

    /// An RSA keypair in the two shapes these tests need: the private half as
    /// a PEM (what a merchant hands `vpay_sdk::Credentials::rsa_pem`) and the
    /// public half as a JWK (what vpay holds in YAML). Generated per call,
    /// never a hard-coded pair — a fixture keypair shared with the verifier
    /// under test would let a broken signature path still "verify".
    ///
    /// Mirrors `sdks/rust/tests/support/mod.rs::generate_key`; kept local
    /// rather than shared because a `tests/support` module in one crate is
    /// not importable from another.
    fn generate_key(kid: Option<&str>) -> (String, Value) {
        let mut rng = rand::rngs::OsRng;
        let private_key =
            rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa key generation succeeds");
        let public_key = private_key.to_public_key();
        let pem = private_key
            .to_pkcs1_pem(LineEnding::LF)
            .expect("pkcs1 pem encoding succeeds")
            .to_string();

        let mut jwk = json!({
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "n": URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be()),
        });
        // `jwk["kid"] = …` would be `clippy::indexing_slicing`, which has no
        // test exemption in `clippy.toml` and would not be exempt here in any
        // case — this is a plain helper, not a `#[test]` function.
        if let (Some(kid), Some(members)) = (kid, jwk.as_object_mut()) {
            members.insert("kid".to_owned(), json!(kid));
        }
        (pem, json!({ "keys": [jwk] }))
    }

    fn merchant(jwks: Value) -> MerchantClient {
        MerchantClient {
            client_id: CLIENT_ID.to_owned(),
            // The OP resolves a *credential*; the tenant it maps to is the
            // `/v1` boundary's business, not this store's. Deliberately not
            // equal to `client_id`, so a conversion that confused the two
            // would fail rather than pass by coincidence.
            merchant_id: format!("{CLIENT_ID}-tenant"),
            jwks: Some(jwks),
            grant_types: vec![ConfigGrantType::ClientCredentials],
            scopes: vec!["payments:write".to_owned()],
            allowed_audiences: vec![MERCHANT_AUDIENCE.to_owned()],
            client_secret: None,
            // The client store converts a registration into an OAuth
            // `ClientRegistration`; webhook endpoints are not part of that
            // conversion and must never become part of it — they are a
            // delivery destination, not an authentication fact.
            webhooks: Vec::new(),
        }
    }

    /// What `handlers::token::authenticate_client` passes as
    /// `expected_audiences`: the OP's token endpoint URL and its issuer
    /// identifier. Same shape as `sdks/rust/tests/op_conformance.rs`, so the
    /// two sides of this contract are verified against the same list.
    fn expected_audiences() -> Vec<String> {
        vec![TOKEN_ENDPOINT.to_owned(), ISSUER.to_owned()]
    }

    /// A pool pointed at a port nothing listens on, which has never opened a
    /// connection. `connect_lazy` parses the URL and returns immediately
    /// (sqlx's own docs), so a test that does not intend to reach Postgres
    /// provably does not — and one that does reach it observes a real
    /// failure rather than silently succeeding against something.
    ///
    /// `acquire_timeout` is cut to 500 ms from sqlx's 30 s default purely for
    /// suite runtime: sqlx keeps retrying a refused connection until the
    /// timeout elapses, so the default made
    /// `a_failed_kill_switch_lookup_refuses_a_known_client_rather_than_admitting_it`
    /// a 30-second test on its own. The property under test is *which* answer
    /// a failed lookup produces, which is independent of how long the failure
    /// took to arrive.
    fn lazy_pool() -> PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(500))
            .connect_lazy("postgres://vpay:vpay@127.0.0.1:1/does-not-exist")
            .expect("a syntactically valid postgres URL parses without connecting")
    }

    #[test]
    fn the_conversion_maps_every_field_the_op_reads() {
        let (_pem, jwks) = generate_key(Some("acme-cameroon-2026-08"));
        let client = merchant(jwks.clone());

        let registration = registration_for(&client);

        assert_eq!(registration.client_id, CLIENT_ID);
        assert_eq!(registration.client_secret_hash, None);
        assert_eq!(registration.redirect_uris, Vec::<String>::new());
        assert_eq!(
            registration.grant_types,
            vec![OpGrantType::ClientCredentials]
        );
        assert_eq!(registration.scopes, vec!["payments:write".to_owned()]);
        assert_eq!(
            registration.allowed_audiences,
            vec![MERCHANT_AUDIENCE.to_owned()]
        );
        assert_eq!(
            registration.token_endpoint_auth_method,
            Some(TokenEndpointAuthMethod::PrivateKeyJwt)
        );
        assert_eq!(registration.jwks, Some(jwks));
        // Deprecated and read by no handler, but still part of the struct —
        // asserted so a future flip would be a deliberate, visible change.
        #[allow(deprecated)]
        {
            assert!(!registration.require_pkce);
        }
    }

    /// The config enum is closed to two variants and `Config::validate_all`
    /// rejects the other one for a merchant — but the conversion must still
    /// map what it is given rather than substituting a constant, or that
    /// boot rule would be enforcing something the runtime ignores.
    #[test]
    fn the_conversion_maps_grants_it_is_given_rather_than_hardcoding_one() {
        let (_pem, jwks) = generate_key(None);
        let mut client = merchant(jwks);
        client.grant_types = vec![ConfigGrantType::AuthorizationCode];

        assert_eq!(
            registration_for(&client).grant_types,
            vec![OpGrantType::AuthorizationCode]
        );
    }

    /// The decisive conformance case: an assertion minted by the real
    /// merchant SDK is accepted by the real OP verifier when handed the
    /// registration this module built from YAML. Same call shape as
    /// `sdks/rust/tests/op_conformance.rs`, deliberately — that test proves
    /// the SDK's half against a hand-written registration; this one proves
    /// *this crate's* conversion is the same registration.
    #[test]
    fn an_sdk_minted_assertion_verifies_against_the_registration_this_module_builds() {
        let (pem, jwks) = generate_key(None);
        let registration = registration_for(&merchant(jwks));

        let credentials = Credentials::rsa_pem(CLIENT_ID, &pem).expect("the generated PEM parses");
        let assertion =
            mint_client_assertion(&credentials, TOKEN_ENDPOINT, Duration::from_secs(60))
                .expect("the SDK mints an assertion");

        let verified = verify_client_assertion(&assertion, &registration, &expected_audiences())
            .expect("the real OP verifier accepts an assertion minted for this registration");

        // The `jti` the OP would spend against `oauth_client_assertion_jtis`
        // is the one the SDK generated — proof the claims reached the
        // verifier intact, not merely that verification returned `Ok`.
        assert!(uuid::Uuid::parse_str(&verified.jti).is_ok(), "{verified:?}");
    }

    #[test]
    fn an_assertion_signed_by_a_key_this_merchant_did_not_register_is_refused() {
        let (_registered_pem, registered_jwks) = generate_key(None);
        let (other_pem, _other_jwks) = generate_key(None);
        let registration = registration_for(&merchant(registered_jwks));

        let credentials =
            Credentials::rsa_pem(CLIENT_ID, &other_pem).expect("the generated PEM parses");
        let assertion =
            mint_client_assertion(&credentials, TOKEN_ENDPOINT, Duration::from_secs(60))
                .expect("the SDK mints an assertion");

        assert!(
            verify_client_assertion(&assertion, &registration, &expected_audiences()).is_err(),
            "an assertion signed by an unregistered keypair must be refused"
        );
    }

    /// Proves the ordering documented on `find_client`: an unknown
    /// `client_id` is answered from the in-memory index and never touches
    /// the database. The pool here points at a port nothing listens on, so a
    /// lookup that did reach Postgres would fail rather than pass.
    #[tokio::test]
    async fn an_unknown_client_id_is_refused_without_touching_the_database() {
        let (_pem, jwks) = generate_key(None);
        let store = YamlClientStore::new(&[merchant(jwks)], lazy_pool());

        let found = store
            .find_client("someone-else")
            .await
            .expect("an unknown client_id is Ok(None), never an error");

        assert!(found.is_none());
    }

    /// The other half of the same ordering, and the more important one: a
    /// *known* client does reach the database, and when the database cannot
    /// answer, this fails closed with [`OpError::Storage`] rather than
    /// returning the registration anyway.
    ///
    /// The unreachable pool is the whole mechanism — nothing listens on
    /// 127.0.0.1:1, so the kill-switch query genuinely fails. If
    /// `find_client` were ever changed to treat a lookup error as "not
    /// disabled", this test would see `Ok(Some(_))` and fail. `matches!`
    /// rather than `assert_eq!` because `OpError` does not implement
    /// `PartialEq`.
    #[tokio::test]
    async fn a_failed_kill_switch_lookup_refuses_a_known_client_rather_than_admitting_it() {
        let (_pem, jwks) = generate_key(None);
        let store = YamlClientStore::new(&[merchant(jwks)], lazy_pool());

        let outcome = store.find_client(CLIENT_ID).await;

        assert!(
            matches!(outcome, Err(OpError::Storage)),
            "a disabled_clients lookup failure must fail the token request closed, got {outcome:?}"
        );
    }

    // `#[tokio::test]` rather than `#[test]`: `PgPool::connect_lazy` opens no
    // connection but still builds the pool's idle reaper, which requires a
    // Tokio context to exist.
    #[tokio::test]
    async fn the_store_indexes_every_configured_client_by_id() {
        let (_pem, first) = generate_key(None);
        let (_pem2, second) = generate_key(None);
        let mut other = merchant(second);
        other.client_id = "beta-merchant".to_owned();

        let store = YamlClientStore::new(&[merchant(first), other], lazy_pool());

        assert_eq!(store.registrations.len(), 2);
        assert!(store.registrations.contains_key(CLIENT_ID));
        assert!(store.registrations.contains_key("beta-merchant"));
    }
}
