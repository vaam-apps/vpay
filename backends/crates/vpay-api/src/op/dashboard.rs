//! The `/dash/v1` half of vpay's OP: the authorization-code grant, served for
//! the dashboard client and for nothing else
//! ([ADR-0017](../../../../../docs/adr/0017-staff-authentication.md)
//! decision 3).
//!
//! # One issuer, two surfaces
//!
//! [`DashboardOp`] does **not** get an issuer of its own. It carries
//! [`crate::op::issuer_for`]'s string — the same `{public_base_url}/v1/oauth`
//! every merchant token is stamped with — because vpay runs *one* OP with
//! one signing key and one JWKS, and the audience is what separates the two
//! surfaces (`crate::resource_auth::Surface`). Giving this half a second
//! issuer would mean a second JWKS URL, a second validator configuration and
//! two `iss` strings that could drift, in exchange for nothing: a token's
//! surface is already unambiguous from its `aud`.
//!
//! # What is reused from `authkestra-op`, and what is not
//!
//! `handle_authorize` is used as it stands. It is worth having rather than
//! re-implementing: it does the client lookup, the **exact** redirect-URI
//! match, the per-scope registration check (authkestra#278), the
//! `response_type` check, the unconditional PKCE requirement (authkestra#273,
//! OAuth 2.1 §4.1) and the error-redirect encoding, and every one of those is
//! a check vpay would otherwise own a second copy of.
//!
//! `handle_token`'s dispatch is **not** used, and this is the one place vpay
//! writes a grant itself. The reason is the merchant claim:
//! `default_handle_authorization_code` mints with
//! `issue_user_token_with_extra(identity, …, Some(client_id), extra)` where
//! `extra` carries only a DPoP `cnf`, and `/dash/v1`'s boundary needs
//! `vpay_config::DASHBOARD_MERCHANT_CLAIM` on the token. `OpStore` does offer
//! a seam for exactly this (`handle_authorization_code_grant`, whose own doc
//! says it exists because "`issue_user_token_with_extra` exists precisely for
//! this, but the built-in handler had no way to reach it"), and taking it
//! would still mean re-writing every check the default performs, because the
//! default's last step is the mint. So the checks are written here, each with
//! its own test and its own mutation, and `crate::staff::oauth` is the
//! handler that runs them.
//!
//! This module is therefore the *assembly*; the checks live beside the route
//! that performs them.

use std::sync::Arc;

use async_trait::async_trait;
use authkestra_engine::auth::state::Identity;
use authkestra_engine::token::TokenManager;
use authkestra_op::client::{ClientRegistration, ClientStore, GrantType};
use authkestra_op::code::{AuthorizationCode, AuthorizationCodeStore};
use authkestra_op::config::OpConfig;
use authkestra_op::error::OpError;
use authkestra_op::store::CompositeOpStore;
use time::OffsetDateTime;
use vpay_config::{Config, DashboardClient};
use vpay_db::{AuthorizationCodeRow, NewAuthorizationCode, Repositories};

use crate::op::keys::LoadedSigningKey;
use crate::op::refusing_stores::{RefusingDeviceCodeStore, RefusingRefreshTokenStore};
use crate::staff_auth::tokens;

/// How long an authorization code may be exchanged for.
///
/// Sixty seconds — RFC 6749 §4.1.2's own recommendation ("a maximum
/// authorization code lifetime of 10 minutes is RECOMMENDED", with the
/// tighter guidance in OAuth 2.1 and in authkestra's RFC-003 §7 of ≤60 s).
/// The exchange here is machine-to-machine and happens in the same request
/// the redirect arrived on, so there is no human latency to accommodate: the
/// only thing a longer window buys is a longer replay surface for a code that
/// leaked through a log or a `Referer`.
///
/// It is the same value `crate::op::MerchantOp` already sets on its own
/// (inert) `OpConfig`, and deliberately so — two OP halves with two code
/// lifetimes would be two answers to one question.
pub const AUTHORIZATION_CODE_TTL_SECS: i64 = 60;

