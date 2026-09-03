//! YAML configuration: load, validate, then reconcile into the database.
//!
//! Administration is YAML in git. A profile selects a *config file*; it must
//! never select a *code path*. See `docs/adr/0003-yaml-configuration.md`.
//!
//! STATUS: boot-sequence steps 1-4 (`docs/flows/configuration.md`) are
//! implemented and tested — [`config::Config::load`] loads
//! `application.yml`, overlays `application-{profile}.yml`, resolves
//! `${ENV}` placeholders (fatal if unresolved), and validates; then each
//! binary joins `providers` against its own linked adapters and calls
//! `vpay_db::config_reconcile::reconcile` to make `currencies` and
//! `providers` match. Both `vpay-server` and `vpay-worker-bin` do all of it
//! at startup, before binding a listener — see each binary's `main.rs` for
//! the ordering and why, and [`ConfigError::ProviderWithoutAdapter`] for the
//! one check this crate cannot make itself.

use serde::{Deserialize, Serialize};

pub mod cli;
pub mod config;
pub mod oauth;
pub mod signal;
pub use cli::{CommonArgs, LogFormat, ServerArgs, WorkerArgs};
pub use config::{Config, CurrencyEntry, ProviderHost};
pub use oauth::{DashboardClient, GrantType, MERCHANT_AUDIENCE, MerchantClient};
pub use signal::ShutdownSignals;

#[derive(Debug, Clone, Serialize, Deserialize, garde::Validate)]
pub struct Deployment {
    #[garde(length(min = 1))]
    pub name: String,
    /// Tenancy label stamped on API objects. Gates no behaviour, ever.
    #[garde(skip)]
    pub livemode: bool,
    #[garde(length(min = 1))]
    pub public_base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, garde::Validate)]
