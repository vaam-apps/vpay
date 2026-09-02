//! Statically registered OAuth2/OIDC clients (ADR-0010,
//! `docs/flows/dashboard-auth.md`).
//!
//! Two kinds, both loaded from YAML (ADR-0003) and both, structurally,
//! carrying no client secret:
//!
//! - [`MerchantClient`] — a merchant's `/v1` credential: `client_credentials`
//!   plus `private_key_jwt` (RFC 7523). vpay never sees a merchant's
//!   private key, only the **public** JWK set that verifies the assertion
//!   it signs.
//! - [`DashboardClient`] — the single `/dash/v1` client: authorization-code
//!   plus PKCE, a public client with no secret at all, requesting exactly
//!   one read-only scope (`docs/flows/dashboard-auth.md`'s "Scope"
//!   section).
//!
//! # Why these types, and not `authkestra_op::client::ClientRegistration`
//! # directly
//!
//! `vpay-config` deliberately does not depend on `authkestra-op` — that
//! crate, and the `ClientStore` that converts these types into a real
//! `ClientRegistration`, belong to the auth-wiring work that owns
//! `backends/crates/vpay-api/**`, not to config loading. These types are
//! shaped to make that conversion mechanical:
//!
//! | This type | `ClientRegistration` field | Fixed by client kind, not YAML |
//! |---|---|---|
//! | `MerchantClient::client_id` / `DashboardClient::client_id` | `client_id` | |
//! | `MerchantClient::jwks` | `jwks` (wrapped in `Some`) | |
//! | `MerchantClient::grant_types` | `grant_types` | |
//! | `MerchantClient::scopes` | `scopes` | |
//! | `MerchantClient::allowed_audiences` | `allowed_audiences` | |
//! | `DashboardClient::redirect_uris` | `redirect_uris` | |
//! | `DashboardClient::scope` | `scopes` (wrapped in a single-element `vec![]`) | |
//! | — | `client_secret_hash` | always `None` — see "No secret, ever" below |
//! | — | `token_endpoint_auth_method` | `PrivateKeyJwt` for merchants, `NoAuth` for the dashboard (RFC 7523 / public client) |
//! | — | `require_pkce` | always `false` for merchants (server-to-server, no browser step), always `true` for the dashboard |
//!
//! `token_endpoint_auth_method` and `require_pkce` are not YAML fields on
//! purpose: they are invariants of *being* a merchant client or *being* the
//! dashboard client, never a per-deployment choice, so there is nothing for
//! an operator to configure — or misconfigure — there. `grant_types` stays a
//! real YAML field on [`MerchantClient`] specifically because ADR-0010 needs
//! something to enforce *against*: "declares any grant other than
//! `client_credentials` is fatal" is a validation rule over a value an
//! operator could actually type, not a tautology over a hardcoded constant.
//!
//! # No secret, ever
//!
//! Both types carry a `client_secret: Option<String>` field whose only
//! legitimate value is `None`. It exists so a config that accidentally
//! carries a secret is refused at boot ([`crate::ConfigError::ClientSecretPresent`],
//! checked in `Config::validate_all`) rather than silently ignored — the
//! field would otherwise just vanish into "unknown YAML key" territory,
//! which is not the fail-fast story ADR-0003 promises. Never populate it in
//! a real config; it is a trap, not a feature.

use std::fmt;

use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// The `aud` value every `/v1` access token must carry, and therefore the one
/// value every merchant registration must list in
/// [`MerchantClient::allowed_audiences`].
///
/// **This lives here, in config, because it has to be one string in one
/// place.** Three parties have to agree on it or `/v1` silently stops
/// working: the merchant's token request (`audience=vpay:v1` — the SDKs send
/// it by default, `docs/flows/merchant-auth.md`), the OP's own
/// `allowed_audiences` gate on `handle_client_credentials`, and vpay's
/// resource-server validator (`vpay_api::resource_auth::Surface::Merchant::audience`,
/// which returns this constant rather than spelling it a second time). Two of
/// the three disagreeing produces no error message anywhere near the cause —
/// see [`crate::ConfigError::MerchantMissingV1Audience`] for the two concrete
/// shapes that failure takes and why it is fatal at boot instead.
pub const MERCHANT_AUDIENCE: &str = "vpay:v1";

/// OAuth2 grant types this workspace's clients can be registered for.
///
/// Deliberately a closed set of exactly the two grants ADR-0010 and
/// `docs/flows/dashboard-auth.md` actually use — unlike
/// `authkestra_op::client::GrantType`, there is no `Custom(String)` fallback
/// and no device-code/refresh-token/token-exchange variant, because this
/// deployment never registers a client for any of those (ADR-0010 explicitly
/// drops the device flow and refresh tokens for `/v1`; the dashboard issues
/// no refresh token either — `docs/flows/dashboard-auth.md`'s "Token
/// lifetimes" table). A grant this workspace has no use for should fail to
/// parse, not silently round-trip through a `Custom` variant.
///
/// Serializes with the same snake_case wire form Authkestra's own
/// `GrantType` uses (`"authorization_code"`, `"client_credentials"`), so a
/// future `ClientStore` conversion is a plain one-to-one match, not a
/// string-rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantType {
    /// Authorization code grant (with PKCE) — the dashboard's only grant.
    AuthorizationCode,
    /// Client credentials grant (machine-to-machine) — a merchant's only
    /// grant (ADR-0010).
    ClientCredentials,
}