/// The `provider_id` every staff [`Identity`] carries.
///
/// `vpay-staff` and not `local`: `provider_id` is what would tell two
/// identity sources apart if vpay ever federated the human step to an
/// external IdP (which ADR-0009 lists as an option nobody took), and a token
/// minted today should already say which source vouched for it rather than
/// being retrofitted.
pub const IDENTITY_PROVIDER: &str = "vpay-staff";

/// The [`Identity::attributes`] key carrying the session that authorised a
/// code, from `/authorize` down to [`PgAuthorizationCodeStore::store_code`].
///
/// It travels on the `Identity` because that is the only field
/// `authkestra_op::handle_authorize` passes through to the store, and it is
/// **stripped before the token is minted** — `crate::staff::oauth` builds a
/// fresh `Identity` from the consumed row rather than re-using the stored
/// one. `authkestra_engine::token::Claims` serialises `identity` whole, so
/// an attribute left on would be a session digest published inside a bearer
/// token that a browser's own server holds.
pub const ATTRIBUTE_SESSION: &str = "vpay_session";

/// The [`Identity::attributes`] key carrying the tenant, travelling the same
/// path as [`ATTRIBUTE_SESSION`] and stripped at the same point.
pub const ATTRIBUTE_MERCHANT: &str = "vpay_merchant";

/// The assembled dashboard grant: one config, one registration, one store,
/// one signer.
///
/// Deliberately not `Clone`, for [`crate::op::MerchantOp`]'s reason: the
/// signer is the process's private key and the config carries the issuer that
/// three parties compare byte for byte. Shared as `Arc<DashboardOp>`.
pub struct DashboardOp {
    config: OpConfig,
    registration: ClientRegistration,
    binding: DashboardClient,
    store: CompositeOpStore<
        DashboardClientStore,
        PgAuthorizationCodeStore,
        RefusingRefreshTokenStore,
        RefusingDeviceCodeStore,
    >,
    tokens: Arc<TokenManager>,
}

/// Shows the configuration and the registration — both public metadata — and
/// neither the store (which holds a pool) nor the signer (which holds the
/// private key).
impl std::fmt::Debug for DashboardOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashboardOp")
            .field("config", &self.config)
            .field("client_id", &self.registration.client_id)
            .finish_non_exhaustive()
    }
}

impl DashboardOp {
    /// Assembles the grant from a validated [`Config`], the process's signing
    /// key and the database.
    ///
    /// `key` is borrowed rather than moved, unlike [`crate::op::MerchantOp::new`]'s:
    /// both halves of one OP sign with the same key, and
    /// [`LoadedSigningKey::token_manager`] hands out an `Arc` of the same
    /// `TokenManager` either way, so there is one signer in the process
    /// whichever order the two are built in.
    #[must_use]
    pub fn new(
        config: &Config,
        dashboard: &DashboardClient,
        key: &LoadedSigningKey,
        repositories: Arc<dyn Repositories>,
    ) -> Self {
        let registration = registration_for(dashboard);
        Self {
            config: OpConfig {
                issuer: crate::op::issuer_for(config),
                scopes_supported: vec![dashboard.scope.clone()],
                response_types_supported: vec!["code".to_owned()],
                grant_types_supported: vec!["authorization_code".to_owned()],
                id_token_signing_alg: "RS256".to_owned(),
                authorization_code_ttl_secs: AUTHORIZATION_CODE_TTL_SECS,
                access_token_ttl_secs: crate::op::ACCESS_TOKEN_TTL_SECS,
                // No device authorization endpoint on this surface either.
                device_code_ttl_secs: 600,
                // RFC 8693 delegation stays off on both halves.
                token_exchange_enabled: false,
            },
            store: CompositeOpStore::new(
                DashboardClientStore {
                    registration: registration.clone(),
                },
                PgAuthorizationCodeStore { repositories },
                RefusingRefreshTokenStore,
                RefusingDeviceCodeStore,
            ),
            registration,
            binding: dashboard.clone(),
            tokens: key.token_manager(),
        }
    }