pub struct HostEntry {
    #[garde(length(min = 1))]
    pub url: String,
    #[garde(length(min = 1))]
    pub label: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("livemode requires https, got {0}")]
    InsecureHost(String),
    #[error("livemode must not reference a stub host: {0}")]
    StubHostInLivemode(String),
    #[error("livemode secrets must come from ${{ENV}} placeholders, not literals: {0}")]
    LiteralSecret(String),

    /// No `--config` / `VPAY_CONFIG` was given. There is no implicit
    /// default path — ADR-0003 wants an explicit file per deployment, and
    /// guessing one would be exactly the kind of silent behaviour this repo
    /// forbids.
    #[error("no config path given (pass --config or set VPAY_CONFIG)")]
    MissingPath,
    /// The base `--config` path does not exist. Figment itself is lenient
    /// about a missing file (it yields an empty document), which would turn
    /// a typo'd path into confusing downstream validation errors instead of
    /// a clear one — so this is checked explicitly before Figment ever runs.
    #[error("config file not found: {0}")]
    FileNotFound(String),
    /// The base or profile-overlay file exists but failed to load or parse.
    #[error("failed to load {0}: {1}")]
    Load(String, String),
    /// A `${` was opened but never closed, or the name inside it was not a
    /// plain identifier.
    #[error("malformed ${{...}} placeholder in: {0}")]
    MalformedPlaceholder(String),
    /// Step 2 of the boot sequence: an unresolved placeholder is fatal,
    /// never an empty string (`docs/flows/configuration.md`).
    #[error("unresolved ${{ENV}} placeholder: environment variable {0} is not set")]
    UnresolvedPlaceholder(String),
    /// The merged, placeholder-resolved document does not match `Config`'s
    /// shape (wrong type, missing required field, ...).
    #[error("config does not match the expected shape: {0}")]
    Shape(String),
    /// A `garde`-derived structural rule failed.
    #[error("config validation failed: {0}")]
    Validation(String),
    /// Mirrors the `providers` table's primary key
    /// (`backends/migrations/0002_create-providers.sql`).
    #[error("duplicate provider code: {0}")]
    DuplicateProviderCode(String),
    /// Mirrors the `currencies` table's primary key
    /// (`backends/migrations/0001_create-currencies.sql`).
    #[error("duplicate currency code: {0}")]
    DuplicateCurrencyCode(String),
    /// A `providers[]` entry names a rail this binary links no adapter for,
    /// so boot step 4 has no `vpay_provider::Capabilities` to seed the
    /// `providers` row from.
    ///
    /// **Not raised by `Config::validate_all`**, and it cannot be: which
    /// adapters exist is a property of the *binary*, not of the YAML, and
    /// `vpay-config` links none of them (ADR-0002 — the port is the only
    /// thing the core knows). Each binary raises it while joining its own
    /// `adapters()` against `config.providers`, which is why the message
    /// lists what that binary actually has.
    ///
    /// Fatal rather than skipped: a configured rail with no code behind it
    /// is a deployment that would accept `payment_method_types[]=<code>` on
    /// a payment intent and then fail to confirm it, and the operator who
    /// wrote that line believed it would work.
    #[error(
        "provider {code} is configured but this binary links no adapter for it (linked: {linked})"
    )]
    ProviderWithoutAdapter {
        /// The `providers[].code` from the YAML.
        code: String,
        /// The codes this binary does link, comma-separated, so the message
        /// is actionable without a second lookup.
        linked: String,
    },
    /// Not one of `vpay_core::Currency`'s variants. Raised for a
    /// `currencies[]` entry and for a `providers[].currency`.
    #[error("unknown currency code: {0}")]
    UnknownCurrency(String),
    /// A rail's YAML omits a `settings`/`credentials` key its adapter cannot
    /// work without — the table is `config::REQUIRED_RAIL_KEYS`, and that
    /// constant's own doc explains why a per-code table lives in this crate
    /// at all.
    ///
    /// Fatal at boot rather than at call time on purpose. The alternative is
    /// a `ProviderError::Config` raised while a merchant's `confirm` is in
    /// flight: the same defect, discovered by a payer, hours later, with the
    /// charge row already written. An operator who has just deployed a rail
    /// wants to hear about a missing `api_user` now.
    ///
    /// `section` is `"settings"` or `"credentials"` — a `&'static str` from
    /// the check itself rather than caller-supplied text, so the message
    /// cannot name a section that does not exist. The *value* never appears
    /// in the message: the key is what an operator needs, and a credential
    /// value in a boot error is a credential in a log aggregator.
    #[error("provider {code} is missing required {section} key `{key}`")]
    MissingProviderSetting {
        /// The `providers[].code` whose block is incomplete.
        code: String,
        /// `"settings"` or `"credentials"`.
        section: &'static str,
        /// The key that is absent or empty.
        key: String,
    },
    /// "Currency exponent matches the canonical table"
    /// (`docs/flows/configuration.md`'s boot-guard table) — the exponent is
    /// a property of the currency itself, never configurable per deployment
    /// (`vpay_core::Currency::exponent`).
    #[error("currency {code} exponent {given} does not match the canonical exponent {expected}")]
    CurrencyExponentMismatch {
        code: String,
        given: u32,
        expected: u32,
    },

    /// ADR-0010 / `docs/flows/dashboard-auth.md`: `client_id` is one
    /// namespace across every merchant and the dashboard combined, not
    /// per-kind — a merchant accidentally reusing the dashboard's id (or
    /// vice versa) is exactly the kind of typo this rule exists to catch
    /// before it reaches a real request.
    #[error("duplicate OAuth client_id: {0}")]
    DuplicateClientId(String),
    /// Two merchant clients claim the same tenant. `/v1` resolves a token's
    /// `client_id` to exactly one `merchant_id` and filters every query by
    /// it, so a shared tenant is two credentials with access to each other's
    /// payment intents — and nothing downstream of boot could tell that from
    /// the intended shape. See [`oauth::MerchantClient::merchant_id`].
    #[error("duplicate merchant_id: {0}")]
    DuplicateMerchantId(String),
    /// ADR-0010: `private_key_jwt` with no key can never authenticate, so
    /// booting with an empty or missing JWK set is exactly as pointless as
    /// booting with none configured at all. See
    /// `oauth::jwks_has_at_least_one_key` for the exact shapes this
    /// rejects.
    #[error("merchant client {0} has an empty or missing JWK set")]
    EmptyMerchantJwks(String),
    /// ADR-0010 drops the device-authorization grant and refresh tokens for
    /// `/v1` outright — a merchant client may declare `client_credentials`
    /// and nothing else.
    #[error(
        "merchant client {client_id} declares grant type {grant:?}, only client_credentials is allowed"
    )]
    DisallowedMerchantGrant {
        client_id: String,
        grant: oauth::GrantType,
    },
    /// ADR-0010: a merchant client whose `allowed_audiences` omits
    /// [`oauth::MERCHANT_AUDIENCE`] can never hold a usable `/v1` token, and
    /// neither of the two ways it fails names the cause:
    ///
    /// - The client requests `audience=vpay:v1` (what both SDKs send by
    ///   default): `authkestra_op`'s `handle_client_credentials` checks the
    ///   requested audience against `allowed_audiences` and answers
    ///   `invalid_target` — an error about a *request* for what is really a
    ///   server-side registration defect.
    /// - The client omits `audience` (the SDKs make it configurable): the
    ///   same handler falls back to `aud = client_id`, so the token endpoint
    ///   returns `200` and every subsequent `/v1` call is rejected by
    ///   `vpay_api::resource_auth`'s audience check as a bare `401` with no
    ///   diagnostic — the worse of the two, because it looks like a
    ///   credential problem and is not.
    ///
    /// Refusing to boot is the only place this is cheap to see.
    ///
    /// A named field rather than a tuple variant only because the message
    /// interpolates [`oauth::MERCHANT_AUDIENCE`] itself — spelling the
    /// audience a second time in this string is exactly the drift the
    /// constant exists to prevent — and `thiserror` refuses to mix a
    /// positional format argument with a tuple variant's own `{0}`.
    #[error(
        "merchant client {client_id} does not list `{}` in allowed_audiences; \
         its /v1 tokens would be minted for the wrong audience (ADR-0010)",
        oauth::MERCHANT_AUDIENCE
    )]
    MerchantMissingV1Audience { client_id: String },
    /// `docs/flows/dashboard-auth.md`: authorization-code + PKCE needs
    /// somewhere to redirect back to; a client that can never redirect can
    /// never complete a login.
    #[error("dashboard client {0} declares no redirect_uris")]
    DashboardMissingRedirectUri(String),
    /// ADR-0010 / `docs/flows/dashboard-auth.md`: vpay stores no client
    /// secret, in any form, for any client kind — a merchant authenticates
    /// only via a signed `private_key_jwt` assertion, and the dashboard is a
    /// public client by design. A config that carries a secret is refused
    /// outright rather than the secret being silently dropped as an unknown
    /// field.
    #[error("OAuth client {0} declares a client_secret; vpay stores none (see ADR-0010)")]
    ClientSecretPresent(String),
}

