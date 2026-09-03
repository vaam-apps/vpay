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

/// One webhook endpoint a merchant has asked vpay to POST events to
/// (`docs/flows/webhooks.md`).
///
/// # Why endpoints are YAML and not a `/v1` resource
///
/// ADR-0003 puts administration in git, and ADR-0008 puts anything the
/// dashboard cannot administer into YAML; `docs/flows/configuration.md`
/// already lists "webhook endpoints" among the values safe to mutate there.
/// Nothing in this repository has ever proposed a `/v1/webhook_endpoints`
/// resource, and a merchant who could re-point their own endpoint at boot
/// speed would be a merchant who could re-point it *without* the review
/// their onboarding is built on (ADR-0010's "merchant onboarding is a PR,
/// not a self-serve flow").
///
/// # There is no `webhook_endpoints` table either
///
/// `webhook_deliveries.endpoint_id` (migration 0022) references no table on
/// purpose: it stores [`Self::id`] verbatim so an operator who fixes a
/// typo'd URL does not orphan the delivery history. That is what makes the
/// uniqueness rule below load-bearing rather than cosmetic — two endpoints
/// sharing an id within one merchant collide on
/// `webhook_deliveries_event_endpoint`, and exactly one of them would ever
/// be delivered to.
#[derive(Clone, Serialize, Deserialize)]
pub struct WebhookEndpoint {
    /// The operator-authored name for this endpoint, stored on every
    /// delivery row and read back in runbooks.
    ///
    /// Required and unique **within one merchant**
    /// ([`ConfigError::DuplicateWebhookEndpointId`](crate::ConfigError::DuplicateWebhookEndpointId)),
    /// deliberately *not* unique across merchants: the delivery index is
    /// `(event_id, endpoint_id)` and events are already merchant-scoped, so
    /// two merchants both calling their endpoint `primary` is the normal
    /// case rather than a collision.
    ///
    /// Not a hash of [`Self::url`]: a hash changes when the URL is
    /// corrected, so the delivery history of the endpoint an operator is
    /// looking at would silently split in two — and a hash is unreadable in
    /// the runbook that has to name it.
    pub id: String,
    /// The absolute URL to POST the signed event body to.
    ///
    /// Validated at boot by `Config::validate_all`: in **either** deployment
    /// it must parse, name a host and carry no userinfo, and under `livemode`
    /// [`crate::validate_webhook_url`] additionally requires the `https`
    /// scheme and refuses a stub marker in the *host*.
    /// **That is the only URL validation there is** — there is no runtime
    /// private/link-local filtering, so a livemode operator who writes
    /// `https://169.254.169.254/…` gets exactly that
    /// (`docs/plans/2026-09-03-step5-webhooks.md`, decision 4: a
    /// resolve-then-connect check is TOCTOU unless reqwest is given a custom
    /// connector, so the honest options were "nothing" or "a connector", and
    /// the second is out of scope).
    pub url: String,
    /// The HMAC-SHA256 signing secrets, in configuration order, one per
    /// `v1=` in the `Vpay-Signature` header.
    ///
    /// One or two ([`ConfigError::WebhookSecretCount`](crate::ConfigError::WebhookSecretCount)):
    /// one normally, two only while a rotation is in flight. A third is
    /// refused rather than accepted-and-ignored because every extra secret
    /// is another `v1=` on every delivery, and an endpoint that never
    /// finished a rotation is a secret nobody has revoked.
    ///
    /// **Covered by the livemode literal-secret rule.** Unlike
    /// [`MerchantClient::jwks`], which is public key material and correct as
    /// a literal, one of these is enough to *forge* a
    /// `payment_intent.succeeded` a merchant's handler will believe — so a
    /// livemode config must write each one as a `${VAR}` placeholder
    /// ([`ConfigError::LiteralSecret`](crate::ConfigError::LiteralSecret)),
    /// checked against the pre-resolution text of the file exactly as
    /// `providers[].credentials` is.
    pub secrets: Vec<String>,
}