    /// The OP configuration `handle_authorize` reads.
    pub(crate) fn config(&self) -> &OpConfig {
        &self.config
    }

    /// The store `handle_authorize` resolves the client and stores the code
    /// through.
    pub(crate) fn store(&self) -> &dyn authkestra_op::OpStore {
        &self.store
    }

    /// The signer the token handler mints with. `pub(crate)` for
    /// [`crate::op::MerchantOp::tokens`]'s reason — this is the private key.
    pub(crate) fn tokens(&self) -> &TokenManager {
        self.tokens.as_ref()
    }

    /// The registration, as YAML declared it.
    pub(crate) fn registration(&self) -> &ClientRegistration {
        &self.registration
    }

    /// The deployment's dashboard binding: the client id, the tenant, the
    /// scope and the redirect URIs.
    pub(crate) fn binding(&self) -> &DashboardClient {
        &self.binding
    }
}

/// The `ClientRegistration` `handle_authorize` checks a request against.
///
/// Built from YAML exactly as [`crate::op::clients::registration_for`] builds
/// a merchant's, and with the three fields that decide this grant:
///
/// * `grant_types` is `[AuthorizationCode]` and nothing else. `/v1`'s
///   registrations are `[ClientCredentials]` and nothing else. Neither can
///   use the other's grant, and `handle_authorize`'s step 5 is what refuses.
/// * `client_secret_hash` is `None` — a **public** client. PKCE is what
///   authenticates the exchange, and `vpay_config::ConfigError::ClientSecretPresent`
///   refuses a deployment that tries to give the dashboard a secret.
/// * `scopes` is the registration's single scope, so `handle_authorize`'s
///   step 3 refuses a request asking for anything else.
fn registration_for(dashboard: &DashboardClient) -> ClientRegistration {
    ClientRegistration {
        client_id: dashboard.client_id.clone(),
        // Never `Some`. See the doc above, and the boot refusal.
        client_secret_hash: None,
        redirect_uris: dashboard.redirect_uris.clone(),
        grant_types: vec![GrantType::AuthorizationCode],
        scopes: vec![dashboard.scope.clone()],
        // Deprecated upstream and no longer consulted: PKCE is unconditional
        // for every client on this grant (authkestra#273). Set `true` anyway
        // so a downgrade of the crate cannot read `false` as "PKCE optional".
        #[allow(deprecated)]
        require_pkce: true,
        // Empty, and it must stay empty: `allowed_audiences` gates a
        // *requested* audience on `client_credentials` and on token
        // exchange, neither of which this client may use. The audience of a
        // token from this grant is the client id itself and comes from the
        // handler, not from this list.
        allowed_audiences: Vec::new(),
        // `None` — a public client presents no credential at `/token`. The
        // exchange is authenticated by the PKCE verifier.
        token_endpoint_auth_method: None,
        // No key: nothing signs a `client_assertion` on this surface.
        jwks: None,
    }
}

/// The one-registration `ClientStore` the dashboard grant resolves through.
///
/// A struct rather than `crate::op::clients::YamlClientStore`, and the
/// difference is the whole boundary: that store is built from
/// `merchant_clients` and consults the `disabled_clients` kill switch; this
/// one knows exactly one client and can never answer with a merchant's
/// registration. A `/dash/v1/oauth/authorize` naming a merchant client id
/// therefore gets `UnknownClient`, which is the same answer it would get for
/// a client id nobody registered.
///
/// **No kill switch, and that is a gap rather than a decision.**
/// `disabled_clients` revokes a *merchant* credential; there is nothing that
/// disables the dashboard client short of removing it from YAML and
/// restarting. What can be disabled per person is a `staff_members` row
/// (`staff_members.status`), which is the granularity that matters here. Recorded in
/// `docs/status.md`.
struct DashboardClientStore {
    registration: ClientRegistration,
}

