//! Statically registered OAuth2/OIDC clients (ADR-0010,
//! [docs/flows/dashboard-auth.md](../../../../docs/flows/dashboard-auth.md)).
//!
//! Two kinds, both loaded from YAML (ADR-0003) and both, structurally, carrying
//! no client secret: [`MerchantClient`] (a merchant's `/v1` credential —
//! `client_credentials` plus `private_key_jwt`, so vpay only ever holds the
//! **public** JWK set) and [`DashboardClient`] (the single `/dash/v1` client —
//! authorization-code plus PKCE, one read-only scope).
//!
//! Both carry a `client_secret: Option<String>` whose only legitimate value is
//! `None`: it exists so a config that accidentally carries a secret is *refused*
//! at boot rather than silently ignored. Never populate it.
//!
//! Why these types rather than `authkestra_op::client::ClientRegistration`
//! directly, and the field-by-field map onto it:
//! [docs/reference/vpay-config.md § OAuth client shapes](../../../../docs/reference/vpay-config.md#oauth-client-shapes).

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
#[serde(rename_all = "snake_case")]
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
    /// That is the only *boot-time* URL validation there is, and it is a
    /// guard against shipping a stub host into production rather than any
    /// kind of SSRF protection: it never inspects the destination address,
    /// because at boot there is no address to inspect — a name resolves
    /// differently tomorrow.
    ///
    /// **The address is checked at delivery**, since Step 8, by
    /// `vpay_worker::ssrf`: the host is resolved once, every address it
    /// answers with is classified, a loopback/private/link-local/CGNAT
    /// answer is a permanent delivery failure, and the connection is pinned
    /// to the addresses that were classified so a second lookup cannot
    /// substitute another. `webhooks.allow_private_targets`
    /// ([`crate::WebhookPolicy`]) is the one value that changes that verdict,
    /// and livemode refuses it.
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
#[serde(rename_all = "snake_case")]
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
    /// What a payer is told they are paying — the one fact about the
    /// merchant vpay's own checkout page shows (Step 9).
    ///
    /// `None` is legal and is the default. The browser session reads then
    /// fall back to [`Self::merchant_id`], which is a true name for who is
    /// being paid but an *internal* one: `acme-cameroon-tenant` is not what a
    /// payer recognises from the site they were on. A deployment that serves
    /// hosted checkout should set this for every merchant, and lane 1b's note
    /// says so; nothing refuses to boot without it, because a merchant who
    /// never uses the checkout page has no payer to show anything to.
    ///
    /// **Not derived from anything.** There is no merchants table
    /// (ADR-0003), so this is the only place a human-readable name for a
    /// tenant exists at all. It is deliberately not `client_id` either: a
    /// credential's name is not a business's name, and one merchant may hold
    /// several credentials.
    ///
    /// **Not secret**, on the same footing as [`Self::publishable_keys`]: it
    /// is rendered to every payer of this merchant by construction, so it
    /// prints in `Debug`.
    ///
    /// Bounded at 80 characters by `Config::validate_all`
    /// ([`ConfigError::MalformedDisplayName`](crate::ConfigError::MalformedDisplayName)),
    /// which is a rendering rule rather than a storage one: it is painted
    /// into a heading on a phone-sized page, and a value that wrapped to four
    /// lines would push the amount and the pay button below the fold. Empty
    /// or whitespace-only is refused rather than treated as absent — an
    /// operator who wrote `display_name: ""` meant to write something.
    #[garde(skip)]
    #[serde(default)]
    pub display_name: Option<String>,
    /// The origins allowed to **frame** vpay's embedded checkout page for
    /// this merchant (Step 9, D4) — the source list
    /// `Content-Security-Policy: frame-ancestors` is built from.
    ///
    /// Each entry is an *origin*: a scheme, a host and optionally a port
    /// (`https://shop.example`, `https://shop.example:8443`). No path, no
    /// query, no fragment, no trailing slash — that is what `frame-ancestors`
    /// matches, and a source-expression the browser cannot parse makes the
    /// whole directive unusable. `Config::validate_all` refuses anything
    /// else ([`ConfigError::MalformedCheckoutOrigin`](crate::ConfigError::MalformedCheckoutOrigin)).
    ///
    /// **Not secret**, on the same footing as [`Self::publishable_keys`] and
    /// for a stronger reason: an origin is the merchant's own public website.
    /// The checkout app fetches this list by publishable key alone
    /// (`GET /v1/browser/checkout/origins?key=…`, no secret) precisely
    /// because there is nothing here to protect, and because a secret in a
    /// server-side lookup would end up in the Next server's logs.
    ///
    /// **An empty list is the fail-closed default and is what most
    /// registrations should have.** It means this merchant may not embed
    /// vpay's page anywhere: the embedded route answers `frame-ancestors
    /// 'none'`, so a page that tries is refused by the payer's own browser.
    /// Hosted checkout is unaffected — it is never framed.
    ///
    /// A deployment with **no** `checkout.public_base_url` and a merchant
    /// with origins is refused at boot
    /// ([`ConfigError::CheckoutOriginsWithoutBaseUrl`](crate::ConfigError::CheckoutOriginsWithoutBaseUrl)):
    /// there would be no page for those origins to frame.
    ///
    /// Validated in `Config::validate_all`, not here, for
    /// [`Self::publishable_keys`]'s reason — uniqueness is a property of the
    /// whole `merchant_clients` list and cannot be seen from one
    /// registration.
    #[garde(skip)]
    #[serde(default)]
    pub checkout_origins: Vec<String>,
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
            // In full: it is rendered to every payer of this merchant by
            // construction, so there is nothing here to hide, and "which
            // name did this deployment load?" is what an operator asks when
            // a payer reports the wrong shop name on the checkout page.
            .field("display_name", &self.display_name)
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
            // In full too, and for a stronger version of the same reason: an
            // origin is the merchant's own public website, and "which origins
            // did this deployment load?" is the *only* question worth asking
            // when an embedded checkout renders an empty iframe.
            .field("checkout_origins", &self.checkout_origins)
            .finish()
    }
}

/// The dashboard's OAuth2 client registration (`docs/flows/dashboard-auth.md`):
/// authorization-code + PKCE, a public client with no secret at all, and
/// exactly one read-only scope — the dashboard observes, it does not
/// administer (ADR-0008).
#[derive(Clone, Serialize, Deserialize, Validate)]
#[serde(rename_all = "snake_case")]
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
            display_name: Some("Acme Cameroun".to_owned()),
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
            checkout_origins: Vec::new(),
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
        // The display name is rendered to every payer of this merchant by
        // construction, so hiding it would cost an operator the answer to
        // "which name did this deployment load?" and protect nothing.
        assert!(
            formatted.contains("Acme Cameroun"),
            "Debug output must still show the (public) display name"
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
