//! Steps 1-3 of the boot sequence in `docs/flows/configuration.md`:
//!
//! 1. Load `application.yml`, overlay `application-{profile}.yml`.
//! 2. Resolve `${ENV}` placeholders. An unresolved one is fatal — never an
//!    empty string.
//! 3. Validate. A validation failure means [`Config::load`] returns `Err`;
//!    the caller must not serve traffic.
//!
//! Step 4 (reconciling into the database in one transaction) is out of
//! scope here — another pass owns persistence, and reconciliation lands
//! after that (`docs/status.md`).
//!
//! # Locating the profile overlay
//!
//! Given a base path `<dir>/<stem>.<ext>` (e.g. `config/application.yml`),
//! the profile overlay is looked up at `<dir>/<stem>-<profile>.<ext>` (e.g.
//! `config/application-sandbox.yml`) — same directory as the base file,
//! Spring Boot's own convention (ADR-0003). The overlay is optional: if it
//! does not exist, only the base file's values apply. If it exists but is
//! malformed, that is a hard error like any other load failure.
//!
//! # What is deliberately not modelled here
//!
//! Merchant OAuth clients and dashboard clients are not represented — those
//! auth decisions are being written up as a separate ADR, and guessing
//! their shape now would be exactly the fabrication `AGENTS.md` forbids.
//! Concretely, this leaves two boot-guard rules from
//! `docs/flows/configuration.md`'s table unimplemented, on purpose:
//!
//! - **"Every merchant's rail host appears in that rail's allowlist"** —
//!   there is no `merchants` table to check against yet (`merchant_id` is a
//!   free `TEXT` column with no FK on both `payment_intents` and
//!   `merchant_api_keys` — see those migrations' own comments), so there is
//!   nothing here to validate a merchant's host against.
//! - **"Every referenced provider exists and is enabled"** — nothing in
//!   this narrow config shape references a provider from outside the
//!   `providers` list itself (no merchant-to-provider routing table
//!   exists), so there is no dangling reference to check for.
//!
//! Both gaps are real and are tracked here rather than papered over with an
//! invented merchant/routing shape. `docs/status.md` is the source of truth
//! for when they get built.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::{Format, Yaml};
use garde::Validate;
use serde::{Deserialize, Serialize};
use vpay_core::Currency;

use crate::{ConfigError, Deployment, HostEntry, validate_host, validate_secret};

/// One rail's connection details for this deployment.
///
/// Mirrors `providers` (`backends/migrations/0002_create-providers.sql`)
/// plus the host/credential material that table's own comment says is
/// reconciled from YAML, not stored there directly.
#[derive(Clone, Serialize, Deserialize, Validate)]
pub struct ProviderHost {
    /// Stable rail code, e.g. `mtn_momo`. Matches `providers.code`.
    #[garde(length(min = 1, max = 64))]
    pub code: String,
    #[garde(dive)]
    pub host: HostEntry,
    /// Non-secret, adapter-defined settings (mirrors
    /// `vpay_provider::ProviderConfig::settings`).
    #[garde(skip)]
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// Secret material (mirrors `vpay_provider::ProviderConfig::credentials`).
    /// Every value must be a `${VAR}` placeholder in a livemode deployment —
    /// enforced by [`validate_secret`] over every entry, driven by
    /// `deployment.livemode` (see [`Config::validate_all`]), not by `garde`:
    /// `garde` has no way to see a sibling field's value from a plain
    /// per-field rule without a custom context, and this rule already has
    /// its own tested implementation.
    ///
    /// `Debug` is hand-written below instead of derived — see that impl for
    /// why.
    #[garde(skip)]
    #[serde(default)]
    pub credentials: BTreeMap<String, String>,
}

/// Redacts `credentials` (rail secrets: MTN subscription keys, Orange client
/// secrets, ...) while leaving every other field visible.
///
/// A derived `Debug` here would print every credential value in plaintext
/// the first time this type reaches a `{:?}`/`{:#?}` — a `tracing::debug!`
/// call, an `anyhow` error chain, a panic message — and for a payment
/// gateway that is a live-credential leak into whatever aggregates the
/// logs, not a hypothetical one. `code`, `host`, and `settings` stay on the
/// derive-equivalent path (printed via their own `Debug` impls) because an
/// operator reading a log line needs to know *which* rail and *which*
/// non-secret settings were loaded; only the credential *values* are the
/// hazard.
///
/// Credential *keys* are printed (e.g. `subscription_key`) with a fixed
/// `"[redacted]"` marker standing in for the value: knowing *which*
/// credentials loaded — did `api_key` come through at all — is the actual
/// debugging need after a boot failure; the value itself never is.
impl fmt::Debug for ProviderHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted_credentials: BTreeMap<&str, &str> = self
            .credentials
            .keys()
            .map(|key| (key.as_str(), "[redacted]"))
            .collect();
        f.debug_struct("ProviderHost")
            .field("code", &self.code)
            .field("host", &self.host)
            .field("settings", &self.settings)
            .field("credentials", &redacted_credentials)
            .finish()
    }
}