/// A statically registered merchant OAuth2 client (ADR-0010): `/v1`
/// authenticates with `client_credentials` + `private_key_jwt`.
///
/// vpay never stores a merchant's private key, in any form. `jwks` is the
/// **public** half only — see the module docs' "No secret, ever" section for
/// what a plaintext `client_secret` here means (nothing good; refuse to
/// boot).
#[derive(Clone, Serialize, Deserialize, Validate)]
pub struct MerchantClient {
    /// Public client identifier. Must be unique across every merchant and
    /// dashboard client combined — checked in `Config::validate_all`, not
    /// here, since uniqueness is a whole-`Config` property no single
    /// client's own `garde` rules can see.
    #[garde(length(min = 1))]
    pub client_id: String,
    /// The merchant's public JWK Set (`{"keys": [...]}`), used to verify the
    /// `private_key_jwt` assertion this client signs (RFC 7523 §2.2).
    ///
    /// **This is not secret — publishing a public key is the point.** A
    /// literal value here is correct and expected, unlike
    /// `ProviderHost::credentials` or [`Self::client_secret`], which must
    /// never be literals in a livemode deployment. Do not copy the "literal
    /// is fine" pattern from this field onto anything that actually is
    /// secret.
    ///
    /// Kept as raw JSON rather than a typed key structure for the same
    /// reason `authkestra_op::client::ClientRegistration::jwks` is: a typed
    /// parse silently drops members it has no field for, and re-validating
    /// from scratch at every use is cheap for something this small and only
    /// ever read at boot and at token-verification time.
    ///
    /// `None`/absent and "present but with an empty `keys` array" are
    /// treated identically as fatal at boot — see
    /// [`ConfigError::EmptyMerchantJwks`](crate::ConfigError::EmptyMerchantJwks):
    /// `private_key_jwt` with no key can never authenticate, so booting with
    /// one is exactly as pointless as booting with none at all.
    #[garde(skip)]
    #[serde(default)]
    pub jwks: Option<JsonValue>,
    /// Grant types this client is permitted to use. ADR-0010: a merchant
    /// client declaring anything other than `[client_credentials]` is fatal
    /// at boot ([`ConfigError::DisallowedMerchantGrant`](crate::ConfigError::DisallowedMerchantGrant)) —
    /// the device flow and refresh tokens are dropped outright, not merely
    /// discouraged.
    #[garde(skip)]
    pub grant_types: Vec<GrantType>,
    /// Scopes this client may request.
    #[garde(skip)]
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Downstream audiences this client may target.
    #[garde(skip)]
    #[serde(default)]
    pub allowed_audiences: Vec<String>,
    /// Must never be set — see the module docs' "No secret, ever" section.
    /// `Debug` is hand-written below to redact this if it is ever
    /// populated, as defence in depth alongside the boot-time validation
    /// that refuses to start at all when it is `Some`.
    #[garde(skip)]
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Redacts [`MerchantClient::client_secret`] (which must always be `None` —
/// see the module docs) while leaving every other field visible, including
/// `jwks`: it is a **public** key set, not a credential, so there is nothing
/// to hide there. Mirrors `ProviderHost`'s hand-written `Debug` in shape,
/// not in what it redacts — the hazard here is structurally different (a
/// field that must never be populated at all, not one that is populated and
/// sensitive by design).
impl fmt::Debug for MerchantClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MerchantClient")
            .field("client_id", &self.client_id)
            .field("jwks", &self.jwks)
            .field("grant_types", &self.grant_types)
            .field("scopes", &self.scopes)
            .field("allowed_audiences", &self.allowed_audiences)
            .field(
                "client_secret",
                &self.client_secret.as_deref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

/// The dashboard's OAuth2 client registration (`docs/flows/dashboard-auth.md`):
/// authorization-code + PKCE, a public client with no secret at all, and
/// exactly one read-only scope — the dashboard observes, it does not
/// administer (ADR-0008).
#[derive(Clone, Serialize, Deserialize, Validate)]
pub struct DashboardClient {
    /// Public client identifier. Must be unique across every merchant and
    /// dashboard client combined — see [`MerchantClient::client_id`]'s doc
    /// comment for why that check lives in `Config::validate_all` instead of
    /// here.
    #[garde(length(min = 1))]
    pub client_id: String,
    /// Exact-match redirect URIs (`authkestra_op::client::ClientRegistration::allows_redirect_uri`
    /// matches these literally, no prefix/wildcard). Must be non-empty — a
    /// client that can never redirect back can never complete the
    /// authorization-code flow, so it is fatal at boot rather than a
    /// runtime 404 on first login attempt.
    #[garde(skip)]
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    /// The dashboard's single scope. A plain `String`, not a `Vec<String>`
    /// with a length-1 rule — the maintainer's decision
    /// (`docs/flows/dashboard-auth.md`'s "Scope" section: the dashboard is
    /// read-only observability and governance, so one scope is all it ever
    /// needs) is enforced by the type itself, not by a rule that could be
    /// loosened without anyone noticing the shape changed.
    #[garde(length(min = 1))]
    pub scope: String,
    /// Must never be set — see the module docs' "No secret, ever" section.
    #[garde(skip)]
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Redacts [`DashboardClient::client_secret`] — see [`MerchantClient`]'s
/// `Debug` impl for the identical reasoning.
impl fmt::Debug for DashboardClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DashboardClient")
            .field("client_id", &self.client_id)
            .field("redirect_uris", &self.redirect_uris)
            .field("scope", &self.scope)
            .field(
                "client_secret",
                &self.client_secret.as_deref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

/// True if `jwks` is present and contains at least one key — the only shape
/// [`ConfigError::EmptyMerchantJwks`](crate::ConfigError::EmptyMerchantJwks)
/// does not fire for. Anything else (missing entirely, not a JSON object,
/// an object with no `keys` array, or a `keys` array that is empty) means
/// `private_key_jwt` can never succeed for this client.
pub(crate) fn jwks_has_at_least_one_key(jwks: &Option<JsonValue>) -> bool {
    let Some(JsonValue::Object(map)) = jwks else {
        return false;
    };
    matches!(map.get("keys"), Some(JsonValue::Array(keys)) if !keys.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwks_has_at_least_one_key_rejects_missing_empty_and_keyless_shapes() {
        assert!(!jwks_has_at_least_one_key(&None));
        assert!(!jwks_has_at_least_one_key(&Some(serde_json::json!({}))));
        assert!(!jwks_has_at_least_one_key(&Some(
            serde_json::json!({"keys": []})
        )));
        assert!(!jwks_has_at_least_one_key(&Some(serde_json::json!(
            {"not_keys": [1]}
        ))));
        assert!(!jwks_has_at_least_one_key(&Some(serde_json::json!(
            "not-even-an-object"
        ))));
    }

    #[test]
    fn jwks_has_at_least_one_key_accepts_a_populated_set() {
        assert!(jwks_has_at_least_one_key(&Some(serde_json::json!({
            "keys": [{"kty": "RSA", "kid": "k1", "n": "...", "e": "AQAB"}]
        }))));
    }

    #[test]
    fn grant_type_uses_authkestras_snake_case_wire_form() {
        assert_eq!(
            serde_json::to_string(&GrantType::ClientCredentials).unwrap(),
            "\"client_credentials\""
        );
        assert_eq!(
            serde_json::to_string(&GrantType::AuthorizationCode).unwrap(),
            "\"authorization_code\""
        );
    }

    #[test]
    fn merchant_client_debug_output_never_contains_a_client_secret_value() {
        let client = MerchantClient {
            client_id: "acme".to_owned(),
            jwks: Some(serde_json::json!({"keys": [{"kid": "k1"}]})),
            grant_types: vec![GrantType::ClientCredentials],
            scopes: vec!["payments:write".to_owned()],
            allowed_audiences: vec!["vpay".to_owned()],
            client_secret: Some("this-should-never-be-here".to_owned()),
        };

        let formatted = format!("{client:?}");

        assert!(
            !formatted.contains("this-should-never-be-here"),
            "{formatted}"
        );
        assert!(formatted.contains("[redacted]"), "{formatted}");
        // Non-secret fields, including the (public) jwks, stay visible.
        assert!(formatted.contains("acme"), "{formatted}");
        assert!(formatted.contains("k1"), "{formatted}");
        assert!(formatted.contains("payments:write"), "{formatted}");
    }

    #[test]
    fn dashboard_client_debug_output_never_contains_a_client_secret_value() {
        let client = DashboardClient {
            client_id: "vpay-dashboard".to_owned(),
            redirect_uris: vec!["https://dashboard.example/callback".to_owned()],
            scope: "dashboard:read".to_owned(),
            client_secret: Some("this-should-never-be-here".to_owned()),
        };

        let formatted = format!("{client:?}");

        assert!(
            !formatted.contains("this-should-never-be-here"),
            "{formatted}"
        );
        assert!(formatted.contains("[redacted]"), "{formatted}");
        assert!(formatted.contains("vpay-dashboard"), "{formatted}");
        assert!(formatted.contains("dashboard.example"), "{formatted}");
        assert!(formatted.contains("dashboard:read"), "{formatted}");
    }
}