/// Redacts [`WebhookEndpoint::secrets`] down to a count, keeping the id and
/// the URL visible.
///
/// The endpoint table is held for a process's whole life — by the server's
/// `AppState` and by the worker's job loop — so it lands in any `{:?}` of
/// either: a `tracing` field, an operator's debug print, a panic message. A
/// webhook secret in a log is a forged webhook. The *count* stays because
/// "is this endpoint mid-rotation?" is a question a runbook asks and
/// answering it needs no secret; the id and URL stay because they are
/// already in the delivery rows and in the merchant's own configuration, and
/// an operator asking "why did this merchant get no webhook?" needs to see
/// what vpay actually loaded.
impl fmt::Debug for WebhookEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookEndpoint")
            .field("id", &self.id)
            .field("url", &self.url)
            .field(
                "secrets",
                &format_args!("[{} redacted]", self.secrets.len()),
            )
            .finish()
    }
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
    /// The tenant every object this client creates belongs to —
    /// `payment_intents.merchant_id`, and the value every `/v1` query is
    /// filtered by.
    ///
    /// **Separate from [`Self::client_id`] because they are different
    /// things.** A `client_id` names a *credential*: it is what a token's
    /// `sub` carries, it is what an operator disables through
    /// `disabled_clients`, and it is expected to be rotated or duplicated
    /// when a merchant re-keys. A `merchant_id` names the *tenant* whose
    /// rows those tokens may touch, and it must outlive any one credential —
    /// rotating a key must not orphan a merchant's payment intents. Deriving
    /// one from the other would tie them together permanently and make that
    /// rotation impossible without a data migration.
    ///
    /// Required, not defaulted to `client_id`: a default would mean a config
    /// that forgot this field still boots and silently invents a tenancy
    /// boundary, which is the one property `/v1` has no second line of
    /// defence for (there is no `merchants` table and therefore no foreign
    /// key — see `backends/migrations/0003_create-payment-intents.sql`).
    ///
    /// Unique across `merchant_clients`
    /// ([`ConfigError::DuplicateMerchantId`](crate::ConfigError::DuplicateMerchantId),
    /// checked in `Config::validate_all`): the `/v1` boundary resolves a
    /// token's `client_id` to exactly one merchant, and two clients sharing
    /// a tenant would be two credentials that can read each other's objects.
    /// That may become a legitimate shape (one merchant, several keys) — it
    /// is refused today because nothing in vpay yet models what a merchant
    /// *is* independently of its credential, so the resemblance would be a
    /// guess rather than a decision.
    #[garde(length(min = 1, max = 128))]
    pub merchant_id: String,
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
    /// Scopes this client may request — **and**, since it is what a token
    /// carries when the client requests none, what it is actually authorised
    /// to do.
    ///
    /// The vocabulary is `vpay_api::v1`'s `SCOPE_PAYMENTS_WRITE`
    /// (`payments:write`) and `SCOPE_PAYMENTS_READ` (`payments:read`); those
    /// constants are the only definition, and `/v1` refuses a request whose
    /// token carries neither of the ones its method needs
    /// (`vpay_api::v1::required_scopes`). Not validated here, deliberately:
    /// this crate does not depend on `vpay-api` (see this module's docs on
    /// why it depends on neither it nor `authkestra-op`), and a *copy* of the
    /// vocabulary in a validator here would be a second definition able to
    /// disagree with the one that decides requests.
    ///
    /// An empty list is legal and means what it says: the client can obtain
    /// a token, and every `/v1` request it makes with that token is `403`.
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
    /// Where this merchant's events are POSTed, and what each delivery is
    /// signed with (`docs/flows/webhooks.md`). See [`WebhookEndpoint`].
    ///
    /// Defaults to empty, and an empty list is a *complete* answer rather
    /// than a missing one: a merchant who has configured no endpoint still
    /// has their events fanned out to nothing and marked `done`, because
    /// leaving them `pending` for an endpoint that might appear later grows
    /// `events_pending_idx` without bound (`vpay_worker::webhooks`).
    ///
    /// `#[garde(skip)]`, unlike every other structural rule on this type:
    /// each rule these endpoints have to satisfy names *which* merchant and
    /// *which* endpoint is wrong, and a `garde` report says only that
    /// `merchant_clients[3].webhooks[1].id` failed `length(min = 1)`. The
    /// rules therefore live in `Config::validate_all`, beside the
    /// uniqueness check that no per-value derive could express anyway.
    #[garde(skip)]
    #[serde(default)]
    pub webhooks: Vec<WebhookEndpoint>,
    /// The `pk_test_…`/`pk_live_…` keys a payer's browser presents on
    /// `/v1/browser` (Step 5c, `docs/plans/2026-09-03-step5c-stripejs.md`
    /// D1), alongside the intent's own `client_secret`.
    ///
    /// **Not secret, and on the same footing as [`Self::jwks`]** — it is
    /// rendered into a merchant's own public checkout page by construction,
    /// so a literal here is correct and it prints in `Debug`. It authorises
    /// nothing on its own: it names a *tenant*, and the credential that
    /// authorises the request is the per-intent `client_secret`. A publishable
    /// key with the wrong `client_secret` is the same uniform 404 as no key
    /// at all (`vpay_api::browser::authenticate`).
    ///
    /// **An explicit list, never derived from [`Self::merchant_id`]** (D1).
    /// Derivation would make a key unretirable — the tenant id cannot change
    /// — and would let anyone who has ever seen an object's owner reconstruct
    /// it. A list also matches what merchants already expect from Stripe: a
    /// key can be rolled by adding the new one, deploying, then removing the
    /// old.
    ///
    /// **An empty list is the fail-closed default and is what most
    /// registrations should have.** A merchant with no `publishable_keys` has
    /// no browser surface at all: every `/v1/browser` request naming a key
    /// this deployment does not know is a 404, and that is the correct answer
    /// for a merchant who never asked for one.
    ///
    /// Validated in `Config::validate_all`, not here, for
    /// [`Self::webhooks`]'s reason — the rules (a shape, uniqueness across
    /// *all* merchants, agreement with `deployment.livemode`) either name
    /// which merchant is wrong or cannot be seen from one registration at
    /// all.
    #[garde(skip)]
    #[serde(default)]
    pub publishable_keys: Vec<String>,
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
            .field("merchant_id", &self.merchant_id)
            .field("jwks", &self.jwks)
            .field("grant_types", &self.grant_types)
            .field("scopes", &self.scopes)
            .field("allowed_audiences", &self.allowed_audiences)
            .field(
                "client_secret",
                &self.client_secret.as_deref().map(|_| "[redacted]"),
            )
            // Its own `Debug` redacts the secrets and keeps the ids and URLs
            // — see [`WebhookEndpoint`]'s impl for why that split.
            .field("webhooks", &self.webhooks)
            // Printed in full, deliberately: a publishable key is public by
            // design (see the field), and "which keys did this deployment
            // actually load?" is the first question asked when a merchant's
            // checkout page answers 404 for every payer.
            .field("publishable_keys", &self.publishable_keys)
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
            merchant_id: "acme".to_owned(),
            jwks: Some(serde_json::json!({"keys": [{"kid": "k1"}]})),
            grant_types: vec![GrantType::ClientCredentials],
            scopes: vec!["payments:write".to_owned()],
            allowed_audiences: vec!["vpay".to_owned()],
            client_secret: Some("this-should-never-be-here".to_owned()),
            webhooks: vec![WebhookEndpoint {
                id: "primary".to_owned(),
                url: "https://acme.example/hooks".to_owned(),
                secrets: vec!["whsec_never_log_me".to_owned()],
            }],
            publishable_keys: vec!["pk_test_visibleonpurpose01".to_owned()],
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
        // And the nested endpoint's secret is gone too: a derived `Debug` on
        // a field whose own `Debug` redacts composes correctly, which is
        // exactly what this asserts rather than assumes.
        assert!(!formatted.contains("whsec_never_log_me"), "{formatted}");
        assert!(
            formatted.contains("https://acme.example/hooks"),
            "{formatted}"
        );
        // A publishable key is public by design and must stay legible: it is
        // what an operator compares against the merchant's own checkout page
        // when every payer is getting a 404.
        assert!(
            formatted.contains("pk_test_visibleonpurpose01"),
            "Debug output must still show the (public) publishable key"
        );
    }

    /// One webhook secret is enough to forge a `payment_intent.succeeded`
    /// a merchant's handler will believe, so it must not be reachable from
    /// any `{:?}` — the registry lives in the server's `AppState` and in the
    /// worker's job loop for the whole life of both processes.
    ///
    /// The *count* is deliberately kept: "is this endpoint mid-rotation?" is
    /// a runbook question that needs no secret to answer.
    #[test]
    fn a_webhook_endpoints_debug_output_never_contains_a_secret() {
        let endpoint = WebhookEndpoint {
            id: "primary".to_owned(),
            url: "https://acme.example/hooks".to_owned(),
            secrets: vec![
                "whsec_current_never_log_me".to_owned(),
                "whsec_incoming_never_log_me".to_owned(),
            ],
        };

        let formatted = format!("{endpoint:?}");

        for secret in &endpoint.secrets {
            assert!(!formatted.contains(secret.as_str()), "{formatted}");
        }
        assert!(formatted.contains("[2 redacted]"), "{formatted}");
        assert!(formatted.contains("primary"), "{formatted}");
        assert!(
            formatted.contains("https://acme.example/hooks"),
            "{formatted}"
        );
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
