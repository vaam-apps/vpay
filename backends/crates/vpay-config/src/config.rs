//! Steps 1-3 of the boot sequence in `docs/flows/configuration.md`:
//!
//! 1. Load `application.yml`, overlay `application-{profile}.yml`.
//! 2. Resolve `${ENV}` placeholders. An unresolved one is fatal — never an
//!    empty string.
//! 3. Validate. A validation failure means [`Config::load`] returns `Err`;
//!    the caller must not serve traffic.
//!
//! # One rule is asked of step 1's text, not step 2's values
//!
//! `validate_secret`'s livemode rule is "a credential must be written as a
//! `${VAR}` placeholder, not as a literal". That is a question about what
//! the *file* says, and it stops being answerable the moment step 2 has run
//! — a resolved `${MTN_API_KEY}` and a literal `hunter2` are the same
//! string by then. Running it after resolution therefore did not enforce
//! the rule; it made **livemode unbootable**, because every correctly
//! written credential resolves to something that is not a placeholder.
//! (Measured: no livemode deployment could ever have started. The literal
//! fixture passed only because a literal is also not a placeholder.)
//!
//! So the pre-resolution text of every secret is captured before step 2 and
//! carried into step 3 (`RawSecrets`, private to this module). "Every
//! secret" is two lists, not one: `providers[].credentials` and — since
//! 2026-09-03 (Step 5) — `merchant_clients[].webhooks[].secrets`. The second
//! was the gap `docs/plans/2026-09-03-step5-webhooks.md`'s S3 names: a
//! livemode config carrying `secrets: [whsec_literal]` booted clean, and one
//! of those values is enough to forge a `payment_intent.succeeded` a
//! merchant's handler will act on.
//! The "an unresolved placeholder is fatal" rule stays exactly where it was,
//! in step 2, and the two rules now answer the two different questions they
//! were always meant to: *was it written as a reference*, and *did the
//! reference resolve*.
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
//! # OAuth clients are modelled; merchant *payment routing* still is not
//!
//! ADR-0010 has since settled the OAuth client shape, so `merchant_clients`
//! and `dashboard_client` below are real, validated config — see
//! `crate::oauth` for the types and the module docs there for how they map
//! onto `authkestra_op::client::ClientRegistration`. That is a narrower
//! claim than "merchant onboarding is modelled": these are *authentication*
//! clients (who may call `/v1` or `/dash/v1`, and how), not the payment
//! routing concept the boot-guard table in `docs/flows/configuration.md`
//! means by "merchant". Concretely, this still leaves two boot-guard rules
//! from that table unimplemented, on purpose:
//!
//! - **"Every merchant's rail host appears in that rail's allowlist"** —
//!   there is no *payment-routing* `merchants` table to check against yet
//!   (`merchant_id` is a free `TEXT` column with no FK on
//!   `payment_intents` — see that migration's own comment); an OAuth
//!   `MerchantClient`'s `client_id` is not the same thing and has no rail
//!   host of its own to check.
//! - **"Every referenced provider exists and is enabled"** — nothing in
//!   this config shape references a provider from outside the `providers`
//!   list itself (no merchant-to-provider routing table exists), so there
//!   is no dangling reference to check for.
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
use vpay_provider::ProviderConfig;

use crate::oauth::{DashboardClient, MERCHANT_AUDIENCE, MerchantClient, jwks_has_at_least_one_key};
use crate::{
    ConfigError, Deployment, GrantType, HostEntry, validate_host, validate_secret,
    validate_webhook_url,
};

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
    /// Whether this rail may be named in a `payment_method_types` on a new
    /// payment intent, and whether `providers.enabled` is `true` after boot
    /// step 4 reconciles this list into the database.
    ///
    /// Defaults to `true`, so an existing config keeps working and the
    /// *only* reason a rail is off is that someone wrote it down. The
    /// opposite default would mean a config that predates this field
    /// silently loses every rail — a deployment that boots, looks healthy,
    /// and refuses every charge.
    ///
    /// Turning a rail off here is deliberately **not** the same as deleting
    /// its block: the host and credentials stay loaded, so an operator can
    /// stop new charges from being routed to a rail without discarding the
    /// configuration needed to reconcile the charges already on it. A rail
    /// removed from the list entirely is disabled too (boot step 4 flips
    /// `enabled = false` for any code no longer present), but its
    /// configuration is then gone.
    ///
    /// No capability fields live here on purpose: `flow`,
    /// `supports_refunds` and the rest are properties of the *adapter*
    /// (`vpay_provider::Capabilities`), not of a deployment, and letting
    /// YAML state them would let a deployment claim a rail can do something
    /// its code cannot (ADR-0002).
    #[garde(skip)]
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[garde(dive)]
    pub host: HostEntry,
    /// Non-secret, adapter-defined settings (mirrors
    /// `vpay_provider::ProviderConfig::settings`).
    #[garde(skip)]
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// Where this rail is told to send its callback, if the derived default
    /// is wrong for this deployment.
    ///
    /// Absent — the normal case — means
    /// `{deployment.public_base_url}/provider/{code}/callback`
    /// (`docs/api/README.md`'s path), derived by
    /// [`ProviderHost::to_provider_config`]. Derivation is the default
    /// because a URL an operator retypes per rail is a URL an operator
    /// mistypes, and a mistyped one points a *live* rail at nothing while
    /// looking perfectly healthy at boot.
    ///
    /// It is overridable because the derived form assumes the rail can reach
    /// this deployment at its own public base URL, and one real deployment
    /// shape breaks that: a rail whose callbacks must arrive on a separate
    /// ingress (an IP-allowlisted host — MTN requires one,
    /// `Capabilities::requires_ip_allowlist`) has a callback host that is
    /// genuinely not `public_base_url`.
    ///
    /// Under `livemode` an override must be `https://` and must not look
    /// like a stub, exactly as a rail host must — see `Config::validate_all`.
    #[garde(skip)]
    #[serde(default)]
    pub callback_url: Option<String>,
    /// The ISO-4217 code this rail transacts in, e.g. `XAF`.
    ///
    /// A property of the rail's *profile*, not of a charge: MTN's sandbox
    /// rejects XAF and accepts EUR only (`docs/flows/money.md`), which is a
    /// deployment fact and must never be a code branch (ADR-0003). Required
    /// rather than defaulted: a rail that submits amounts in the wrong
    /// currency is the single most expensive kind of config typo, and there
    /// is no value that is safe to guess.
    ///
    /// Checked against [`vpay_core::Currency::from_code`] by
    /// `Config::validate_all`; `garde` only pins the length, because the
    /// canonical set lives in `vpay-core` and duplicating it here is how the
    /// two drift.
    #[garde(length(min = 3, max = 3))]
    pub currency: String,
    /// Secret material (mirrors `vpay_provider::ProviderConfig::credentials`).
    /// Every value must be a `${VAR}` placeholder in a livemode deployment —
    /// enforced by [`validate_secret`] over every entry, driven by
    /// `deployment.livemode` (see `Config::validate_all`), not by `garde`:
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

/// `#[serde(default)]` for a `bool` yields `false`; [`ProviderHost::enabled`]
/// needs the other one. A named function rather than a literal because serde
/// only accepts a path here.
const fn default_true() -> bool {
    true
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
            .field("enabled", &self.enabled)
            .field("host", &self.host)
            .field("callback_url", &self.callback_url)
            .field("currency", &self.currency)
            .field("settings", &self.settings)
            .field("credentials", &redacted_credentials)
            .finish()
    }
}

/// Decision record: [ADR-0012](../../../../docs/adr/0012-rail-configuration-requirements-in-config.md)
/// — this table is the one sanctioned provider-code match outside an adapter
/// crate, and it moves behind the port the day `required_settings()` exists.
/// The keys each rail's adapter cannot work without, checked at boot.
///
/// # Why a table keyed by provider code lives outside an adapter crate
///
/// ADR-0002 forbids branching on a provider *code* outside
/// `vpay-adapter-*`, and this is deliberately not that: nothing here selects
/// behaviour: it selects a *refusal to start*. The alternative — an adapter
/// declaring its own required keys through the port — is the better design
/// and is not built (the port has no `required_settings()` today), so this
/// table is the honest interim: one visible list, checked once, that fails a
/// deployment at boot instead of failing a payer's charge at 3am with
/// `ProviderError::Config`. Growing a rail means editing this list *and* an
/// adapter, which is the coupling to remove when the port grows the hook.
///
/// The contents mirror `docs/flows/adapter-mtn-momo.md` and
/// `docs/flows/adapter-orange-money.md`, which are the source of truth for
/// what each rail's API demands.
///
/// Orange's `merchant_key` sits under `credentials` rather than `settings`
/// (Step 3's decision 4): it is not a bearer secret, but it is per-merchant
/// material that already lives there in `config/application.yml`, and
/// `ProviderHost`'s `Debug` redacts `credentials` while printing `settings`
/// in full — so moving it would make it log-visible for no gain.
const REQUIRED_RAIL_KEYS: [RequiredRailKeys; 2] = [
    RequiredRailKeys {
        code: "mtn_momo",
        // `target_environment` is MTN's `X-Target-Environment` header
        // (`sandbox`, or the country product name in production) and
        // `api_user` is the UUID half of the Basic credential the token call
        // uses. Neither is secret; both are fatal to omit, because MTN
        // answers a missing target environment with a 500 whose body says
        // `NOT_ALLOWED_TARGET_ENVIRONMENT` — a failure that looks like the
        // rail is broken rather than like our YAML is.
        settings: &["target_environment", "api_user"],
        credentials: &["subscription_key", "api_key"],
    },
    RequiredRailKeys {
        code: "orange_money",
        settings: &[],
        credentials: &["merchant_key", "client_id", "client_secret"],
    },
];

/// One row of [`REQUIRED_RAIL_KEYS`].
struct RequiredRailKeys {
    code: &'static str,
    settings: &'static [&'static str],
    credentials: &'static [&'static str],
}

impl ProviderHost {
    /// Projects this rail's YAML onto the [`ProviderConfig`] an adapter is
    /// handed at call time.
    ///
    /// This is the *only* place a `ProviderConfig` is built from
    /// configuration, so the callback-URL derivation, the currency parse and
    /// the timeout defaults cannot diverge between the server and the worker
    /// — two processes disagreeing about a rail's callback URL would be a
    /// defect with no symptom until a rail called the wrong host.
    ///
    /// `deployment` rather than a bare base URL: the derivation needs
    /// `public_base_url`, and passing the whole struct means a future rule
    /// that needs another deployment fact does not change this signature.
    ///
    /// The timeouts are [`vpay_provider::DEFAULT_CONNECT_TIMEOUT`] /
    /// [`vpay_provider::DEFAULT_REQUEST_TIMEOUT`] and are not configurable in
    /// YAML: no deployment has asked for a different budget, and a knob no
    /// one sets is a knob nobody has tested. The conformance suite overrides
    /// them by building a `ProviderConfig` directly, which is the one caller
    /// that genuinely needs a 100 ms deadline.
    ///
    /// # Errors
    ///
    /// [`ConfigError::UnknownCurrency`] if `currency` is not one
    /// [`vpay_core::Currency`] knows. `Config::validate_all` has already
    /// refused to boot on that, so reaching it here means a `ProviderHost`
    /// built in code rather than loaded from a file.
    pub fn to_provider_config(
        &self,
        deployment: &Deployment,
    ) -> Result<ProviderConfig, ConfigError> {
        Ok(ProviderConfig {
            base_url: self.host.url.clone(),
            callback_url: self.effective_callback_url(deployment),
            currency: Currency::from_code(&self.currency.to_ascii_uppercase())
                .map_err(|_| ConfigError::UnknownCurrency(self.currency.clone()))?,
            settings: self.settings.clone(),
            credentials: self.credentials.clone(),
            connect_timeout: vpay_provider::DEFAULT_CONNECT_TIMEOUT,
            request_timeout: vpay_provider::DEFAULT_REQUEST_TIMEOUT,
        })
    }