/// One entry in the currency table
/// (`backends/migrations/0001_create-currencies.sql`).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CurrencyEntry {
    /// ISO-4217 alphabetic code, e.g. `XAF`.
    #[garde(length(min = 1))]
    pub code: String,
    /// Minor units per major unit, as a power of ten. Checked against
    /// [`vpay_core::Currency::exponent`] — the canonical table — by
    /// [`Config::validate_all`], not just range-checked here: the exponent
    /// is a property of the currency itself, never a per-deployment choice.
    #[garde(range(min = 0, max = 4))]
    pub exponent: u32,
}

/// The whole loaded, validated configuration (ADR-0003).
///
/// `#[derive(Debug)]` is safe to keep here even though `providers` carries
/// [`ProviderHost::credentials`] (rail secrets): a derived `Debug` formats
/// each field by calling *that field's own* `Debug` impl, and
/// [`ProviderHost`] hand-writes one that redacts. There is no raw-memory
/// dump involved, so this composes correctly — proved in this module's
/// tests rather than just asserted here.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct Config {
    #[garde(dive)]
    pub deployment: Deployment,
    #[garde(dive)]
    #[serde(default)]
    pub providers: Vec<ProviderHost>,
    #[garde(dive)]
    #[serde(default)]
    pub currencies: Vec<CurrencyEntry>,
}

impl Config {
    /// Loads, resolves `${ENV}` placeholders against the real process
    /// environment, and validates — steps 1-3 of the boot sequence
    /// (`docs/flows/configuration.md`). `path` is `CommonArgs::config`,
    /// `profile` is `CommonArgs::profile`.
    ///
    /// # Errors
    /// See [`ConfigError`]. A validation failure here means the caller must
    /// exit non-zero without serving traffic (ADR-0003) — this function
    /// never returns a half-valid `Config`.
    pub fn load(path: Option<&Path>, profile: &str) -> Result<Self, ConfigError> {
        Self::load_with_env(path, profile, &|key: &str| std::env::var(key).ok())
    }

    /// As [`Config::load`], but `${ENV}` placeholders resolve through `env`
    /// instead of the real process environment.
    ///
    /// This is the injection point that makes the fatal-on-missing-var path
    /// testable without touching real process env: `std::env::set_var` is
    /// `unsafe` as of edition 2024, and this workspace forbids `unsafe`
    /// outright with no test carve-out (see `cli.rs`'s own note on the same
    /// constraint). A plain `&dyn Fn` also happens to be the better design
    /// regardless of testability — it makes the dependency on the
    /// environment explicit at the call site instead of implicit inside the
    /// function body.
    ///
    /// # Errors
    /// See [`ConfigError`].
    pub fn load_with_env(
        path: Option<&Path>,
        profile: &str,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let base_path = path.ok_or(ConfigError::MissingPath)?;
        if !base_path.is_file() {
            return Err(ConfigError::FileNotFound(base_path.display().to_string()));
        }

        let mut figment = Figment::new().merge(Yaml::file(base_path));
        if let Some(overlay_path) = profile_overlay_path(base_path, profile)
            && overlay_path.is_file()
        {
            figment = figment.merge(Yaml::file(&overlay_path));
        }

        let mut raw: figment::value::Value = figment
            .extract()
            .map_err(|e| ConfigError::Load(base_path.display().to_string(), e.to_string()))?;

        resolve_placeholders(&mut raw, env)?;

        // `figment::value::Value` and `serde_yaml_ng::Value` are different
        // crates' types with no direct conversion between them. Figment's
        // `Value` already implements `Serialize`
        // (`figment::value::ser::impl Serialize for Value`), so round-
        // tripping through YAML text reuses well-worn serde machinery
        // instead of hand-rolling a `serde::Deserializer` over Figment's
        // tree.
        let yaml_text =
            serde_yaml_ng::to_string(&raw).map_err(|e| ConfigError::Shape(e.to_string()))?;
        let config: Self =
            serde_yaml_ng::from_str(&yaml_text).map_err(|e| ConfigError::Shape(e.to_string()))?;

        config.validate_all()?;
        Ok(config)
    }

