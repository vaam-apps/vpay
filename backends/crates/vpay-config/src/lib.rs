//! YAML configuration: load, validate, then reconcile into the database.
//!
//! Administration is YAML in git. A profile selects a *config file*; it must
//! never select a *code path*. See `docs/adr/0003-yaml-configuration.md`.
//!
//! STATUS: boot-sequence steps 1-3 (`docs/flows/configuration.md`) are
//! implemented and tested — [`config::Config::load`] loads
//! `application.yml`, overlays `application-{profile}.yml`, resolves
//! `${ENV}` placeholders (fatal if unresolved), and validates. Both
//! `vpay-server` and `vpay-worker-bin` now call it at startup, before
//! connecting to the database or (for `vpay-server`) binding a listener —
//! see each binary's `main.rs` for the ordering and why. Step 4 (DB
//! reconciliation) is NOT implemented — `docs/status.md`.

use serde::{Deserialize, Serialize};

pub mod cli;
pub mod config;
pub mod oauth;
pub mod signal;
pub use cli::{CommonArgs, LogFormat, ServerArgs, WorkerArgs};
pub use config::{Config, CurrencyEntry, ProviderHost};
pub use oauth::{DashboardClient, GrantType, MerchantClient};
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
    /// Not one of `vpay_core::Currency`'s variants.
    #[error("unknown currency code: {0}")]
    UnknownCurrency(String),
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
    /// ADR-0010: `private_key_jwt` with no key can never authenticate, so
    /// booting with an empty or missing JWK set is exactly as pointless as
    /// booting with none configured at all. See
    /// [`oauth::jwks_has_at_least_one_key`] for the exact shapes this
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