    /// The callback URL this rail will actually be given: the override if
    /// there is one, otherwise `docs/api/README.md`'s derived path.
    ///
    /// Separate from [`ProviderHost::to_provider_config`] so the livemode
    /// `https` rule can be checked against the *effective* value at boot —
    /// validating only an override would leave the derived form (the one
    /// almost every deployment uses) unchecked.
    #[must_use]
    pub fn effective_callback_url(&self, deployment: &Deployment) -> String {
        self.callback_url.clone().unwrap_or_else(|| {
            let base = deployment.public_base_url.trim_end_matches('/');
            format!("{base}/provider/{}/callback", self.code)
        })
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
    /// `Config::validate_all`, not just range-checked here: the exponent
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
    /// Statically registered merchant OAuth2 clients (ADR-0010). See
    /// `crate::oauth` for the shape and what boots refuse to start.
    #[garde(dive)]
    #[serde(default)]
    pub merchant_clients: Vec<MerchantClient>,
    /// The dashboard's OAuth2 client (`docs/flows/dashboard-auth.md`).
    /// `Option` because not every deployment needs the dashboard wired up
    /// yet — unlike a merchant client, there is exactly one of these ever,
    /// so there is nothing to default to an empty list.
    #[garde(dive)]
    #[serde(default)]
    pub dashboard_client: Option<DashboardClient>,
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

        // Before step 2, because after it there is nothing left to ask —
        // see this module's header.
        let raw_secrets = RawSecrets::from_document(&raw);

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

        config.validate_all(&raw_secrets)?;
        Ok(config)
    }

    /// Runs every validation rule: `garde`'s structural derive, then the
    /// existing tested guard rules (`validate_host`, `validate_secret`)
    /// over every provider, then the currency-table checks.
    fn validate_all(&self, raw_secrets: &RawSecrets) -> Result<(), ConfigError> {
        self.validate()
            .map_err(|report| ConfigError::Validation(report.to_string()))?;

        let livemode = self.deployment.livemode;
        // Uniqueness is a property of the *list*, so it is checked over the
        // whole list before any single rail's rules run. Otherwise the first
        // entry's own defect (a missing key, a bad currency) would be
        // reported instead of the duplicate, and an operator would fix the
        // named problem and hit the real one on the next boot.
        let mut seen_providers = BTreeSet::new();
        for provider in &self.providers {
            if !seen_providers.insert(provider.code.as_str()) {
                return Err(ConfigError::DuplicateProviderCode(provider.code.clone()));
            }
        }
        for provider in &self.providers {
            // Both destinations first — where we call the rail, and where the
            // rail calls us — then what we authenticate with, then whether the
            // block is complete. The order is what an operator reads: an
            // unreachable or plaintext endpoint makes every later question
            // moot.
            validate_host(&provider.host, livemode)?;
            // The *effective* callback URL, not just an override: the derived
            // form is what almost every deployment uses, and a livemode
            // deployment whose `public_base_url` is `http://` would otherwise
            // hand a live rail an unencrypted callback host that nothing
            // checked. Reuses `validate_host` for the same reason
            // `validate_dashboard_client` does — a callback URL is
            // structurally the same kind of destination, and a leftover
            // `localhost`/`wiremock` one in production is exactly as
            // dangerous. The synthetic `label` only feeds the stub-marker
            // search and is stored nowhere.
            validate_host(
                &HostEntry {
                    url: provider.effective_callback_url(&self.deployment),
                    label: format!("{} callback", provider.code),
                },
                livemode,
            )?;
            for key in provider.credentials.keys() {
                // The value **as written**, never `provider.credentials[key]`
                // — that one has been through step 2. `as_written` answering
                // `None` fails closed for the same reason the rule exists.
                let as_written = raw_secrets.provider_credential_as_written(&provider.code, key);
                validate_secret(
                    &format!("{}.{key}", provider.code),
                    as_written.unwrap_or(""),
                    livemode,
                )?;
            }
            // The currency is checked here, not only when
            // `to_provider_config` runs, because that call happens per
            // request: an unknown code would otherwise be a 500 on a
            // merchant's confirm rather than a refusal to start.
            Currency::from_code(&provider.currency.to_ascii_uppercase())
                .map_err(|_| ConfigError::UnknownCurrency(provider.currency.clone()))?;
            validate_required_rail_keys(provider)?;
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

        // `client_id` is one namespace across every merchant and the
        // dashboard combined (crate::oauth's module docs) — checked before
        // either kind's own rules so a duplicate is reported as exactly
        // that, not as whichever per-kind rule happens to run first.
        let mut seen_client_ids = BTreeSet::new();
        for merchant in &self.merchant_clients {
            if !seen_client_ids.insert(merchant.client_id.as_str()) {
                return Err(ConfigError::DuplicateClientId(merchant.client_id.clone()));
            }
        }

        // Tenancy, not identity: two credentials mapping to one
        // `merchant_id` would be two clients that can read and cancel each
        // other's payment intents, and `/v1` has no second check that would
        // notice (see `MerchantClient::merchant_id`). Checked over merchants
        // only — the dashboard client has no `merchant_id` and is not a
        // tenant.
        let mut seen_merchant_ids = BTreeSet::new();
        for merchant in &self.merchant_clients {
            if !seen_merchant_ids.insert(merchant.merchant_id.as_str()) {
                return Err(ConfigError::DuplicateMerchantId(
                    merchant.merchant_id.clone(),
                ));
            }
        }
        if let Some(dashboard) = &self.dashboard_client
            && !seen_client_ids.insert(dashboard.client_id.as_str())
        {
            return Err(ConfigError::DuplicateClientId(dashboard.client_id.clone()));
        }

        // Uniqueness across *every* merchant, before any single key's own
        // shape is checked, for the reason the provider-code walk above runs
        // in that order: otherwise the first duplicate's own malformation
        // would be reported instead of the collision, and an operator would
        // fix the named problem and hit the real one on the next boot.
        let mut seen_publishable_keys: BTreeMap<&str, &str> = BTreeMap::new();
        for merchant in &self.merchant_clients {
            for key in &merchant.publishable_keys {
                if let Some(first) =
                    seen_publishable_keys.insert(key.as_str(), merchant.client_id.as_str())
                {
                    return Err(ConfigError::DuplicatePublishableKey {
                        key: key.clone(),
                        first: first.to_owned(),
                        second: merchant.client_id.clone(),
                    });
                }
            }
        }

        for merchant in &self.merchant_clients {
            validate_merchant_client(merchant)?;
            validate_publishable_keys(merchant, livemode)?;
            validate_webhook_endpoints(merchant, livemode, raw_secrets)?;
        }
        if let Some(dashboard) = &self.dashboard_client {
            validate_dashboard_client(dashboard, livemode)?;
        }

        Ok(())
    }
}

/// The pre-resolution text of every secret in the document: each
/// `providers[].credentials` value, keyed by `(provider code, credential
/// key)`, and each `merchant_clients[].webhooks[].secrets` entry, keyed by
/// `(client id, endpoint id)` and held in configuration order.
///
/// # Why the webhook half is here and not checked at the value
///
/// It is the same question `providers[].credentials` asks — *was this
/// written as a `${VAR}` reference?* — and it stops being answerable the
/// moment step 2 has run, for exactly the same reason. Before Step 5 the
/// walk covered only `providers`, so a livemode config with
/// `secrets: [whsec_literal]` booted clean
/// (`docs/plans/2026-09-03-step5-webhooks.md`, S3).
///
/// # Why a side table and not a field on [`ProviderHost`]
///
/// `ProviderHost` is the *resolved* configuration — it is what a rail
/// adapter is eventually handed, and every consumer of it wants the value,
/// not the placeholder. Carrying both halves on it would put a second,
/// almost-identical map in front of every caller (and in `Debug`, where a
/// literal secret would then be printed by whichever of the two nobody
/// remembered to redact). This map exists for the length of one
/// [`Config::load_with_env`] call and is dropped when validation ends.
///
/// It is read from the *merged* document, so a credential supplied by a
/// profile overlay is the one checked — the overlay is what a livemode
/// deployment actually edits.
#[derive(Debug, Default)]
struct RawSecrets {
    /// `(providers[].code, credential key)` -> the value as written.
    provider_credentials: BTreeMap<(String, String), String>,
    /// `(merchant_clients[].client_id, webhooks[].id)` -> every `secrets:`
    /// entry as written, in configuration order.
    ///
    /// A `Vec` keyed by the endpoint rather than a map keyed by
    /// `(client, endpoint, index)`, because the rule is applied by walking
    /// the *resolved* endpoint's `secrets` with `enumerate()` and asking
    /// this table for the same position. Positional matching is what makes a
    /// resolved value traceable back to its own text at all: two secrets on
    /// one endpoint are two different strings with no key to tell them apart.
    webhook_secrets: BTreeMap<(String, String), Vec<String>>,
}

impl RawSecrets {
    /// Walks the merged, **unresolved** document for
    /// `providers[].credentials` and `merchant_clients[].webhooks[].secrets`.
    ///
    /// Every shape mismatch is skipped rather than reported: this runs
    /// before `serde` has had a chance to say the document is not a
    /// `Config` at all, so a malformed `providers` block must not produce a
    /// worse error message here than the shape error the caller is about to
    /// get anyway. A skipped entry is not a silently passed one — see
    /// [`Self::provider_credential_as_written`].
    fn from_document(document: &figment::value::Value) -> Self {
        Self {
            provider_credentials: Self::provider_credentials_from(document),
            webhook_secrets: Self::webhook_secrets_from(document),
        }
    }

    fn provider_credentials_from(
        document: &figment::value::Value,
    ) -> BTreeMap<(String, String), String> {
        use figment::value::Value;

        let mut out = BTreeMap::new();
        let Some(providers) = document.find_ref("providers").and_then(Value::as_array) else {
            return out;
        };
        for provider in providers {
            let Some(dict) = provider.as_dict() else {
                continue;
            };
            let Some(code) = dict.get("code").and_then(Value::as_str) else {
                continue;
            };
            let Some(credentials) = dict.get("credentials").and_then(Value::as_dict) else {
                continue;
            };
            for (key, value) in credentials {
                if let Some(text) = value.as_str() {
                    out.insert((code.to_owned(), key.clone()), text.to_owned());
                }
            }
        }
        out
    }

    /// The same walk for webhook secrets.
    ///
    /// A non-string entry (`secrets: [{}]`, `secrets: [7]`) is pushed as an
    /// empty string rather than skipped, so the *positions* still line up
    /// with the resolved list: dropping it would shift every later secret
    /// onto its neighbour's text and check the wrong value against the wrong
    /// rule. An empty string fails the livemode rule, which is the fail-closed
    /// answer for a value this walk could not read.
    fn webhook_secrets_from(
        document: &figment::value::Value,
    ) -> BTreeMap<(String, String), Vec<String>> {
        use figment::value::Value;

        let mut out = BTreeMap::new();
        let Some(clients) = document
            .find_ref("merchant_clients")
            .and_then(Value::as_array)
        else {
            return out;
        };
        for client in clients {
            let Some(dict) = client.as_dict() else {
                continue;
            };
            let Some(client_id) = dict.get("client_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(endpoints) = dict.get("webhooks").and_then(Value::as_array) else {
                continue;
            };
            for endpoint in endpoints {
                let Some(endpoint) = endpoint.as_dict() else {
                    continue;
                };
                let Some(endpoint_id) = endpoint.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(secrets) = endpoint.get("secrets").and_then(Value::as_array) else {
                    continue;
                };
                out.insert(
                    (client_id.to_owned(), endpoint_id.to_owned()),
                    secrets
                        .iter()
                        .map(|secret| secret.as_str().unwrap_or_default().to_owned())
                        .collect(),
                );
            }
        }
        out
    }