    /// Runs every validation rule: `garde`'s structural derive, then the
    /// existing tested guard rules (`validate_host`, `validate_secret`)
    /// over every provider, then the currency-table checks.
    fn validate_all(&self) -> Result<(), ConfigError> {
        self.validate()
            .map_err(|report| ConfigError::Validation(report.to_string()))?;

        let livemode = self.deployment.livemode;
        let mut seen_providers = BTreeSet::new();
        for provider in &self.providers {
            if !seen_providers.insert(provider.code.as_str()) {
                return Err(ConfigError::DuplicateProviderCode(provider.code.clone()));
            }
            validate_host(&provider.host, livemode)?;
            for (key, raw) in &provider.credentials {
                validate_secret(&format!("{}.{key}", provider.code), raw, livemode)?;
            }
        }

        let mut seen_currencies = BTreeSet::new();
        for entry in &self.currencies {
            if !seen_currencies.insert(entry.code.as_str()) {
                return Err(ConfigError::DuplicateCurrencyCode(entry.code.clone()));
            }
            let canonical = Currency::from_code(&entry.code)
                .map_err(|_| ConfigError::UnknownCurrency(entry.code.clone()))?;
            if canonical.exponent() != entry.exponent {
                return Err(ConfigError::CurrencyExponentMismatch {
                    code: entry.code.clone(),
                    given: entry.exponent,
                    expected: canonical.exponent(),
                });
            }
        }

        Ok(())
    }
}

/// See the module docs' "Locating the profile overlay" section.
fn profile_overlay_path(base_path: &Path, profile: &str) -> Option<PathBuf> {
    let parent = base_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = base_path.file_stem()?.to_str()?;
    let file_name = match base_path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{stem}-{profile}.{ext}"),
        None => format!("{stem}-{profile}"),
    };
    Some(parent.join(file_name))
}

