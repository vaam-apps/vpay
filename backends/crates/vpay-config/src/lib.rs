//! YAML configuration: load, validate, then reconcile into the database.
//!
//! Administration is YAML in git. A profile selects a *config file*; it must
//! never select a *code path*. See `docs/adr/0003-yaml-configuration.md`.
//!
//! STATUS: types and the deployment guard rules are implemented and tested.
//! Figment layering and DB reconciliation are NOT implemented — `docs/STATUS.md`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub name: String,
    /// Tenancy label stamped on API objects. Gates no behaviour, ever.
    pub livemode: bool,
    pub public_base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEntry {
    pub url: String,
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