    /// A rail credential as the file wrote it, or `None` if this map never
    /// saw it.
    ///
    /// `None` is not "no rule applies": the caller passes it to
    /// `validate_secret` as an empty string, which fails the livemode rule.
    /// A credential this function cannot account for is one nobody can
    /// prove came from a `${VAR}`, and the whole point of the rule is that
    /// a livemode deployment does not start until that proof exists.
    fn provider_credential_as_written(&self, code: &str, key: &str) -> Option<&str> {
        self.provider_credentials
            .get(&(code.to_owned(), key.to_owned()))
            .map(String::as_str)
    }

    /// One webhook secret as the file wrote it, by position. `None` fails
    /// closed for [`Self::provider_credential_as_written`]'s reason.
    fn webhook_secret_as_written(
        &self,
        client_id: &str,
        endpoint_id: &str,
        index: usize,
    ) -> Option<&str> {
        self.webhook_secrets
            .get(&(client_id.to_owned(), endpoint_id.to_owned()))?
            .get(index)
            .map(String::as_str)
    }
}

#[cfg(test)]
impl RawSecrets {
    /// The identity map, for a [`Config`] built in memory by a test rather
    /// than loaded from a file.
    ///
    /// Sound only because nothing resolved anything: a hand-built `Config`'s
    /// credential *is* its own text. A test that wants to exercise the
    /// resolution ordering has to go through [`Config::load_with_env`] and a
    /// fixture file — which is what
    /// `a_livemode_config_whose_placeholders_resolve_loads` does, and why
    /// this shortcut cannot be used to fake that proof.
    fn identity(config: &Config) -> Self {
        Self {
            provider_credentials: config
                .providers
                .iter()
                .flat_map(|provider| {
                    provider
                        .credentials
                        .iter()
                        .map(|(key, value)| ((provider.code.clone(), key.clone()), value.clone()))
                })
                .collect(),
            webhook_secrets: config
                .merchant_clients
                .iter()
                .flat_map(|client| {
                    client.webhooks.iter().map(|endpoint| {
                        (
                            (client.client_id.clone(), endpoint.id.clone()),
                            endpoint.secrets.clone(),
                        )
                    })
                })
                .collect(),
        }
    }
}

/// Refuses a rail whose YAML omits a key its adapter cannot work without —
/// see [`REQUIRED_RAIL_KEYS`] for the table and why it lives here.
///
/// A rail with no row in the table is not an error: a deployment may
/// configure a code this binary has no adapter for, and that is
/// [`ConfigError::ProviderWithoutAdapter`]'s job to report, from the binary
/// that knows what it links. Reporting it twice, differently, would make the
/// first message the one an operator reads and the wrong one.
///
/// An *empty* value counts as missing. A `${VAR}` that resolved to an empty
/// string cannot reach here (step 2 makes an unresolved placeholder fatal),
/// but a literal `api_key: ""` in a sandbox file can, and it fails on the
/// wire in a way that names neither the key nor the file.
fn validate_required_rail_keys(provider: &ProviderHost) -> Result<(), ConfigError> {
    let Some(required) = REQUIRED_RAIL_KEYS
        .iter()
        .find(|entry| entry.code == provider.code)
    else {
        return Ok(());
    };

    for key in required.settings {
        if provider
            .settings
            .get(*key)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(ConfigError::MissingProviderSetting {
                code: provider.code.clone(),
                section: "settings",
                key: (*key).to_owned(),
            });
        }
    }
    for key in required.credentials {
        if provider
            .credentials
            .get(*key)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(ConfigError::MissingProviderSetting {
                code: provider.code.clone(),
                section: "credentials",
                key: (*key).to_owned(),
            });
        }
    }
    Ok(())
}

/// ADR-0010's merchant-client rules: no client secret, ever; a non-empty,
/// non-degenerate JWK set (`private_key_jwt` with no key can never
/// authenticate); `client_credentials` as the only permitted grant; and
/// [`MERCHANT_AUDIENCE`] present in `allowed_audiences`, without which the
/// client's `/v1` tokens are unusable in a way nothing downstream can
/// diagnose (see [`ConfigError::MerchantMissingV1Audience`]).
///
/// The audience rule is checked last on purpose: it is the only one of the
/// four that is about what a *correctly shaped* registration would go on to
/// do at runtime, so a config that is malformed in a more basic way should
/// be reported as that instead.
fn validate_merchant_client(merchant: &MerchantClient) -> Result<(), ConfigError> {
    if merchant.client_secret.is_some() {
        return Err(ConfigError::ClientSecretPresent(merchant.client_id.clone()));
    }
    if !jwks_has_at_least_one_key(&merchant.jwks) {
        return Err(ConfigError::EmptyMerchantJwks(merchant.client_id.clone()));
    }
    for grant in &merchant.grant_types {
        if *grant != GrantType::ClientCredentials {
            return Err(ConfigError::DisallowedMerchantGrant {
                client_id: merchant.client_id.clone(),
                grant: *grant,
            });
        }
    }
    if !merchant
        .allowed_audiences
        .iter()
        .any(|audience| audience == MERCHANT_AUDIENCE)
    {
        return Err(ConfigError::MerchantMissingV1Audience {
            client_id: merchant.client_id.clone(),
        });
    }
    Ok(())
}

/// The two prefixes a publishable key may carry, paired with the
/// `deployment.livemode` each one belongs to.
///
/// A table rather than two `if`s so that the shape rule and the livemode rule
/// read the same value: a third prefix added here is refused by
/// [`validate_publishable_keys`] until this table names which deployment it
/// belongs to, rather than silently passing the shape check and failing the
/// livemode one for a reason nobody wrote down.
const PUBLISHABLE_KEY_PREFIXES: [(&str, bool); 2] = [("pk_test_", false), ("pk_live_", true)];

/// How many `[A-Za-z0-9]` characters must follow a publishable key's prefix.
///
/// Stripe's own keys sit comfortably inside this range, and so does anything
/// an operator would generate with `openssl rand -hex 16`. The floor is not a
/// security bound — the key authorises nothing on its own
/// ([`MerchantClient::publishable_keys`]) — it is a *typo* bound: `pk_test_x`
/// is a truncated paste, and refusing it at boot is cheaper than a merchant
/// discovering it as a uniform 404 on their checkout page.
const PUBLISHABLE_KEY_BODY_CHARS: std::ops::RangeInclusive<usize> = 16..=64;

/// One merchant's publishable keys (D1 of
/// `docs/plans/2026-09-03-step5c-stripejs.md`): each shaped
/// `pk_(test|live)_[A-Za-z0-9]{16,64}`, and each agreeing with
/// `deployment.livemode`.
///
/// Uniqueness is **not** here: it is a property of the whole
/// `merchant_clients` list and is checked in `Config::validate_all`, for the
/// same reason `client_id` uniqueness is.
///
/// # Why the shape is hand-checked rather than a regex
///
/// The pattern is a fixed prefix plus a bounded run of ASCII alphanumerics,
/// which is three lines of `char` predicates. Taking the `regex` crate as a
/// dependency of `vpay-config` — which has none today, and whose whole job is
/// to run once at boot — to express that would be a compile-time and
/// supply-chain cost for a pattern with no alternation, no backtracking and
/// no Unicode classes in it. The escape hatch matters too: this way the
/// message can name *which* half is wrong.
///
/// # Errors
///
/// [`ConfigError::MalformedPublishableKey`] for a key that is not shaped like
/// one, and [`ConfigError::PublishableKeyLivemodeMismatch`] for a well-shaped
/// key whose marker contradicts the deployment.
fn validate_publishable_keys(merchant: &MerchantClient, livemode: bool) -> Result<(), ConfigError> {
    for key in &merchant.publishable_keys {
        // Shape first, then the label: a value that is not a publishable key
        // at all has no marker to contradict anything, and reporting the
        // livemode rule for `pk_tset_…` would send an operator looking at
        // `deployment.livemode` instead of at their typo.
        let Some((_, key_is_live)) = PUBLISHABLE_KEY_PREFIXES
            .iter()
            .find_map(|(prefix, is_live)| key.strip_prefix(prefix).map(|body| (body, *is_live)))
            .filter(|(body, _)| {
                PUBLISHABLE_KEY_BODY_CHARS.contains(&body.chars().count())
                    && body.chars().all(|c| c.is_ascii_alphanumeric())
            })
        else {
            return Err(ConfigError::MalformedPublishableKey {
                client_id: merchant.client_id.clone(),
                key: key.clone(),
            });
        };
        if key_is_live != livemode {
            return Err(ConfigError::PublishableKeyLivemodeMismatch {
                client_id: merchant.client_id.clone(),
                key: key.clone(),
                livemode,
            });
        }
    }
    Ok(())
}