/// Walks the merged document, replacing every `${VAR}` found in a string
/// leaf. Step 2 of the boot sequence — see the module docs.
fn resolve_placeholders(
    value: &mut figment::value::Value,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<(), ConfigError> {
    use figment::value::Value;

    match value {
        Value::String(_, s) => {
            *s = resolve_string(s, env)?;
        }
        Value::Array(_, items) => {
            for item in items.iter_mut() {
                resolve_placeholders(item, env)?;
            }
        }
        Value::Dict(_, dict) => {
            for v in dict.values_mut() {
                resolve_placeholders(v, env)?;
            }
        }
        Value::Char(..) | Value::Bool(..) | Value::Num(..) | Value::Empty(..) => {}
    }
    Ok(())
}

/// Replaces every `${VAR}` in `s` with what `env` returns for `VAR`. An
/// unresolved or malformed placeholder is a hard, named error — never an
/// empty string.
///
/// Deliberately index-free (no `s[a..b]`): `clippy::indexing_slicing` is a
/// warn-promoted-to-error lint here, and a byte-range slice on a `&str` can
/// panic on a non-`char`-boundary index anyway, which a "trusted config
/// file" input does not earn an exemption from.
fn resolve_string(s: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '$' || chars.peek() != Some(&'{') {
            out.push(c);
            continue;
        }
        chars.next(); // consume the '{'

        let mut var = String::new();
        let mut closed = false;
        for c2 in chars.by_ref() {
            if c2 == '}' {
                closed = true;
                break;
            }
            var.push(c2);
        }

        let well_formed =
            closed && !var.is_empty() && var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !well_formed {
            return Err(ConfigError::MalformedPlaceholder(s.to_owned()));
        }

        let resolved = env(&var).ok_or_else(|| ConfigError::UnresolvedPlaceholder(var.clone()))?;
        out.push_str(&resolved);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;

    /// The example config shipped under `config/` at the repo root — see
    /// the module docs. Referenced from a test so it cannot rot.
    const EXAMPLE_BASE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../config/application.yml"
    );

    fn example_env(overrides: BTreeMap<String, String>) -> impl Fn(&str) -> Option<String> {
        let defaults: BTreeMap<&str, &str> = [
            ("MTN_SUBSCRIPTION_KEY", "sub-key-test"),
            ("MTN_API_KEY", "api-key-test"),
            ("ORANGE_MERCHANT_KEY", "merchant-key-test"),
        ]
        .into_iter()
        .collect();
        move |key: &str| {
            overrides
                .get(key)
                .cloned()
                .or_else(|| defaults.get(key).map(|v| (*v).to_owned()))
        }
    }

    #[test]
    fn a_valid_config_loads_and_produces_the_expected_typed_values() {
        let env = example_env(BTreeMap::new());
        let config = Config::load_with_env(Some(Path::new(EXAMPLE_BASE)), "does-not-exist", &env)
            .expect("example config should load");

        assert_eq!(config.deployment.name, "vpay");
        assert!(!config.deployment.livemode);
        assert_eq!(config.deployment.public_base_url, "http://localhost:8080");

        let mtn = config
            .providers
            .iter()
            .find(|p| p.code == "mtn_momo")
            .expect("mtn_momo provider present");
        assert_eq!(
            mtn.credentials.get("subscription_key").map(String::as_str),
            Some("sub-key-test")
        );

        let xaf = config
            .currencies
            .iter()
            .find(|c| c.code == "XAF")
            .expect("XAF currency present");
        assert_eq!(xaf.exponent, 0);
    }

    #[test]
    fn the_profile_overlay_actually_overrides_a_base_value() {
        let env = example_env(BTreeMap::new());

        let base_only =
            Config::load_with_env(Some(Path::new(EXAMPLE_BASE)), "does-not-exist", &env)
                .expect("base-only load should succeed");
        assert_eq!(base_only.deployment.name, "vpay");

        let overlaid = Config::load_with_env(Some(Path::new(EXAMPLE_BASE)), "sandbox", &env)
            .expect("sandbox overlay should load");
        assert_eq!(overlaid.deployment.name, "vpay-sandbox");
        // Only `deployment.name` differs in the overlay — the rest of the
        // base document must survive the merge untouched.
        assert_eq!(overlaid.deployment.public_base_url, "http://localhost:8080");
        assert_eq!(overlaid.providers.len(), base_only.providers.len());
    }

    #[test]
    fn an_unresolved_placeholder_is_a_hard_error_naming_the_variable() {
        let mut overrides = BTreeMap::new();
        // Everything resolves except MTN_API_KEY, which the env fn below
        // will refuse.
        overrides.insert("MTN_SUBSCRIPTION_KEY", "sub-key-test");
        overrides.insert("ORANGE_MERCHANT_KEY", "merchant-key-test");
        let env = move |key: &str| overrides.get(key).map(|v| (*v).to_owned());

        let err = Config::load_with_env(Some(Path::new(EXAMPLE_BASE)), "sandbox", &env)
            .expect_err("a missing env var must be fatal");
        assert_eq!(
            err,
            ConfigError::UnresolvedPlaceholder("MTN_API_KEY".to_owned())
        );
    }

    #[test]
    fn a_livemode_config_with_an_http_host_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/livemode-insecure-host.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("an http host under livemode must be rejected");
        assert!(matches!(err, ConfigError::InsecureHost(_)));
    }

    #[test]
    fn a_livemode_config_with_a_literal_secret_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/livemode-literal-secret.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a literal secret under livemode must be rejected");
        assert!(matches!(err, ConfigError::LiteralSecret(_)));
    }

    #[test]
    fn a_currency_exponent_that_does_not_match_the_canonical_table_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/currency-exponent-mismatch.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a wrong exponent for a known currency must be rejected");
        assert_eq!(
            err,
            ConfigError::CurrencyExponentMismatch {
                code: "XAF".to_owned(),
                given: 2,
                expected: 0,
            }
        );
    }

    #[test]
    fn a_duplicate_provider_code_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/duplicate-provider-code.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a duplicate provider code must be rejected");
        assert_eq!(
            err,
            ConfigError::DuplicateProviderCode("mtn_momo".to_owned())
        );
    }

    #[test]
    fn a_malformed_yaml_file_produces_a_clean_typed_error_not_a_panic() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/malformed.yml");
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("malformed YAML must be a clean error");
        assert!(matches!(err, ConfigError::Load(_, _)));
    }

    #[test]
    fn a_missing_config_path_is_a_named_error_not_a_default() {
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(None, "sandbox", &env)
            .expect_err("no path must not silently fall back to a default");
        assert_eq!(err, ConfigError::MissingPath);
    }

    #[test]
    fn a_nonexistent_base_file_is_a_named_error() {
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(
            Some(Path::new("/definitely/does/not/exist/application.yml")),
            "sandbox",
            &env,
        )
        .expect_err("a missing base file must be a named error");
        assert!(matches!(err, ConfigError::FileNotFound(_)));
    }

    #[test]
    fn resolve_string_leaves_plain_text_untouched() {
        let env = |_: &str| None;
        assert_eq!(
            resolve_string("plain text", &env).as_deref(),
            Ok("plain text")
        );
    }

    #[test]
    fn resolve_string_substitutes_an_embedded_placeholder() {
        let env = |k: &str| (k == "HOST").then(|| "example.com".to_owned());
        assert_eq!(
            resolve_string("https://${HOST}/api", &env).as_deref(),
            Ok("https://example.com/api")
        );
    }

    #[test]
    fn resolve_string_rejects_an_unterminated_placeholder() {
        let env = |_: &str| None;
        assert_eq!(
            resolve_string("${UNCLOSED", &env),
            Err(ConfigError::MalformedPlaceholder("${UNCLOSED".to_owned()))
        );
    }

    /// A known-secret literal must never appear in `ProviderHost`'s `Debug`
    /// output. This is the test that would fail if someone re-derived
    /// `Debug` on `ProviderHost` — the exact regression this hand-written
    /// impl exists to prevent.
    #[test]
    fn provider_host_debug_output_never_contains_a_credential_value() {
        let host = ProviderHost {
            code: "mtn_momo".to_owned(),
            host: HostEntry {
                url: "https://proxy.momoapi.mtn.com".to_owned(),
                label: "mtn-cm-prod".to_owned(),
            },
            settings: BTreeMap::from([(
                "subscription_key_header".to_owned(),
                "Ocp-Apim-Subscription-Key".to_owned(),
            )]),
            credentials: BTreeMap::from([(
                "api_key".to_owned(),
                "super-secret-live-mtn-key".to_owned(),
            )]),
        };

        let formatted = format!("{host:?}");

        assert!(
            !formatted.contains("super-secret-live-mtn-key"),
            "credential value leaked into Debug output: {formatted}"
        );
    }

    /// The redaction must not swallow everything: an operator reading a log
    /// line still needs the rail code, host, non-secret settings, and which
    /// credential *keys* loaded (without their values) — otherwise the
    /// redacted `Debug` is useless and people will work around it.
    #[test]
    fn provider_host_debug_output_still_contains_the_non_secret_fields() {
        let host = ProviderHost {
            code: "mtn_momo".to_owned(),
            host: HostEntry {
                url: "https://proxy.momoapi.mtn.com".to_owned(),
                label: "mtn-cm-prod".to_owned(),
            },
            settings: BTreeMap::from([(
                "subscription_key_header".to_owned(),
                "Ocp-Apim-Subscription-Key".to_owned(),
            )]),
            credentials: BTreeMap::from([(
                "api_key".to_owned(),
                "super-secret-live-mtn-key".to_owned(),
            )]),
        };

        let formatted = format!("{host:?}");

        assert!(formatted.contains("mtn_momo"), "{formatted}");
        assert!(
            formatted.contains("https://proxy.momoapi.mtn.com"),
            "{formatted}"
        );
        assert!(formatted.contains("mtn-cm-prod"), "{formatted}");
        assert!(formatted.contains("subscription_key_header"), "{formatted}");
        assert!(
            formatted.contains("Ocp-Apim-Subscription-Key"),
            "{formatted}"
        );
        assert!(formatted.contains("api_key"), "{formatted}");
        assert!(formatted.contains("[redacted]"), "{formatted}");
    }

    /// Proves the composition argued in [`Config`]'s doc comment: a whole
    /// loaded [`Config`] (which keeps `#[derive(Debug)]`) still never leaks
    /// a nested provider's credential value, because the derive delegates
    /// to [`ProviderHost`]'s hand-written impl for each element of
    /// `providers`.
    #[test]
    fn a_whole_config_debug_output_never_contains_a_credential_value() {
        let env = example_env(BTreeMap::new());
        let config = Config::load_with_env(Some(Path::new(EXAMPLE_BASE)), "does-not-exist", &env)
            .expect("example config should load");

        let formatted = format!("{config:?}");

        assert!(!formatted.contains("sub-key-test"), "{formatted}");
        assert!(!formatted.contains("api-key-test"), "{formatted}");
        assert!(!formatted.contains("merchant-key-test"), "{formatted}");
        // Non-secret fields survive the trip through the whole `Config`.
        assert!(formatted.contains("vpay"), "{formatted}");
        assert!(formatted.contains("mtn_momo"), "{formatted}");
        assert!(formatted.contains("[redacted]"), "{formatted}");
    }
}