#[async_trait]
impl ClientStore for DashboardClientStore {
    async fn find_client(&self, client_id: &str) -> Result<Option<ClientRegistration>, OpError> {
        Ok((client_id == self.registration.client_id).then(|| self.registration.clone()))
    }
}

/// `oauth_authorization_codes`, as an `AuthorizationCodeStore`.
///
/// Only [`AuthorizationCodeStore::store_code`] is reached in production:
/// `handle_authorize` calls it, and vpay's own token handler consumes through
/// `vpay_db::AuthorizationCodes::consume_code` rather than through this trait
/// (see this module's header for why the exchange is written here).
/// [`AuthorizationCodeStore::consume_code`] is implemented anyway, against
/// the same compare-and-swap, because a trait method that refused would be a
/// half-implementation waiting to be reached by an upstream change — and it
/// is exercised by this module's own tests.
struct PgAuthorizationCodeStore {
    repositories: Arc<dyn Repositories>,
}

#[async_trait]
impl AuthorizationCodeStore for PgAuthorizationCodeStore {
    /// Persists a code, hashing it on the way in.
    ///
    /// The two columns `authkestra_op::AuthorizationCode` has nowhere to put
    /// — the session and the tenant — travel on
    /// [`Identity::attributes`] and are lifted out here. A code whose
    /// identity carries neither is refused rather than stored with a blank
    /// tenant: it can only mean `/authorize` was reached by a path that did
    /// not establish a session, and a code with no tenant is a token with no
    /// tenant.
    async fn store_code(&self, code: AuthorizationCode) -> Result<(), OpError> {
        let (Some(session_id), Some(merchant_id)) = (
            code.identity.attributes.get(ATTRIBUTE_SESSION),
            code.identity.attributes.get(ATTRIBUTE_MERCHANT),
        ) else {
            tracing::error!(
                client_id = %code.client_id,
                "an authorization code reached the store with no session or tenant attribute; \
                 refusing to persist a code that could only mint a token naming no merchant"
            );
            return Err(OpError::Storage);
        };

        let expires_at = OffsetDateTime::from_unix_timestamp(code.expires_at.timestamp())
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);

        self.repositories
            .store_code(NewAuthorizationCode {
                code_hash: tokens::digest(&code.code),
                row: AuthorizationCodeRow {
                    client_id: code.client_id.clone(),
                    staff_id: code.identity.external_id.clone(),
                    session_id: session_id.clone(),
                    merchant_id: merchant_id.clone(),
                    scope: code.scope.clone(),
                    // `unwrap_or_default` on neither: `handle_authorize`
                    // refuses a request with no challenge (step 6) and
                    // refuses a method that is not `S256`, so both are
                    // present by the time this runs — and migration 0035's
                    // `NOT NULL` plus `method_is_s256` refuse a writer that
                    // reached here another way.
                    code_challenge: code.code_challenge.clone().unwrap_or_default(),
                    code_challenge_method: code.code_challenge_method.clone().unwrap_or_default(),
                    redirect_uri: code.redirect_uri.clone(),
                    nonce: code.nonce.clone(),
                    expires_at,
                },
                now: OffsetDateTime::now_utc(),
            })
            .await
            .map_err(|error| {
                tracing::error!(%error, "storing a dashboard authorization code");
                OpError::Storage
            })
    }

    /// Spends a code, atomically. See
    /// `vpay_db::AuthorizationCodes::consume_code` for why the read that
    /// precedes the swap is not a TOCTOU.
    async fn consume_code(&self, code: &str) -> Result<Option<AuthorizationCode>, OpError> {
        let row = self
            .repositories
            .consume_code(&tokens::digest(code), OffsetDateTime::now_utc())
            .await
            .map_err(|error| {
                tracing::error!(%error, "consuming a dashboard authorization code");
                OpError::Storage
            })?;

        Ok(row.map(|row| authorization_code_from_row(code, &row)))
    }
}