/// One merchant's webhook endpoints (`docs/flows/webhooks.md`): a non-empty
/// `id`, unique within this merchant; a URL that names a host and survives
/// [`validate_webhook_url`]; one or two non-empty signing secrets, each
/// written as a `${VAR}` under livemode.
///
/// # Order, and why it is this one
///
/// Identity first (`id`, then uniqueness), because every later message names
/// the endpoint by its id and an endpoint with no usable id has nothing to be
/// named by. Then the destination, then the credential — the same order
/// `validate_all` walks a rail in, and for the same reason: an endpoint
/// nobody can reach makes the question of what it is signed with moot.
///
/// # The URL is parsed first, and the livemode rules are asked of the parse
///
/// A rail host goes through [`validate_host`], which is two substring tests
/// over a raw string. A webhook endpoint is a whole URL rather than an
/// origin, and those tests are wrong for one:
/// `starts_with("https://")` refuses `HTTPS://Hooks.Example/x`, and a
/// stub-marker search over the whole URL refuses a merchant's own
/// `https://hooks.example/mockups`. So the URL is parsed once and the two
/// livemode rules are asked of the parse, by [`validate_webhook_url`] —
/// scheme, and stub markers over the host alone. `validate_host` is left
/// exactly as it is for the rails; nothing here weakens it.
///
/// Parsing also answers what neither substring test can. A value that is not
/// a URL is [`ConfigError::WebhookUrlUnparseable`]; one carrying userinfo is
/// [`ConfigError::WebhookUrlHasCredentials`] (a webhook URL is stored on
/// every delivery row and printed by `EndpointRegistry`'s `Debug`, so it must
/// never be a secret); one naming no host is
/// [`ConfigError::WebhookUrlMissingHost`], **in both deployments** — see
/// that variant. The alternative to all of it is a boot that succeeds and a
/// delivery that fails once per attempt per event, recorded only as a
/// `webhook_deliveries` row walking the ladder to `exhausted`.
///
/// # The length bounds are migration 0022's, and they are not livemode rules
///
/// `endpoint_id` and `url` are written into `webhook_deliveries` by the
/// fan-out's per-event transaction, under `endpoint_id_length` and
/// `url_length` CHECKs. A value outside them is refused by Postgres *inside
/// that transaction* — hours after a healthy-looking boot, on an event that
/// then stays `fanout_state = 'pending'` forever while every pass re-raises
/// the same failure. Mirroring the CHECKs here moves that to the boot that
/// introduced it. Both bounds hold in sandbox exactly as in livemode,
/// because the constraint they mirror does.
///
/// # The literal-secret rule reads the file, not the value
///
/// `raw_secrets` carries the pre-resolution text (see [`RawSecrets`]); the
/// resolved `endpoint.secrets[position]` is deliberately never passed to
/// [`validate_secret`], because by then a correctly written `${VAR}` and a
/// literal are the same string. A position the table cannot account for
/// fails closed as `""`.
fn validate_webhook_endpoints(
    merchant: &MerchantClient,
    livemode: bool,
    raw_secrets: &RawSecrets,
) -> Result<(), ConfigError> {
    let mut seen_ids = BTreeSet::new();

    for (index, endpoint) in merchant.webhooks.iter().enumerate() {
        if endpoint.id.trim().is_empty() {
            return Err(ConfigError::WebhookEndpointMissingId {
                client_id: merchant.client_id.clone(),
                index,
            });
        }
        if !seen_ids.insert(endpoint.id.as_str()) {
            return Err(ConfigError::DuplicateWebhookEndpointId {
                client_id: merchant.client_id.clone(),
                endpoint_id: endpoint.id.clone(),
            });
        }
        // Characters, not bytes: `char_length` is what the CHECK counts.
        let id_chars = endpoint.id.chars().count();
        if id_chars > *WEBHOOK_ENDPOINT_ID_CHARS.end() {
            return Err(ConfigError::WebhookEndpointIdTooLong {
                client_id: merchant.client_id.clone(),
                endpoint_id: endpoint.id.clone(),
                length: id_chars,
                max: *WEBHOOK_ENDPOINT_ID_CHARS.end(),
            });
        }

        // Emptiness before length before parsing, so each answer is about the
        // defect an operator can act on: a lost value, an over-long one, and
        // a value that is not a URL are three different edits.
        if endpoint.url.trim().is_empty() {
            return Err(ConfigError::WebhookUrlMissing {
                client_id: merchant.client_id.clone(),
                endpoint_id: endpoint.id.clone(),
            });
        }
        let url_chars = endpoint.url.chars().count();
        if url_chars > *WEBHOOK_URL_CHARS.end() {
            return Err(ConfigError::WebhookUrlTooLong {
                client_id: merchant.client_id.clone(),
                endpoint_id: endpoint.id.clone(),
                length: url_chars,
                max: *WEBHOOK_URL_CHARS.end(),
            });
        }
        let parsed =
            url::Url::parse(&endpoint.url).map_err(|error| ConfigError::WebhookUrlUnparseable {
                client_id: merchant.client_id.clone(),
                endpoint_id: endpoint.id.clone(),
                reason: error.to_string(),
            })?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(ConfigError::WebhookUrlHasCredentials {
                client_id: merchant.client_id.clone(),
                endpoint_id: endpoint.id.clone(),
            });
        }
        // In **both** deployments, and before the livemode rules so a
        // `file:///…` is reported as the hostless URL it is rather than as
        // the `InsecureHost` the scheme rule would answer with a line later.
        //
        // Not a livemode rule, for the reason the length bounds are not: a
        // `file:///var/spool/webhooks` or a `mailto:ops@example` has nowhere
        // to POST to in a sandbox either, and letting it boot there means
        // the failure is discovered by a sandbox delivery walking the retry
        // ladder to `exhausted` instead of by the deploy that introduced it.
        if parsed.host().is_none() {
            return Err(ConfigError::WebhookUrlMissingHost {
                client_id: merchant.client_id.clone(),
                endpoint_id: endpoint.id.clone(),
            });
        }

        // `validate_webhook_url`, not `validate_host`: the rail's version
        // tests `starts_with("https://")` (case-sensitive, and a scheme is
        // not) and searches the whole URL for stub markers (so a legitimate
        // `/mockups` path is refused). See its doc comment.
        validate_webhook_url(&parsed, &endpoint.url, livemode)?;

        if !WEBHOOK_SECRET_COUNT.contains(&endpoint.secrets.len()) {
            return Err(ConfigError::WebhookSecretCount {
                client_id: merchant.client_id.clone(),
                endpoint_id: endpoint.id.clone(),
                count: endpoint.secrets.len(),
            });
        }
        for (position, secret) in endpoint.secrets.iter().enumerate() {
            // Emptiness is asked of the *resolved* value, because that is
            // what would be handed to HMAC. A `${VAR}` that resolved to ""
            // cannot reach here (step 2 is fatal on an unset variable), but a
            // literal `""` in a sandbox file can — and it signs perfectly
            // well, with a secret anybody can also use.
            if secret.trim().is_empty() {
                return Err(ConfigError::EmptyWebhookSecret {
                    client_id: merchant.client_id.clone(),
                    endpoint_id: endpoint.id.clone(),
                    index: position,
                });
            }
            // The value **as written**, never `secret` — that one has been
            // through step 2.
            validate_secret(
                &format!(
                    "{}.webhooks.{}.secrets[{position}]",
                    merchant.client_id, endpoint.id
                ),
                raw_secrets
                    .webhook_secret_as_written(&merchant.client_id, &endpoint.id, position)
                    .unwrap_or(""),
                livemode,
            )?;
            // Strength is asked of the *resolved* value, and only that one
            // can answer it: `${MERCHANT_WEBHOOK_SECRET}` is nine characters
            // of placeholder whatever it holds. The pair is the point — the
            // rule above says the secret came from the environment, this one
            // says the environment holds something worth having.
            if livemode && secret.len() < MIN_LIVEMODE_WEBHOOK_SECRET_BYTES {
                return Err(ConfigError::WeakWebhookSecret {
                    client_id: merchant.client_id.clone(),
                    endpoint_id: endpoint.id.clone(),
                    index: position,
                    length: secret.len(),
                    min: MIN_LIVEMODE_WEBHOOK_SECRET_BYTES,
                });
            }
        }
    }

    Ok(())
}

/// How many signing secrets one endpoint may declare — see
/// [`ConfigError::WebhookSecretCount`] for why both ends are closed.
const WEBHOOK_SECRET_COUNT: std::ops::RangeInclusive<usize> = 1..=2;

/// `webhook_deliveries.endpoint_id`'s `endpoint_id_length` CHECK, in
/// characters (migration 0022).
///
/// Transcribed rather than derived: the database is the thing that actually
/// closes the bound, and `the_length_bounds_are_migration_0022s` reads both
/// this constant and the migration file so a divergence fails the build
/// instead of a fan-out transaction.
pub(crate) const WEBHOOK_ENDPOINT_ID_CHARS: std::ops::RangeInclusive<usize> = 1..=64;

/// `webhook_deliveries.url`'s `url_length` CHECK, in characters (migration
/// 0022). Pinned exactly as [`WEBHOOK_ENDPOINT_ID_CHARS`] is.
pub(crate) const WEBHOOK_URL_CHARS: std::ops::RangeInclusive<usize> = 1..=2048;

/// The shortest livemode webhook signing secret this deployment will boot
/// with, in bytes.
///
/// Thirty-two, which is HMAC-SHA256's own output length: RFC 2104 §3 puts
/// the floor for an HMAC key at the hash's output size, and beyond it a
/// longer key adds nothing. Below it an offline search over the recorded
/// `t.body` and its `v1=` digest gets cheap — and what that search recovers
/// is the ability to *forge* a `payment_intent.succeeded` a merchant's
/// handler will believe.
///
/// Bytes rather than characters, because bytes are what
/// `vpay_worker::signing::signature_header` feeds to HMAC. A 20-character
/// secret of emoji is 80 bytes of key material and passes; a 31-character
/// ASCII one does not.
pub(crate) const MIN_LIVEMODE_WEBHOOK_SECRET_BYTES: usize = 32;