impl vpay_core::Classify for ConfigError {
    fn category(&self) -> vpay_core::Category {
        // Every variant is something an operator fixes with a deploy: a
        // missing flag, a file that does not parse, a rule the YAML broke.
        // None is a caller's request and none heals on retry, so the whole
        // enum is one category and the defaults (500 / never / error / exit
        // 78) all apply. The variant still reaches the log in full via
        // `Display`; only the category is coarse.
        vpay_core::Category::Configuration
    }
}

/// Substrings that mark a host as a stub. A live deployment must refuse them.
///
/// This is the guardrail that makes "the code cannot tell a stub from a real
/// rail" safe to live with.
const STUB_MARKERS: [&str; 4] = ["wiremock", "stub", "mock", "localhost"];

/// # Errors
/// See [`ConfigError`].
pub fn validate_host(host: &HostEntry, livemode: bool) -> Result<(), ConfigError> {
    if !livemode {
        return Ok(());
    }
    if !host.url.starts_with("https://") {
        return Err(ConfigError::InsecureHost(host.url.clone()));
    }
    let hay = format!("{} {}", host.url, host.label).to_ascii_lowercase();
    if STUB_MARKERS.iter().any(|m| hay.contains(m)) {
        return Err(ConfigError::StubHostInLivemode(host.url.clone()));
    }
    Ok(())
}

/// # Errors
/// [`ConfigError::LiteralSecret`] if a live deployment carries an inline secret.
pub fn validate_secret(key: &str, raw: &str, livemode: bool) -> Result<(), ConfigError> {
    if livemode && !(raw.starts_with("${") && raw.ends_with('}')) {
        return Err(ConfigError::LiteralSecret(key.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(url: &str, label: &str) -> HostEntry {
        HostEntry {
            url: url.into(),
            label: label.into(),
        }
    }

    #[test]
    fn a_stub_host_cannot_reach_a_live_deployment() {
        let h = host("http://wiremock.internal:8080", "wiremock-ci");
        assert!(matches!(
            validate_host(&h, true),
            Err(ConfigError::InsecureHost(_) | ConfigError::StubHostInLivemode(_))
        ));
    }

    #[test]
    fn a_stub_host_over_https_is_still_refused_in_livemode() {
        let h = host("https://mock.internal", "ci");
        assert_eq!(
            validate_host(&h, true),
            Err(ConfigError::StubHostInLivemode(
                "https://mock.internal".into()
            ))
        );
    }

    #[test]
    fn sandbox_may_use_stub_hosts_freely() {
        let h = host("http://wiremock.internal:8080", "wiremock-ci");
        assert!(validate_host(&h, false).is_ok());
    }

    #[test]
    fn real_production_hosts_pass() {
        let h = host("https://proxy.momoapi.mtn.com", "mtn-cm-prod");
        assert!(validate_host(&h, true).is_ok());
    }

    #[test]
    fn literal_secrets_are_refused_in_livemode() {
        assert!(validate_secret("api-key", "hunter2", true).is_err());
        assert!(validate_secret("api-key", "${MTN_API_KEY}", true).is_ok());
        assert!(validate_secret("api-key", "hunter2", false).is_ok());
    }
}