/// A stored row, back in `authkestra-op`'s shape.
///
/// `used: false`, always, and that is correct rather than a shortcut: the
/// value is reconstructed **after** the compare-and-swap has already spent
/// the row, so this describes the code as it was at the instant it was
/// legitimately consumed. A `true` here would make `handle_token`'s own
/// bookkeeping refuse the exchange it just won.
pub(crate) fn authorization_code_from_row(
    code: &str,
    row: &AuthorizationCodeRow,
) -> AuthorizationCode {
    let mut reconstructed = AuthorizationCode::new(
        code.to_owned(),
        row.client_id.clone(),
        row.redirect_uri.clone(),
        row.scope.clone(),
        identity_for(&row.staff_id, &row.session_id, &row.merchant_id),
        chrono::DateTime::from_timestamp(row.expires_at.unix_timestamp(), 0)
            .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC),
        false,
    );
    reconstructed.code_challenge = Some(row.code_challenge.clone());
    reconstructed.code_challenge_method = Some(row.code_challenge_method.clone());
    reconstructed.nonce = row.nonce.clone();
    reconstructed
}

/// The `Identity` that travels from `/authorize` into the code store.
///
/// **Not the one that reaches a token.** `crate::staff::oauth` builds a
/// second, attribute-free identity for the mint — see [`ATTRIBUTE_SESSION`]
/// for why an attribute left on would end up published inside a bearer token.
#[must_use]
pub fn identity_for(staff_id: &str, session_id: &str, merchant_id: &str) -> Identity {
    Identity {
        provider_id: IDENTITY_PROVIDER.to_owned(),
        external_id: staff_id.to_owned(),
        // Neither is carried, and both are omissions rather than gaps. The
        // email is a staff member's personal data and the display name is
        // their name; `Claims::identity` is serialised whole into every
        // access token, so putting either here would publish it to anything
        // that can read a token — including a log line that records one.
        email: None,
        username: None,
        attributes: [
            (ATTRIBUTE_SESSION.to_owned(), session_id.to_owned()),
            (ATTRIBUTE_MERCHANT.to_owned(), merchant_id.to_owned()),
        ]
        .into_iter()
        .collect(),
    }
}