/// The dashboard client's rules (`docs/flows/dashboard-auth.md`): no client
/// secret, ever; at least one redirect URI (unconditionally — a client that
/// can never redirect can never complete a login, sandbox or not); and,
/// under `livemode`, every redirect URI is `https://` and not stub-labelled.
///
/// The `livemode` half reuses [`validate_host`] rather than reimplementing
/// the same two checks: a redirect URI is, structurally, exactly the kind of
/// host `validate_host` already guards against — a livemode dashboard
/// redirecting to `http://` or to a leftover `localhost`/`wiremock`-looking
/// URL is exactly as dangerous as a stub payment rail reachable in
/// production, for the same reason (the code cannot tell a stub from a real
/// destination, so the boot guard has to). Each redirect URI is wrapped in a
/// throwaway [`HostEntry`] purely to reuse that tested function; the
/// synthetic `label` only feeds `validate_host`'s stub-marker text search; it is not
/// stored anywhere.
fn validate_dashboard_client(
    dashboard: &DashboardClient,
    livemode: bool,
) -> Result<(), ConfigError> {
    if dashboard.client_secret.is_some() {
        return Err(ConfigError::ClientSecretPresent(
            dashboard.client_id.clone(),
        ));
    }
    if dashboard.redirect_uris.is_empty() {
        return Err(ConfigError::DashboardMissingRedirectUri(
            dashboard.client_id.clone(),
        ));
    }
    for uri in &dashboard.redirect_uris {
        let synthetic_host = HostEntry {
            url: uri.clone(),
            label: format!("dashboard-redirect-uri:{}", dashboard.client_id),
        };
        validate_host(&synthetic_host, livemode)?;
    }
    Ok(())
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
            ("MTN_API_USER", "11111111-2222-3333-4444-555555555555"),
            ("ORANGE_MERCHANT_KEY", "merchant-key-test"),
            ("ORANGE_CLIENT_ID", "client-id-test"),
            ("ORANGE_CLIENT_SECRET", "client-secret-test"),
            // `config/application.yml`'s example merchant has carried a
            // webhook endpoint since Step 5; without this, every test that
            // loads the example file fails at step 2 with
            // `UnresolvedPlaceholder` rather than exercising what it is for.
            //
            // Both are at least `MIN_LIVEMODE_WEBHOOK_SECRET_BYTES` long, and
            // that is load-bearing rather than cosmetic: the livemode
            // fixtures resolve through this same table, so a shorter default
            // would make `WeakWebhookSecret` the answer to every livemode
            // webhook case and each of them would pass for the wrong reason.
            (
                "MERCHANT_WEBHOOK_SECRET",
                "whsec-test-32-bytes-of-key-material",
            ),
            (
                "MERCHANT_WEBHOOK_SECRET_NEXT",
                "whsec-test-next-32-bytes-of-key-material",
            ),
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

        let merchant = config
            .merchant_clients
            .iter()
            .find(|m| m.client_id == "acme-cameroon")
            .expect("acme-cameroon merchant client present");
        assert_eq!(merchant.grant_types, vec![GrantType::ClientCredentials]);
        // The tenant, not the credential: `/v1` filters every query by this
        // value, so a config whose merchant lost it would boot with no
        // tenancy boundary at all.
        assert_eq!(merchant.merchant_id, "acme-cameroon");
        assert!(
            jwks_has_at_least_one_key(&merchant.jwks),
            "example merchant jwks should be non-empty"
        );
        assert_eq!(merchant.client_secret, None);

        let dashboard = config
            .dashboard_client
            .as_ref()
            .expect("dashboard_client present");
        assert_eq!(dashboard.client_id, "vpay-dashboard");
        assert_eq!(dashboard.scope, "dashboard:read");
        assert!(!dashboard.redirect_uris.is_empty());
        assert_eq!(dashboard.client_secret, None);
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
        // `deployment.name` differs in the overlay — the rest of the base
        // document must survive the merge untouched.
        assert_eq!(overlaid.deployment.public_base_url, "http://localhost:8080");
        assert_eq!(overlaid.providers.len(), base_only.providers.len());

        // The sandbox overlay also overrides `dashboard_client.redirect_uris`
        // specifically (not the whole `dashboard_client` block) — this
        // proves figment's dict merge is genuinely recursive one level
        // deeper than the top-level `deployment.name` case above: only the
        // one nested field named in the overlay changes, `client_id` and
        // `scope` still come from the base file.
        let base_dashboard = base_only
            .dashboard_client
            .as_ref()
            .expect("base dashboard_client present");
        let overlaid_dashboard = overlaid
            .dashboard_client
            .as_ref()
            .expect("overlaid dashboard_client present");
        assert_eq!(overlaid_dashboard.client_id, base_dashboard.client_id);
        assert_eq!(overlaid_dashboard.scope, base_dashboard.scope);
        assert_ne!(
            overlaid_dashboard.redirect_uris,
            base_dashboard.redirect_uris
        );
        assert_eq!(
            overlaid_dashboard.redirect_uris,
            vec!["http://localhost:3000/dash/v1/callback".to_owned()]
        );
    }

    #[test]
    fn an_unresolved_placeholder_is_a_hard_error_naming_the_variable() {
        let mut overrides = BTreeMap::new();
        // Everything resolves except MTN_API_KEY, which the env fn below
        // will refuse.
        overrides.insert("MTN_SUBSCRIPTION_KEY", "sub-key-test");
        overrides.insert("ORANGE_MERCHANT_KEY", "merchant-key-test");
        // Resolved on purpose: the merged document walks `merchant_clients`
        // before `providers` (figment's dicts are ordered), so leaving the
        // webhook secret unset would make *it* the variable this test names
        // and the assertion below would be about the wrong rule.
        overrides.insert(
            "MERCHANT_WEBHOOK_SECRET",
            "whsec-test-32-bytes-of-key-material",
        );
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
        assert_eq!(
            err,
            ConfigError::LiteralSecret("mtn_momo.api_key".to_owned()),
            "the refusal names the credential, and it is the *credential* rule that fired \
             rather than any of the fixture's other livemode rules"
        );
    }

    /// The other side of the same rule, and the one that had never worked:
    /// a livemode deployment written exactly as the documentation says —
    /// every credential a `${VAR}` — with every variable set, **loads**.
    ///
    /// This is the regression test for a bug that made livemode unbootable.
    /// `validate_secret` asks whether a value is written as a placeholder,
    /// and it used to be asked *after* placeholders had been substituted, so
    /// a correctly written `${MTN_API_KEY}` was `api-key-test` by the time
    /// the rule ran and every livemode boot failed `LiteralSecret`. Both
    /// existing tests passed throughout: the literal fixture failed for the
    /// right reason by accident (a literal is also not a placeholder), and
    /// nothing else in the suite ever loaded a livemode file.
    ///
    /// Reverting the ordering fails exactly here, which is the point.
    #[test]
    fn a_livemode_config_whose_placeholders_resolve_loads() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/livemode-resolved-secret.yml"
        );
        let env = example_env(BTreeMap::new());
        let config = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect("a livemode config whose ${VAR}s all resolve must load");

        assert!(config.deployment.livemode);
        let mtn = config
            .providers
            .first()
            .expect("the fixture configures one rail");
        assert_eq!(
            mtn.credentials.get("api_key").map(String::as_str),
            Some("api-key-test"),
            "the loaded config carries the RESOLVED value — the raw text is only what the \
             literal-secret rule is asked about"
        );
    }

    // ------------------------------------------------- webhook endpoints ---

    /// The smallest `Config` that passes every rule: one merchant, no rails,
    /// no currencies, `livemode: false`.
    ///
    /// A struct literal rather than a file because the cases that use it vary
    /// **one field** and assert on the variant that names it — a fixture per
    /// variation would be a directory of files differing by one line, and the
    /// thing under test would be harder to see, not easier. The rules that
    /// need a real file to prove anything (that a YAML key reaches the
    /// struct at all) use [`load_fixture`] instead, and say so.
    fn valid_config() -> Config {
        Config {
            deployment: Deployment {
                name: "test".to_owned(),
                livemode: false,
                public_base_url: "https://api.vpay.test".to_owned(),
            },
            providers: Vec::new(),
            currencies: Vec::new(),
            merchant_clients: vec![MerchantClient {
                client_id: "acme-cameroon".to_owned(),
                merchant_id: "acme-cameroon-tenant".to_owned(),
                jwks: Some(serde_json::json!({ "keys": [{ "kty": "RSA", "kid": "k1" }] })),
                grant_types: vec![crate::oauth::GrantType::ClientCredentials],
                scopes: vec!["payments:write".to_owned()],
                allowed_audiences: vec![MERCHANT_AUDIENCE.to_owned()],
                client_secret: None,
                webhooks: Vec::new(),
                publishable_keys: Vec::new(),
            }],
            dashboard_client: None,
        }
    }

    /// Loads a fixture under `tests/fixtures/` with the shared example
    /// environment, and returns whatever `Config::load_with_env` answered.
    ///
    /// The fixture *name* rather than a full path so the cases below read as
    /// a table of rules; every one of them is a real file on disk, because a
    /// rule proved against a `Config` built in memory would not prove that
    /// the YAML shape reaches it (`webhooks:` is `#[serde(default)]`, so a
    /// field name typo is silently an empty list).
    fn load_fixture(name: &str) -> Result<Config, ConfigError> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        Config::load_with_env(Some(&path), "does-not-exist", &example_env(BTreeMap::new()))
    }

    /// The rules `validate_webhook_endpoints` enforces, one fixture each.
    ///
    /// Table-driven rather than eight functions because each case is one
    /// file and one expected variant, and a table makes a *missing* rule
    /// visible as a missing row. Each fixture's own header states what its
    /// only defect is — which is what stops a case from passing for the
    /// wrong reason.
    #[test]
    fn every_webhook_endpoint_rule_refuses_its_own_fixture() {
        let client = "acme-cameroon".to_owned();
        let endpoint = "primary".to_owned();

        let cases: Vec<(&str, ConfigError)> = vec![
            (
                "webhook-empty-endpoint-id.yml",
                ConfigError::WebhookEndpointMissingId {
                    client_id: client.clone(),
                    index: 0,
                },
            ),
            (
                "webhook-duplicate-endpoint-id.yml",
                ConfigError::DuplicateWebhookEndpointId {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                },
            ),
            (
                "webhook-no-secrets.yml",
                ConfigError::WebhookSecretCount {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                    count: 0,
                },
            ),
            (
                "webhook-three-secrets.yml",
                ConfigError::WebhookSecretCount {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                    count: 3,
                },
            ),
            (
                "webhook-empty-secret.yml",
                ConfigError::EmptyWebhookSecret {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                    index: 0,
                },
            ),
            (
                "webhook-livemode-insecure-url.yml",
                ConfigError::InsecureHost("http://hooks.acme.example/vpay".to_owned()),
            ),
            (
                "webhook-livemode-stub-url.yml",
                ConfigError::StubHostInLivemode(
                    "https://wiremock-webhook.internal/webhooks".to_owned(),
                ),
            ),
            (
                "webhook-livemode-literal-secret.yml",
                ConfigError::LiteralSecret("acme-cameroon.webhooks.primary.secrets[0]".to_owned()),
            ),
            // The four migration-0022 bounds, in both deployments — the
            // constraint they mirror is not a livemode policy, and a table
            // with only the sandbox half could not show that.
            (
                "webhook-endpoint-id-too-long.yml",
                ConfigError::WebhookEndpointIdTooLong {
                    client_id: client.clone(),
                    endpoint_id: "p".repeat(65),
                    length: 65,
                    max: 64,
                },
            ),
            (
                "webhook-livemode-endpoint-id-too-long.yml",
                ConfigError::WebhookEndpointIdTooLong {
                    client_id: client.clone(),
                    endpoint_id: "p".repeat(65),
                    length: 65,
                    max: 64,
                },
            ),
            (
                "webhook-empty-url.yml",
                ConfigError::WebhookUrlMissing {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                },
            ),
            (
                "webhook-url-too-long.yml",
                ConfigError::WebhookUrlTooLong {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                    length: 2049,
                    max: 2048,
                },
            ),
            (
                "webhook-livemode-url-too-long.yml",
                ConfigError::WebhookUrlTooLong {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                    length: 2049,
                    max: 2048,
                },
            ),
            // The three the URL parser answers and `validate_host`
            // structurally cannot.
            (
                "webhook-unparseable-url.yml",
                ConfigError::WebhookUrlUnparseable {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                    reason: "relative URL without a base".to_owned(),
                },
            ),
            (
                "webhook-livemode-bare-scheme-url.yml",
                ConfigError::WebhookUrlUnparseable {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                    reason: "empty host".to_owned(),
                },
            ),
            (
                "webhook-livemode-hostless-url.yml",
                ConfigError::WebhookUrlMissingHost {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                },
            ),
            // The same rule in sandbox: `mailto:x` has nowhere to POST to in
            // either deployment, so this is not livemode policy. Without this
            // row the rule could go back to being livemode-only and only the
            // row above would still pass.
            (
                "webhook-sandbox-hostless-url.yml",
                ConfigError::WebhookUrlMissingHost {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                },
            ),
            // A stub marker in the *host*, with a clean path — the half of
            // the stub rule that survives searching `host_str` alone rather
            // than the whole URL.
            (
                "webhook-livemode-stub-host-url.yml",
                ConfigError::StubHostInLivemode("https://mock.example/x".to_owned()),
            ),
            (
                "webhook-url-with-credentials.yml",
                ConfigError::WebhookUrlHasCredentials {
                    client_id: client.clone(),
                    endpoint_id: endpoint.clone(),
                },
            ),
        ];

        for (fixture, expected) in cases {
            let error = load_fixture(fixture)
                .err()
                .unwrap_or_else(|| panic!("{fixture} must be refused"));
            assert_eq!(
                error, expected,
                "{fixture} was refused for the wrong reason"
            );
        }
    }

    /// Two livemode URLs that must **load**, both of which the rail-host
    /// rules refused: an uppercase scheme, and a stub word in the path.
    ///
    /// This is the direction the refusal table cannot show. Every row there
    /// passes for a validator that refuses everything, and both of these are
    /// URLs a real merchant writes — `HTTPS://…` because a scheme is
    /// case-insensitive and something upstream normalised it, `/mockups`
    /// because it is a path on their own domain. `webhook-livemode-stub-
    /// host-url.yml` is the counterweight: narrowing the search to the host
    /// and deleting it are different, and only both tests together say so.
    #[test]
    fn a_livemode_endpoint_may_be_uppercase_and_may_have_a_stub_word_in_its_path() {
        let config = load_fixture("webhook-livemode-accepted-urls.yml")
            .expect("an uppercase scheme and a `/mockups` path are ordinary livemode endpoints");

        let urls: Vec<(&str, &str)> = config
            .merchant_clients
            .first()
            .expect("the fixture configures one merchant")
            .webhooks
            .iter()
            .map(|endpoint| (endpoint.id.as_str(), endpoint.url.as_str()))
            .collect();
        assert_eq!(
            urls,
            [
                // Carried through exactly as written: validation parses a
                // copy and never rewrites the value the deliverer will use.
                ("uppercase-scheme", "HTTPS://Hooks.Example/x"),
                ("stub-word-in-path", "https://hooks.example/mockups"),
            ]
        );
    }

    /// The other half of the literal-secret pair: a livemode endpoint whose
    /// `${VAR}`s all resolve must **load**.
    ///
    /// This is the case that catches the mistake the rail credentials
    /// already made once (`livemode-resolved-secret.yml`'s header): running
    /// the rule after step 2 would make every correctly written livemode
    /// deployment unbootable, and the literal fixture above would go on
    /// passing because a literal is also not a placeholder. Both directions,
    /// or neither proves anything.
    #[test]
    fn a_livemode_webhook_secret_written_as_a_placeholder_loads_and_carries_the_resolved_value() {
        let config = load_fixture("webhook-livemode-resolved-secret.yml")
            .expect("a livemode config whose webhook ${VAR}s all resolve must load");

        let endpoint = config
            .merchant_clients
            .first()
            .and_then(|client| client.webhooks.first())
            .expect("the fixture configures one endpoint");
        assert_eq!(endpoint.id, "primary");
        assert_eq!(
            endpoint.secrets,
            vec![
                "whsec-test-32-bytes-of-key-material".to_owned(),
                "whsec-test-next-32-bytes-of-key-material".to_owned()
            ],
            "the loaded config carries the RESOLVED secrets — the raw text is only what the \
             literal-secret rule is asked about"
        );
    }

    /// A livemode secret shorter than the floor is refused, and a
    /// 32-byte one boots.
    ///
    /// Both halves, and neither proves anything alone: a rule implemented as
    /// "refuse every livemode secret" would pass the first assertion, and one
    /// implemented as `Ok(())` would pass the second. The fixture is written
    /// correctly — a `${VAR}`, so `LiteralSecret` is satisfied — and only the
    /// *resolved* value differs between the two runs, which is the whole
    /// distinction this rule adds.
    #[test]
    fn a_livemode_webhook_secret_below_the_floor_is_refused() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/webhook-livemode-weak-secret.yml");

        let weak = example_env(BTreeMap::from([(
            "MERCHANT_WEBHOOK_SECRET".to_owned(),
            "a".to_owned(),
        )]));
        assert_eq!(
            Config::load_with_env(Some(&path), "does-not-exist", &weak)
                .expect_err("a one-byte livemode signing secret must be refused"),
            ConfigError::WeakWebhookSecret {
                client_id: "acme-cameroon".to_owned(),
                endpoint_id: "primary".to_owned(),
                index: 0,
                length: 1,
                min: MIN_LIVEMODE_WEBHOOK_SECRET_BYTES,
            }
        );

        // Exactly at the floor — the boundary, not a comfortable margin.
        let at_the_floor = "x".repeat(MIN_LIVEMODE_WEBHOOK_SECRET_BYTES);
        let strong = example_env(BTreeMap::from([(
            "MERCHANT_WEBHOOK_SECRET".to_owned(),
            at_the_floor.clone(),
        )]));
        let config = Config::load_with_env(Some(&path), "does-not-exist", &strong)
            .expect("a secret at the floor must boot");
        assert_eq!(
            config
                .merchant_clients
                .first()
                .and_then(|client| client.webhooks.first())
                .map(|endpoint| endpoint.secrets.clone()),
            Some(vec![at_the_floor]),
        );

        // And one byte under it does not, which is what makes the comparison
        // `<` rather than `<=`.
        let one_short = example_env(BTreeMap::from([(
            "MERCHANT_WEBHOOK_SECRET".to_owned(),
            "x".repeat(MIN_LIVEMODE_WEBHOOK_SECRET_BYTES - 1),
        )]));
        assert!(matches!(
            Config::load_with_env(Some(&path), "does-not-exist", &one_short),
            Err(ConfigError::WeakWebhookSecret { .. })
        ));
    }

    /// The floor is livemode's alone: a sandbox stack's throwaway secret is
    /// short and must keep booting.
    ///
    /// Without this case the rule could be tightened to every deployment and
    /// nothing here would say that `compose.e2e.yml`, `just gen-demo-keys`
    /// and every developer's local stack had stopped starting.
    #[test]
    fn a_short_sandbox_webhook_secret_still_loads() {
        let config = load_fixture("webhook-empty-endpoint-id.yml");
        // That fixture is refused for its own reason; what matters is that
        // its one-word literal secret is not the reason.
        assert_eq!(
            config.expect_err("the fixture has its own defect"),
            ConfigError::WebhookEndpointMissingId {
                client_id: "acme-cameroon".to_owned(),
                index: 0,
            }
        );

        // And the real one: `sandbox-literal-secret.yml`'s deployment is
        // `livemode: false`, and `config/application.yml`'s example merchant
        // resolves a secret through the environment. Neither is asked for 32
        // bytes.
        let short = example_env(BTreeMap::from([(
            "MERCHANT_WEBHOOK_SECRET".to_owned(),
            "whsec-short".to_owned(),
        )]));
        Config::load_with_env(Some(Path::new(EXAMPLE_BASE)), "does-not-exist", &short)
            .expect("a sandbox deployment's throwaway webhook secret is not held to the floor");
    }

    /// The two length bounds are transcribed from migration 0022, and this
    /// reads the migration to prove it.
    ///
    /// The bounds exist to refuse at boot what the database would otherwise
    /// refuse inside a fan-out transaction, so a constant that drifted from
    /// the CHECK would put the refusal back exactly where it was — except now
    /// with a boot-time rule that looks like it covers the case. Reading the
    /// SQL is the only assertion that cannot drift with it.
    #[test]
    fn the_length_bounds_are_migration_0022s() {
        let sql = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../migrations/0022_create-webhook-deliveries.sql"),
        )
        .expect("migration 0022 is in the tree");

        assert!(
            sql.contains(&format!(
                "CONSTRAINT endpoint_id_length CHECK (char_length(endpoint_id) BETWEEN {} AND {})",
                WEBHOOK_ENDPOINT_ID_CHARS.start(),
                WEBHOOK_ENDPOINT_ID_CHARS.end()
            )),
            "WEBHOOK_ENDPOINT_ID_CHARS no longer matches migration 0022's endpoint_id_length"
        );
        assert!(
            sql.contains(&format!(
                "CONSTRAINT url_length CHECK (char_length(url) BETWEEN {} AND {})",
                WEBHOOK_URL_CHARS.start(),
                WEBHOOK_URL_CHARS.end()
            )),
            "WEBHOOK_URL_CHARS no longer matches migration 0022's url_length"
        );
    }

    /// A merchant with no `webhooks:` key at all is a complete, valid
    /// registration.
    ///
    /// Not a formality: an empty list is what `vpay_worker::webhooks`'s
    /// fan-out treats as "mark this event done with zero deliveries", so a
    /// rule that required at least one endpoint would make webhooks
    /// mandatory for every merchant vpay has.
    #[test]
    fn a_merchant_with_no_webhooks_configured_is_valid() {
        let config = load_fixture("oauth-duplicate-merchant-id.yml");
        // That fixture is refused for its *own* reason (a shared tenant),
        // never for the absent `webhooks:` key.
        assert_eq!(
            config.expect_err("the fixture has its own defect"),
            ConfigError::DuplicateMerchantId("shared-tenant".to_owned())
        );

        // And the example config, which does configure one, loads.
        let example = Config::load_with_env(
            Some(Path::new(EXAMPLE_BASE)),
            "does-not-exist",
            &example_env(BTreeMap::new()),
        )
        .expect("the example config loads");
        let acme = example
            .merchant_clients
            .iter()
            .find(|client| client.client_id == "acme-cameroon")
            .expect("the example merchant is registered");
        assert_eq!(
            acme.webhooks.len(),
            1,
            "config/application.yml's example merchant carries the compose receiver endpoint"
        );
        let endpoint = acme.webhooks.first().expect("one endpoint");
        assert_eq!(endpoint.id, "primary");
        assert_eq!(endpoint.url, "http://wiremock-webhook:8080/webhooks");
        assert_eq!(
            endpoint.secrets,
            vec!["whsec-test-32-bytes-of-key-material".to_owned()],
            "the placeholder in the file resolved through `example_env`"
        );
    }

    /// A literal is fine in a sandbox: the rule is about live money, and a
    /// developer's stub rail has a throwaway key.
    ///
    /// Without this case, "check the raw text" could be implemented as
    /// "refuse every literal" and every local stack would stop booting with
    /// nothing here to say so.
    #[test]
    fn a_sandbox_config_with_a_literal_secret_loads() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sandbox-literal-secret.yml"
        );
        let env = example_env(BTreeMap::new());
        let config = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect("a literal secret outside livemode must load");
        assert!(!config.deployment.livemode);
    }

    /// An unresolved placeholder stays fatal in livemode, and stays fatal as
    /// *itself*.
    ///
    /// The two rules are easy to collapse into one now that both are about
    /// `${VAR}`s, and collapsing them would be a real loss: "you wrote a
    /// literal" and "the variable is not set in this environment" send an
    /// operator to two different places, and only one of them is a file they
    /// can edit.
    #[test]
    fn a_livemode_placeholder_that_does_not_resolve_is_still_the_unresolved_error() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/livemode-resolved-secret.yml"
        );
        // Everything resolves except MTN_API_KEY.
        let env = |key: &str| match key {
            "MTN_API_KEY" => None,
            _ => Some("set".to_owned()),
        };
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("an unset variable must still be fatal");
        assert_eq!(
            err,
            ConfigError::UnresolvedPlaceholder("MTN_API_KEY".to_owned())
        );
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
    fn a_rail_missing_a_required_setting_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/provider-missing-setting.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a rail missing a required setting must be rejected");
        assert_eq!(
            err,
            ConfigError::MissingProviderSetting {
                code: "mtn_momo".to_owned(),
                section: "settings",
                key: "api_user".to_owned(),
            }
        );
    }

    #[test]
    fn a_rail_missing_a_required_credential_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/provider-missing-credential.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a rail missing a required credential must be rejected");
        assert_eq!(
            err,
            ConfigError::MissingProviderSetting {
                code: "orange_money".to_owned(),
                section: "credentials",
                key: "client_secret".to_owned(),
            }
        );
    }

    /// The half a `contains_key` check would miss: an operator who wrote the
    /// key and left the value blank gets the same refusal as one who omitted
    /// it, rather than a 401 from the rail hours later.
    #[test]
    fn a_required_key_present_but_empty_is_treated_as_missing() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/provider-empty-credential.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("an empty required credential must be rejected");
        assert_eq!(
            err,
            ConfigError::MissingProviderSetting {
                code: "orange_money".to_owned(),
                section: "credentials",
                key: "client_secret".to_owned(),
            }
        );
    }

    /// A rail with no row in `REQUIRED_RAIL_KEYS` must load: which rails a
    /// *binary* implements is `ConfigError::ProviderWithoutAdapter`'s
    /// question, raised where the linked adapters are known, and answering it
    /// twice would mean an operator reads the less useful message.
    #[test]
    fn a_rail_this_crate_has_no_key_table_for_is_not_refused_here() {
        let host = ProviderHost {
            code: "some_future_rail".to_owned(),
            enabled: true,
            host: HostEntry {
                url: "https://rail.example".to_owned(),
                label: "future".to_owned(),
            },
            settings: BTreeMap::new(),
            callback_url: None,
            currency: "XAF".to_owned(),
            credentials: BTreeMap::new(),
        };
        assert_eq!(validate_required_rail_keys(&host), Ok(()));
    }

    #[test]
    fn a_rail_currency_outside_the_canonical_table_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/provider-unknown-currency.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a currency vpay-core does not know must be rejected");
        assert_eq!(err, ConfigError::UnknownCurrency("USD".to_owned()));
    }

    #[test]
    fn a_livemode_callback_url_that_is_not_https_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/livemode-insecure-callback-url.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("an http callback URL under livemode must be rejected");
        assert_eq!(
            err,
            ConfigError::InsecureHost(
                "http://callbacks.vpay.example/provider/mtn_momo/callback".to_owned()
            )
        );
    }

    /// The rule applies to the *derived* callback too, which is the form
    /// almost every deployment uses: a livemode deployment whose
    /// `public_base_url` is plaintext would otherwise hand a live rail an
    /// `http://` callback that nothing had checked.
    #[test]
    fn a_livemode_deployment_cannot_derive_a_plaintext_callback_url() {
        let config = Config {
            deployment: Deployment {
                name: "prod".to_owned(),
                livemode: true,
                public_base_url: "http://api.vpay.example".to_owned(),
            },
            providers: vec![ProviderHost {
                code: "mtn_momo".to_owned(),
                enabled: true,
                host: HostEntry {
                    url: "https://proxy.momoapi.mtn.com".to_owned(),
                    label: "mtn-cm-prod".to_owned(),
                },
                settings: BTreeMap::from([
                    ("target_environment".to_owned(), "mtncameroon".to_owned()),
                    ("api_user".to_owned(), "a-uuid".to_owned()),
                ]),
                callback_url: None,
                currency: "EUR".to_owned(),
                credentials: BTreeMap::from([
                    ("subscription_key".to_owned(), "${K}".to_owned()),
                    ("api_key".to_owned(), "${A}".to_owned()),
                ]),
            }],
            currencies: Vec::new(),
            merchant_clients: Vec::new(),
            dashboard_client: None,
        };

        assert_eq!(
            config.validate_all(&RawSecrets::identity(&config)),
            Err(ConfigError::InsecureHost(
                "http://api.vpay.example/provider/mtn_momo/callback".to_owned()
            ))
        );
    }

    /// The projection every rail call is handed. Asserted on the example
    /// config rather than a hand-built struct so the derivation is proven
    /// against the file an operator actually edits.
    #[test]
    fn to_provider_config_projects_the_example_config_onto_the_port() {
        let env = example_env(BTreeMap::new());
        let config = Config::load_with_env(Some(Path::new(EXAMPLE_BASE)), "does-not-exist", &env)
            .expect("example config should load");
        let mtn = config
            .providers
            .iter()
            .find(|p| p.code == "mtn_momo")
            .expect("the example config configures mtn_momo");

        let projected = mtn
            .to_provider_config(&config.deployment)
            .expect("the example config's currency is canonical");

        assert_eq!(projected.base_url, "http://wiremock-mtn:8080");
        // Derived, not configured: `public_base_url` + docs/api/README.md's
        // path.
        assert_eq!(
            projected.callback_url,
            "http://localhost:8080/provider/mtn_momo/callback"
        );
        // EUR, from the rail's profile — MTN's sandbox rejects XAF.
        assert_eq!(projected.currency, Currency::Eur);
        assert_eq!(
            projected
                .settings
                .get("target_environment")
                .map(String::as_str),
            Some("sandbox")
        );
        // The `${MTN_API_KEY}` placeholder is resolved by the time an
        // adapter sees it, never handed on as a literal `${...}`.
        assert_eq!(
            projected.credentials.get("api_key").map(String::as_str),
            Some("api-key-test")
        );
        assert_eq!(
            projected.connect_timeout,
            vpay_provider::DEFAULT_CONNECT_TIMEOUT
        );
        assert_eq!(
            projected.request_timeout,
            vpay_provider::DEFAULT_REQUEST_TIMEOUT
        );
    }

    /// A trailing slash on `public_base_url` must not produce a doubled one
    /// in the callback path — a rail would POST to a URL the router does not
    /// match, and the only symptom would be callbacks that never arrive.
    #[test]
    fn a_derived_callback_url_survives_a_trailing_slash_and_an_override_wins() {
        let deployment = Deployment {
            name: "test".to_owned(),
            livemode: false,
            public_base_url: "https://api.vpay.test/".to_owned(),
        };
        let mut host = ProviderHost {
            code: "orange_money".to_owned(),
            enabled: true,
            host: HostEntry {
                url: "https://rail.example/orange-money-webpay/dev".to_owned(),
                label: "orange".to_owned(),
            },
            settings: BTreeMap::new(),
            callback_url: None,
            currency: "xaf".to_owned(),
            credentials: BTreeMap::new(),
        };

        assert_eq!(
            host.effective_callback_url(&deployment),
            "https://api.vpay.test/provider/orange_money/callback"
        );
        // Lower-case in YAML is accepted: the wire API is lower-case and an
        // operator copying a currency from a request body should not be
        // refused for it.
        assert_eq!(
            host.to_provider_config(&deployment)
                .expect("xaf is canonical once upper-cased")
                .currency,
            Currency::Xaf
        );

        host.callback_url = Some("https://callbacks.vpay.test/orange".to_owned());
        assert_eq!(
            host.effective_callback_url(&deployment),
            "https://callbacks.vpay.test/orange"
        );
    }

    /// The one error `to_provider_config` can return, reachable only for a
    /// `ProviderHost` built in code — a loaded one has already been refused
    /// at boot.
    #[test]
    fn to_provider_config_names_a_currency_it_cannot_parse() {
        let deployment = Deployment {
            name: "test".to_owned(),
            livemode: false,
            public_base_url: "https://api.vpay.test".to_owned(),
        };
        let host = ProviderHost {
            code: "mtn_momo".to_owned(),
            enabled: true,
            host: HostEntry {
                url: "https://rail.example".to_owned(),
                label: "rail".to_owned(),
            },
            settings: BTreeMap::new(),
            callback_url: None,
            currency: "GBP".to_owned(),
            credentials: BTreeMap::new(),
        };

        assert_eq!(
            host.to_provider_config(&deployment),
            Err(ConfigError::UnknownCurrency("GBP".to_owned()))
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
            enabled: true,
            host: HostEntry {
                url: "https://proxy.momoapi.mtn.com".to_owned(),
                label: "mtn-cm-prod".to_owned(),
            },
            settings: BTreeMap::from([(
                "subscription_key_header".to_owned(),
                "Ocp-Apim-Subscription-Key".to_owned(),
            )]),
            callback_url: None,
            currency: "EUR".to_owned(),
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
            enabled: true,
            host: HostEntry {
                url: "https://proxy.momoapi.mtn.com".to_owned(),
                label: "mtn-cm-prod".to_owned(),
            },
            settings: BTreeMap::from([(
                "subscription_key_header".to_owned(),
                "Ocp-Apim-Subscription-Key".to_owned(),
            )]),
            callback_url: None,
            currency: "EUR".to_owned(),
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

        // No assertion message interpolates `formatted`. The subject of this
        // test is a `Debug` string built from a `Config` whose credentials
        // were resolved from the environment, so precisely when one of these
        // assertions fails, `formatted` is the string holding the credential
        // value in cleartext — and a failed test writes its message to the
        // CI log (CodeQL rust/cleartext-logging). Each message names the
        // credential's path in `config/application.yml` instead; that is
        // enough to find the missing redaction, and costs nothing an
        // operator would have been allowed to read anyway.
        assert!(
            !formatted.contains("sub-key-test"),
            "Config Debug leaked providers[mtn_momo].credentials.subscription_key"
        );
        assert!(
            !formatted.contains("api-key-test"),
            "Config Debug leaked providers[mtn_momo].credentials.api_key"
        );
        assert!(
            !formatted.contains("merchant-key-test"),
            "Config Debug leaked providers[orange_money].credentials.merchant_key"
        );
        // Non-secret fields survive the trip through the whole `Config`.
        assert!(
            formatted.contains("vpay"),
            "Config Debug dropped deployment.name"
        );
        assert!(
            formatted.contains("mtn_momo"),
            "Config Debug dropped providers[mtn_momo].code"
        );
        assert!(
            formatted.contains("[redacted]"),
            "Config Debug dropped the [redacted] marker that stands in for a credential value"
        );
    }

    // --- OAuth client validation rules (ADR-0010, docs/flows/dashboard-auth.md) ---

    #[test]
    fn a_duplicate_client_id_across_merchant_and_dashboard_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oauth-duplicate-client-id.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a client_id shared by a merchant and the dashboard must be rejected");
        assert_eq!(err, ConfigError::DuplicateClientId("shared-id".to_owned()));
    }

    /// Two credentials, one tenant. Everything else about both clients is
    /// valid — only the shared `merchant_id` is wrong — so this fires on the
    /// rule under test rather than on whichever check runs first.
    #[test]
    fn two_merchant_clients_sharing_a_merchant_id_are_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oauth-duplicate-merchant-id.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("two clients naming the same tenant must be rejected");
        assert_eq!(
            err,
            ConfigError::DuplicateMerchantId("shared-tenant".to_owned())
        );
    }

    // --- publishable keys (Step 5c D1) ---

    /// One key, two tenants. Every other rule passes, so this fires on the
    /// collision rather than on whichever check runs first.
    ///
    /// Decisive: deleting the uniqueness walk from `validate_all` makes this
    /// config boot, and `merchant_id_for_publishable_key` then answers with
    /// whichever merchant `BTreeMap::insert` saw last.
    #[test]
    fn two_merchant_clients_sharing_a_publishable_key_are_rejected() {
        let err = load_fixture("publishable-key-duplicate.yml")
            .expect_err("one publishable key naming two tenants must be rejected");
        assert_eq!(
            err,
            ConfigError::DuplicatePublishableKey {
                key: "pk_test_sharedbetweentwotenants".to_owned(),
                first: "acme-cameroon".to_owned(),
                second: "beta-douala".to_owned(),
            }
        );
    }

    /// A registration that lists the same key twice is the same defect and
    /// the same refusal — it is one merchant, but it is also a file whose
    /// author believes something that is not true about it.
    ///
    /// Written in memory rather than as a fixture because the shape under
    /// test is a repeated list item, which reads as a copy/paste slip in a
    /// file and as a deliberate case here.
    #[test]
    fn one_merchant_listing_a_publishable_key_twice_is_rejected() {
        let mut config = valid_config();
        config
            .merchant_clients
            .first_mut()
            .expect("the fixture registers one merchant")
            .publishable_keys = vec![
            "pk_test_listedtwicebyoneclient".to_owned(),
            "pk_test_listedtwicebyoneclient".to_owned(),
        ];

        let err = config
            .validate_all(&RawSecrets::default())
            .expect_err("a key listed twice must be rejected");
        assert_eq!(
            err,
            ConfigError::DuplicatePublishableKey {
                key: "pk_test_listedtwicebyoneclient".to_owned(),
                first: "acme-cameroon".to_owned(),
                second: "acme-cameroon".to_owned(),
            }
        );
    }

    /// The fixture's key is a truncated paste. The table below then walks
    /// every *other* way the shape can be wrong against the same rule, in
    /// memory, because one file per near-miss would be twelve files saying
    /// the same thing.
    #[test]
    fn a_malformed_publishable_key_is_rejected() {
        let err = load_fixture("publishable-key-malformed.yml")
            .expect_err("a publishable key with a nine-character body must be rejected");
        assert_eq!(
            err,
            ConfigError::MalformedPublishableKey {
                client_id: "acme-cameroon".to_owned(),
                key: "pk_test_short".to_owned(),
            }
        );
    }

    /// Every near-miss an operator actually produces, and the two values at
    /// the length bound that must be *accepted*.
    ///
    /// The bounds are written as literals rather than derived from
    /// `PUBLISHABLE_KEY_BODY_CHARS`, so widening the constant is a deliberate
    /// change to this test too.
    #[test]
    fn the_publishable_key_shape_admits_exactly_what_it_documents() {
        let sixteen = "a".repeat(16);
        let sixty_four = "b".repeat(64);
        for accepted in [
            format!("pk_test_{sixteen}"),
            format!("pk_test_{sixty_four}"),
        ] {
            let mut config = valid_config();
            config
                .merchant_clients
                .first_mut()
                .expect("one merchant")
                .publishable_keys = vec![accepted.clone()];
            config
                .validate_all(&RawSecrets::default())
                .unwrap_or_else(|e| panic!("{accepted} is inside the documented shape: {e}"));
        }

        for refused in [
            // A body one character short of the floor, and one past the
            // ceiling: the two inputs an off-by-one would let through.
            format!("pk_test_{}", "a".repeat(15)),
            format!("pk_test_{}", "b".repeat(65)),
            // No prefix at all, and the two prefixes a merchant confuses it
            // with — a secret key's, and Stripe's older restricted-key form.
            sixteen.clone(),
            format!("sk_test_{sixteen}"),
            format!("rk_test_{sixteen}"),
            // A transposed prefix, which is what a hand-typed key looks like.
            format!("pk_tset_{sixteen}"),
            // Uppercase prefix: the prefix is matched literally, and a key
            // that differed from the one on the merchant's page by case would
            // resolve to no tenant at request time anyway.
            format!("PK_TEST_{sixteen}"),
            // Characters outside `[A-Za-z0-9]` — the ones a base64 or a
            // hyphenated UUID would bring, and which would then have to
            // survive a query string unchanged.
            format!("pk_test_{}-{}", "a".repeat(8), "b".repeat(8)),
            format!("pk_test_{}+{}", "a".repeat(8), "b".repeat(8)),
            format!("pk_test_{}/{}", "a".repeat(8), "b".repeat(8)),
            // Empty, which is what a `publishable_keys: [""]` list item is.
            String::new(),
        ] {
            let mut config = valid_config();
            config
                .merchant_clients
                .first_mut()
                .expect("one merchant")
                .publishable_keys = vec![refused.clone()];
            let err = config
                .validate_all(&RawSecrets::default())
                .expect_err(&format!("{refused} is not a publishable key"));
            assert_eq!(
                err,
                ConfigError::MalformedPublishableKey {
                    client_id: "acme-cameroon".to_owned(),
                    key: refused.clone(),
                },
                "{refused}"
            );
        }
    }

    /// A `pk_live_` key under `livemode: false` — a merchant pasting what
    /// they believe is a production credential into a sandbox page, or the
    /// reverse, which is the one that takes real money.
    #[test]
    fn a_publishable_key_whose_marker_contradicts_livemode_is_rejected() {
        let err = load_fixture("publishable-key-livemode-mismatch.yml")
            .expect_err("a pk_live_ key under livemode: false must be rejected");
        assert_eq!(
            err,
            ConfigError::PublishableKeyLivemodeMismatch {
                key: "pk_live_wronglabelforasandbox".to_owned(),
                client_id: "acme-cameroon".to_owned(),
                livemode: false,
            }
        );
    }

    /// The other direction, which the fixture above cannot express: a
    /// `pk_test_` key served by a **livemode** deployment. Built in memory
    /// because a livemode fixture would have to satisfy every other livemode
    /// rule (https hosts, `${VAR}` secrets) to reach this one, and would then
    /// be a fixture about those rules.
    #[test]
    fn a_test_publishable_key_is_refused_by_a_livemode_deployment() {
        let mut config = valid_config();
        config.deployment.livemode = true;
        config
            .merchant_clients
            .first_mut()
            .expect("one merchant")
            .publishable_keys = vec!["pk_test_sandboxkeyinproduction".to_owned()];

        let err = config
            .validate_all(&RawSecrets::default())
            .expect_err("a pk_test_ key under livemode: true must be rejected");
        assert_eq!(
            err,
            ConfigError::PublishableKeyLivemodeMismatch {
                key: "pk_test_sandboxkeyinproduction".to_owned(),
                client_id: "acme-cameroon".to_owned(),
                livemode: true,
            }
        );
    }

    /// A merchant with no `publishable_keys:` key at all is a complete,
    /// valid registration — and the one most merchants should have.
    ///
    /// The empty list is the fail-closed default (D1): such a merchant has
    /// no browser surface, and every `/v1/browser` request naming a key this
    /// deployment does not know is the uniform 404. A rule that required one
    /// would make the payer-facing checkout mandatory for every merchant vpay
    /// has.
    #[test]
    fn a_merchant_with_no_publishable_keys_is_valid() {
        // A real file that omits the key entirely, refused for its *own*
        // reason and never for the absent list.
        assert_eq!(
            load_fixture("oauth-duplicate-merchant-id.yml")
                .expect_err("the fixture has its own defect"),
            ConfigError::DuplicateMerchantId("shared-tenant".to_owned())
        );

        let config = valid_config();
        assert!(
            config
                .merchant_clients
                .first()
                .expect("one merchant")
                .publishable_keys
                .is_empty()
        );
        config
            .validate_all(&RawSecrets::default())
            .expect("a merchant with no browser surface is a valid registration");
    }

    /// The other half: a spelled `publishable_keys:` actually reaches the
    /// struct.
    ///
    /// `#[serde(default)]` means a typo in the field name is *also* an empty
    /// list, so "no key was refused" proves nothing on its own — this reads
    /// the value back out of the example file every deployment starts from.
    #[test]
    fn the_example_config_registers_a_publishable_key() {
        let example = Config::load_with_env(
            Some(Path::new(EXAMPLE_BASE)),
            "does-not-exist",
            &example_env(BTreeMap::new()),
        )
        .expect("the example config loads");
        let acme = example
            .merchant_clients
            .iter()
            .find(|client| client.client_id == "acme-cameroon")
            .expect("the example merchant is registered");
        assert_eq!(
            acme.publishable_keys.len(),
            1,
            "config/application.yml's example merchant carries one browser key"
        );
        let key = acme.publishable_keys.first().expect("one key");
        assert!(
            key.starts_with("pk_test_"),
            "the example deployment is livemode: false, so its key must say so: {key}"
        );
    }

    /// `merchant_id` is required, not defaulted from `client_id`: a config
    /// that omits it must fail to load rather than silently invent a tenant.
    ///
    /// Written against an in-memory document rather than a fixture file
    /// because the *absence* of a line is what is under test, and a fixture
    /// that merely forgot it would look like an oversight to the next reader.
    #[test]
    fn a_merchant_client_without_a_merchant_id_does_not_load() {
        let yaml = "\
deployment:
  name: vpay
  livemode: false
  public_base_url: http://localhost:8080

merchant_clients:
  - client_id: acme-cameroon
    jwks:
      keys:
        - kty: RSA
          kid: k1
          n: placeholder
          e: AQAB
    grant_types: [client_credentials]
    allowed_audiences: [\"vpay:v1\"]
";
        let err = serde_yaml_ng::from_str::<Config>(yaml)
            .expect_err("a merchant client with no merchant_id must not deserialize");
        assert!(
            err.to_string().contains("merchant_id"),
            "the error must name the missing field, got: {err}"
        );
    }

    /// A provider with no `enabled:` line is enabled. The opposite default
    /// would silently disable every rail in every config written before the
    /// field existed.
    #[test]
    fn a_provider_with_no_enabled_line_is_enabled() {
        let env = example_env(BTreeMap::new());
        let config = Config::load_with_env(Some(Path::new(EXAMPLE_BASE)), "does-not-exist", &env)
            .expect("example config should load");
        assert!(
            config.providers.iter().all(|p| p.enabled),
            "config/application.yml names no `enabled:`, so every rail must default to enabled"
        );
    }

    /// And an explicit `enabled: false` survives the load — otherwise the
    /// default above would be indistinguishable from ignoring the field.
    #[test]
    fn an_explicitly_disabled_provider_stays_disabled() {
        let yaml = "\
deployment:
  name: vpay
  livemode: false
  public_base_url: http://localhost:8080

providers:
  - code: mtn_momo
    enabled: false
    currency: EUR
    host:
      url: http://wiremock-mtn:8080
      label: mtn-sandbox-wiremock
";
        let config: Config =
            serde_yaml_ng::from_str(yaml).expect("an explicit `enabled: false` deserializes");
        assert_eq!(config.providers.first().map(|p| p.enabled), Some(false));
    }

    #[test]
    fn a_merchant_client_with_an_empty_jwks_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oauth-merchant-empty-jwks.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("an empty JWK set must be rejected");
        assert_eq!(
            err,
            ConfigError::EmptyMerchantJwks("acme-cameroon".to_owned())
        );
    }

    #[test]
    fn a_merchant_client_declaring_a_disallowed_grant_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oauth-merchant-disallowed-grant.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a merchant declaring authorization_code must be rejected");
        assert_eq!(
            err,
            ConfigError::DisallowedMerchantGrant {
                client_id: "acme-cameroon".to_owned(),
                grant: GrantType::AuthorizationCode,
            }
        );
    }

    /// The rule that keeps the three parties named on [`MERCHANT_AUDIENCE`]
    /// in agreement. The fixture's `allowed_audiences: [vpay]` is not a
    /// strawman — it is verbatim what `config/application.yml` shipped until
    /// this rule landed, and it would have booted happily.
    #[test]
    fn a_merchant_client_that_cannot_target_the_v1_audience_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oauth-merchant-missing-v1-audience.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env).expect_err(
            "a merchant whose allowed_audiences omits the /v1 audience must be rejected",
        );
        assert_eq!(
            err,
            ConfigError::MerchantMissingV1Audience {
                client_id: "acme-cameroon".to_owned(),
            }
        );
    }

    /// The real `config/application.yml` must satisfy the rule above, and
    /// must satisfy it by carrying the *constant* — not a second copy of the
    /// same spelling that could drift from it.
    #[test]
    fn the_example_config_registers_its_merchant_for_the_v1_audience() {
        let env = example_env(BTreeMap::new());
        let config = Config::load_with_env(Some(Path::new(EXAMPLE_BASE)), "does-not-exist", &env)
            .expect("example config should load");

        let merchant = config
            .merchant_clients
            .first()
            .expect("the example config registers one merchant client");
        assert!(
            merchant
                .allowed_audiences
                .iter()
                .any(|audience| audience == MERCHANT_AUDIENCE),
            "{:?}",
            merchant.allowed_audiences
        );
    }

    #[test]
    fn a_dashboard_client_with_no_redirect_uris_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oauth-dashboard-missing-redirect-uris.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("an empty redirect_uris must be rejected, even outside livemode");
        assert_eq!(
            err,
            ConfigError::DashboardMissingRedirectUri("vpay-dashboard".to_owned())
        );
    }

    #[test]
    fn a_livemode_dashboard_redirect_uri_that_is_not_https_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oauth-dashboard-livemode-insecure-redirect.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("an http:// redirect_uri under livemode must be rejected");
        assert_eq!(
            err,
            ConfigError::InsecureHost("http://dashboard.vpay.example/callback".to_owned())
        );
    }

    #[test]
    fn a_client_secret_anywhere_in_the_config_is_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oauth-merchant-client-secret-present.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a merchant client_secret must be rejected, never silently ignored");
        assert_eq!(
            err,
            ConfigError::ClientSecretPresent("acme-cameroon".to_owned())
        );
    }

    #[test]
    fn a_dashboard_client_secret_is_also_rejected() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oauth-dashboard-client-secret-present.yml"
        );
        let env = example_env(BTreeMap::new());
        let err = Config::load_with_env(Some(Path::new(path)), "does-not-exist", &env)
            .expect_err("a dashboard client_secret must be rejected too — it is a public client");
        assert_eq!(
            err,
            ConfigError::ClientSecretPresent("vpay-dashboard".to_owned())
        );
    }
}