/// The `Identity` a minted token carries: the staff id, and nothing else.
///
/// The counterpart to [`identity_for`], and the two exist as a pair so that
/// "what is in the code" and "what is in the token" are two answers a reader
/// can compare rather than one value that quietly grows.
#[must_use]
pub fn token_identity_for(staff_id: &str) -> Identity {
    Identity {
        provider_id: IDENTITY_PROVIDER.to_owned(),
        external_id: staff_id.to_owned(),
        email: None,
        username: None,
        attributes: std::collections::HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dashboard() -> DashboardClient {
        DashboardClient {
            client_id: "vpay-dashboard".to_owned(),
            merchant_id: "acme-tenant".to_owned(),
            scope: "dashboard:read".to_owned(),
            redirect_uris: vec!["https://dash.example.test/api/auth/callback".to_owned()],
            client_secret: None,
        }
    }

    /// The registration is a public client on one grant with one scope.
    /// Every one of the four is what refuses something: a secret would make
    /// this a confidential client, a second grant would let the dashboard
    /// mint without a browser, a second scope would widen what a token
    /// carries, and a redirect URI is matched byte for byte.
    #[test]
    fn the_registration_is_a_public_client_on_one_grant_with_one_scope() {
        let registration = registration_for(&dashboard());

        assert_eq!(registration.client_id, "vpay-dashboard");
        assert!(
            registration.client_secret_hash.is_none(),
            "the dashboard is a public client; PKCE authenticates the exchange"
        );
        assert_eq!(registration.grant_types, vec![GrantType::AuthorizationCode]);
        assert!(
            !registration
                .grant_types
                .contains(&GrantType::ClientCredentials),
            "a dashboard client that could use client_credentials would be a machine \
             credential for a staff surface"
        );
        assert_eq!(registration.scopes, vec!["dashboard:read".to_owned()]);
        assert_eq!(
            registration.redirect_uris,
            vec!["https://dash.example.test/api/auth/callback".to_owned()]
        );
        assert!(
            registration.allowed_audiences.is_empty(),
            "an allowed_audiences entry here would gate a requested audience on a grant this \
             client may not use, and would read as though the audience came from a list"
        );
        assert!(registration.token_endpoint_auth_method.is_none());
        assert!(registration.jwks.is_none());
    }

    /// The store answers for its own client and for nothing else — including
    /// for a merchant client id, which is the case that matters.
    #[tokio::test]
    async fn the_client_store_knows_exactly_one_client() {
        let store = DashboardClientStore {
            registration: registration_for(&dashboard()),
        };

        assert!(
            store
                .find_client("vpay-dashboard")
                .await
                .expect("the lookup is infallible")
                .is_some()
        );
        for other in ["acme-cameroon", "", "vpay-dashboard "] {
            assert!(
                store
                    .find_client(other)
                    .await
                    .expect("the lookup is infallible")
                    .is_none(),
                "the dashboard client store answered for {other:?}"
            );
        }
    }

    /// The identity in the code carries the two attributes the store needs;
    /// the identity in the token carries neither, and no personal data.
    ///
    /// The decisive mutation is `token_identity_for` returning
    /// `identity_for(...)`: `Claims::identity` is serialised whole, so that
    /// one-line change publishes a session digest inside every access token.
    #[test]
    fn the_token_identity_carries_no_attributes_and_no_personal_data() {
        let in_code = identity_for("stf_1", &"a".repeat(64), "acme-tenant");
        assert_eq!(
            in_code
                .attributes
                .get(ATTRIBUTE_SESSION)
                .map(String::as_str),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(
            in_code
                .attributes
                .get(ATTRIBUTE_MERCHANT)
                .map(String::as_str),
            Some("acme-tenant")
        );

        let in_token = token_identity_for("stf_1");
        assert_eq!(in_token.external_id, "stf_1");
        assert!(
            in_token.attributes.is_empty(),
            "Claims::identity is serialised whole into the access token: an attribute here is \
             published to anything that can read one"
        );
        assert!(in_token.email.is_none(), "a staff email is personal data");
        assert!(in_token.username.is_none());
        assert_eq!(in_token.provider_id, IDENTITY_PROVIDER);
    }

    /// A row round-trips into the shape the exchange's checks read, with the
    /// PKCE pair present — which is what `default_handle_authorization_code`
    /// refuses a code without.
    #[test]
    fn a_stored_row_reconstructs_with_its_pkce_pair() {
        let row = AuthorizationCodeRow {
            client_id: "vpay-dashboard".to_owned(),
            staff_id: "stf_1".to_owned(),
            session_id: "a".repeat(64),
            merchant_id: "acme-tenant".to_owned(),
            scope: "dashboard:read".to_owned(),
            code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_owned(),
            code_challenge_method: "S256".to_owned(),
            redirect_uri: "https://dash.example.test/api/auth/callback".to_owned(),
            nonce: Some("n-0S6".to_owned()),
            expires_at: OffsetDateTime::UNIX_EPOCH,
        };

        let code = authorization_code_from_row("the-code", &row);
        assert_eq!(code.code, "the-code");
        assert_eq!(code.client_id, "vpay-dashboard");
        assert_eq!(code.identity.external_id, "stf_1");
        assert_eq!(
            code.code_challenge.as_deref(),
            Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        );
        assert_eq!(code.code_challenge_method.as_deref(), Some("S256"));
        assert!(
            !code.used,
            "the row is reconstructed AFTER the swap spent it; `true` here would make the \
             exchange refuse the code it just legitimately won"
        );
    }
}
