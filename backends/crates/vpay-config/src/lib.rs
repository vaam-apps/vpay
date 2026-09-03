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
pub use oauth::{DashboardClient, GrantType, MERCHANT_AUDIENCE, MerchantClient, WebhookEndpoint};
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

    /// `docs/flows/webhooks.md`: an endpoint's `id` is what every
    /// `webhook_deliveries` row stores (migration 0022) and what a runbook
    /// names, so an empty one is a delivery history nobody can attribute.
    ///
    /// The message names the client and the endpoint's *position*, because
    /// there is nothing else to name it by — its id is the missing value.
    #[error(
        "merchant client {client_id} declares a webhook endpoint at position {index} with an \
         empty `id`"
    )]
    WebhookEndpointMissingId {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// Zero-based position in `webhooks:`.
        index: usize,
    },
    /// Two of one merchant's endpoints share an `id`.
    ///
    /// Fatal rather than deduplicated: `webhook_deliveries` is unique on
    /// `(event_id, endpoint_id)`, so the second endpoint's insert is
    /// swallowed by the fan-out's `ON CONFLICT DO NOTHING` and *one* of the
    /// two URLs silently never receives anything — with which one depending
    /// on iteration order. Across two merchants the same id is legal and
    /// normal: the index is already merchant-scoped through `event_id`.
    #[error("merchant client {client_id} declares two webhook endpoints with id `{endpoint_id}`")]
    DuplicateWebhookEndpointId {
        /// The registration the duplicate is in.
        client_id: String,
        /// The id declared twice.
        endpoint_id: String,
    },
    /// An endpoint's `id` is longer than `webhook_deliveries.endpoint_id`'s
    /// `endpoint_id_length` CHECK will accept (migration 0022: 1–64
    /// characters).
    ///
    /// Refused at boot rather than left to Postgres, because of *where* the
    /// database would refuse it: the insert happens inside the fan-out's
    /// per-event transaction, long after the deployment came up healthy. The
    /// event then stays `fanout_state = 'pending'` forever and every pass
    /// re-raises the same failure — a merchant silently receives nothing,
    /// with nothing in the boot log to connect it to. The one place this is
    /// cheap to see is the boot that introduced it.
    ///
    /// The bound is characters, not bytes, because `char_length` is what the
    /// CHECK counts.
    #[error(
        "merchant client {client_id}'s webhook endpoint `{endpoint_id}` has an id of \
         {length} characters; webhook_deliveries.endpoint_id accepts 1 to {max} \
         (migration 0022)"
    )]
    WebhookEndpointIdTooLong {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// The over-long id, as written. Not a secret — it is the string
        /// every delivery row and every runbook query names.
        endpoint_id: String,
        /// Its length in characters.
        length: usize,
        /// The CHECK's ceiling, interpolated so this message and
        /// `config::WEBHOOK_ENDPOINT_ID_CHARS` cannot drift.
        max: usize,
    },
    /// An endpoint declares a blank `url`.
    ///
    /// A separate variant from [`Self::WebhookUrlTooLong`] because it is a
    /// different mistake with a different fix — a `url:` key whose value was
    /// lost, rather than one that is too long — and because a blank URL is
    /// otherwise invisible under `livemode: false`, where
    /// [`validate_host`] returns `Ok` without looking at the string at all.
    #[error("merchant client {client_id}'s webhook endpoint `{endpoint_id}` declares no url")]
    WebhookUrlMissing {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// The endpoint's `id`.
        endpoint_id: String,
    },
    /// An endpoint's `url` is longer than `webhook_deliveries.url`'s
    /// `url_length` CHECK will accept (migration 0022: 1–2048 characters).
    ///
    /// Refused at boot for exactly [`Self::WebhookEndpointIdTooLong`]'s
    /// reason. The URL is **not** interpolated: at over 2048 characters it
    /// would bury the rest of the boot log, and the endpoint id is what an
    /// operator needs to find the line to fix.
    #[error(
        "merchant client {client_id}'s webhook endpoint `{endpoint_id}` has a url of \
         {length} characters; webhook_deliveries.url accepts 1 to {max} (migration 0022)"
    )]
    WebhookUrlTooLong {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// The endpoint's `id`.
        endpoint_id: String,
        /// The URL's length in characters.
        length: usize,
        /// The CHECK's ceiling, interpolated so this message and
        /// `config::WEBHOOK_URL_CHARS` cannot drift.
        max: usize,
    },
    /// An endpoint's `url` is not a URL.
    ///
    /// [`validate_webhook_url`] asks two questions of an already-parsed URL
    /// and cannot be asked anything about a string that is not one: under
    /// `livemode: false` it does not look at the URL at all, and a livemode
    /// `https://` with nothing after it satisfies its scheme test. Both
    /// would boot and then fail once, per attempt, per event, inside the
    /// delivery handler — where the only record is a `webhook_deliveries`
    /// row that walks the retry ladder to `exhausted`. So the parse itself
    /// is a boot rule, in both deployments.
    ///
    /// `reason` is [`url::ParseError`]'s own text (`empty host`, `relative
    /// URL without a base`, ...), which names the defect more precisely than
    /// this crate could restate it.
    #[error(
        "merchant client {client_id}'s webhook endpoint `{endpoint_id}` declares a url that \
         is not a URL: {reason}"
    )]
    WebhookUrlUnparseable {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// The endpoint's `id`.
        endpoint_id: String,
        /// `url::ParseError`'s `Display`.
        reason: String,
    },
    /// An endpoint's `url` carries `user:password@` userinfo.
    ///
    /// Refused rather than stripped or passed through. `reqwest` turns
    /// userinfo into an `Authorization: Basic` header on every attempt, so
    /// the URL a delivery is aimed at would also be a credential store — one
    /// that is copied verbatim into `webhook_deliveries.url` on every row
    /// (migration 0022 denormalises it for forensics), read back by
    /// `GET /__admin/requests`-style tooling and printed by
    /// `EndpointRegistry`'s `Debug`, none of which redact it because none of
    /// them can know it is there. `vpay_worker::webhooks::Endpoint`'s
    /// hand-written `Debug` redacts `secrets` and deliberately keeps `url`
    /// visible, which is only safe if a URL is never a secret.
    ///
    /// Neither the username nor the password is named in the message, for
    /// [`Self::MissingProviderSetting`]'s reason.
    #[error(
        "merchant client {client_id}'s webhook endpoint `{endpoint_id}` declares a url with \
         embedded credentials; a webhook URL is stored, logged and printed in full and must \
         never be a secret"
    )]
    WebhookUrlHasCredentials {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// The endpoint's `id`.
        endpoint_id: String,
    },
    /// An endpoint's `url` names no host, in **either** deployment.
    ///
    /// Reachable for a scheme that permits it — `file:///…`, `mailto:…` —
    /// which parses cleanly and has nowhere to POST to. Checked before
    /// [`validate_webhook_url`], so the message says *that* rather than the
    /// `InsecureHost` the scheme rule would answer with a moment later.
    ///
    /// **Not a livemode rule**, and it used to be — which meant a sandbox
    /// deployment could boot with `mailto:ops@example` as a webhook endpoint
    /// and discover it as a delivery walking the retry ladder to
    /// `exhausted`. A URL with no host is not a policy choice about live
    /// money; it is a value nothing can ever be delivered to, exactly like
    /// the migration-0022 length bounds a few lines above it.
    #[error(
        "merchant client {client_id}'s webhook endpoint `{endpoint_id}` declares a url with \
         no host, which nothing can be delivered to"
    )]
    WebhookUrlMissingHost {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// The endpoint's `id`.
        endpoint_id: String,
    },
    /// A livemode signing secret is shorter than
    /// `config::MIN_LIVEMODE_WEBHOOK_SECRET_BYTES`.
    ///
    /// HMAC-SHA256 keys shorter than the hash's 32-byte output add no
    /// security over a 32-byte one and are what makes an offline guessing
    /// attack cheap — and whoever recovers this key can *forge* a
    /// `payment_intent.succeeded` that a merchant's handler will believe and
    /// ship goods against. The floor is on the **resolved** value, because
    /// that is what is handed to HMAC: [`Self::LiteralSecret`] already
    /// answers "was it written as a `${VAR}`?", and a `${VAR}` holding `a`
    /// satisfies that rule completely.
    ///
    /// Livemode only, for the reason `sandbox-literal-secret.yml` exists: a
    /// sandbox deployment's webhooks carry no money and its secrets are
    /// throwaway strings in a compose file. The non-blank rule
    /// ([`Self::EmptyWebhookSecret`]) still applies there.
    ///
    /// The *value* never appears, and neither does any prefix of it — only
    /// its length, which is the number an operator needs to fix it.
    #[error(
        "merchant client {client_id}'s webhook endpoint `{endpoint_id}` declares a livemode \
         signing secret of {length} bytes at position {index}; at least {min} are required"
    )]
    WeakWebhookSecret {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// The endpoint's `id`.
        endpoint_id: String,
        /// Zero-based position within `secrets:`.
        index: usize,
        /// The resolved secret's length in bytes.
        length: usize,
        /// The floor, interpolated so this message and
        /// `config::MIN_LIVEMODE_WEBHOOK_SECRET_BYTES` cannot drift.
        min: usize,
    },
    /// An endpoint declares no signing secret, or more than two.
    ///
    /// Zero is refused because `vpay_worker::webhooks` will not send an
    /// unsigned webhook — a receiver may not act on one — so an endpoint
    /// with no secret is an endpoint that silently receives nothing. More
    /// than two is refused because every secret is another `v1=` on every
    /// delivery, and a third means a rotation nobody finished: the old
    /// secret is still live and still unrevoked.
    #[error(
        "merchant client {client_id}'s webhook endpoint `{endpoint_id}` declares {count} signing \
         secrets; exactly one, or two during a rotation"
    )]
    WebhookSecretCount {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// The endpoint's `id`.
        endpoint_id: String,
        /// How many `secrets:` entries were found.
        count: usize,
    },
    /// A secret is present but blank.
    ///
    /// Separate from [`Self::WebhookSecretCount`] because it is a different
    /// mistake with a different fix: `secrets: [""]`, or a list item that
    /// lost its value, HMACs to a perfectly well-formed signature that
    /// anyone else can also produce. The *value* never appears in the
    /// message, for the same reason [`Self::MissingProviderSetting`]'s does
    /// not.
    #[error(
        "merchant client {client_id}'s webhook endpoint `{endpoint_id}` declares an empty signing \
         secret at position {index}"
    )]
    EmptyWebhookSecret {
        /// The registration the endpoint belongs to.
        client_id: String,
        /// The endpoint's `id`.
        endpoint_id: String,
        /// Zero-based position within `secrets:`.
        index: usize,
    },
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

/// [`validate_host`]'s two livemode rules, asked of a URL that has already
/// been **parsed** — the webhook endpoints' version.
///
/// # Why a sibling and not the same function
///
/// [`validate_host`] answers both questions with substring tests over a raw
/// `providers[].hosts[]` string, and a rail host entry — a bare origin — is
/// the only shape that can afford them. A webhook endpoint cannot, in two
/// ways that were both real defects:
///
/// * `starts_with("https://")` is case-sensitive and a URL scheme is not, so
///   `HTTPS://Hooks.Example/x` — an ordinary https URL — was refused as
///   [`ConfigError::InsecureHost`]. Here the scheme comes from
///   [`url::Url::scheme`], which is lowercase by construction.
/// * the stub-marker search ran over the **whole URL**, so a livemode
///   endpoint whose path is `/mockups` — a legitimate path on a merchant's
///   own domain — was refused as a stub host. The markers describe a *host*
///   (`wiremock`, `localhost`), so this searches [`url::Url::host_str`] and
///   nothing else. Not the label either: an endpoint's label is synthesised
///   from the client and endpoint ids, so searching it would refuse an
///   endpoint an operator happened to name `mock`.
///
/// The rail path keeps [`validate_host`] unchanged and unweakened. This one
/// is *stricter* on the scheme (`https`, not "starts with `https://`") and
/// narrower only about where a marker may appear.
///
/// Livemode-only, exactly as [`validate_host`] is. The rules that hold in
/// both deployments — a host is present, no userinfo, migration 0022's
/// length bounds — live in `config::validate_webhook_endpoints`, because
/// they are not livemode policy.
///
/// # Errors
///
/// [`ConfigError::InsecureHost`] for any scheme but `https`, and
/// [`ConfigError::StubHostInLivemode`] for a host carrying a stub marker.
/// Both carry `raw`, the URL as the operator wrote it, so the message names
/// the line to edit rather than `url`'s normalisation of it.
pub fn validate_webhook_url(
    parsed: &url::Url,
    raw: &str,
    livemode: bool,
) -> Result<(), ConfigError> {
    if !livemode {
        return Ok(());
    }
    if parsed.scheme() != "https" {
        return Err(ConfigError::InsecureHost(raw.to_owned()));
    }
    // Lowercased rather than trusted to be: `host_str` normalises a domain,
    // but an IPv6 literal or a percent-encoded host reaches here as written.
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if STUB_MARKERS.iter().any(|marker| host.contains(marker)) {
        return Err(ConfigError::StubHostInLivemode(raw.to_owned()));
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

    /// The two rules `validate_webhook_url` answers differently from
    /// [`validate_host`], as a table — each row is a URL `validate_host`
    /// gets wrong, or one both agree on and which is here so a rewrite
    /// cannot quietly stop refusing it.
    ///
    /// `config::every_webhook_endpoint_rule_refuses_its_own_fixture` and
    /// `a_livemode_endpoint_may_be_uppercase_and_may_have_a_stub_word_in_its_path`
    /// prove the same four cases arrive here through a YAML document.
    #[test]
    fn a_webhook_urls_scheme_is_case_insensitive_and_only_its_host_is_searched() {
        let cases: [(&str, Result<(), ConfigError>); 6] = [
            // The scheme is compared as a scheme, not as a prefix.
            ("HTTPS://Hooks.Example/x", Ok(())),
            ("https://hooks.example/vpay", Ok(())),
            // A stub word in the *path* is a merchant's own URL.
            ("https://hooks.example/mockups", Ok(())),
            // …and one in the host is the leftover this rule exists for.
            (
                "https://mock.example/x",
                Err(ConfigError::StubHostInLivemode(
                    "https://mock.example/x".to_owned(),
                )),
            ),
            (
                "https://localhost:9000/hooks",
                Err(ConfigError::StubHostInLivemode(
                    "https://localhost:9000/hooks".to_owned(),
                )),
            ),
            (
                "http://hooks.example/x",
                Err(ConfigError::InsecureHost(
                    "http://hooks.example/x".to_owned(),
                )),
            ),
        ];

        for (raw, expected) in cases {
            let parsed = url::Url::parse(raw).expect("every case is a parseable URL");
            assert_eq!(
                validate_webhook_url(&parsed, raw, true),
                expected,
                "{raw} was judged wrongly in livemode"
            );
            // Sandbox has neither rule: both are livemode policy, and a
            // developer's stack is `http://wiremock-webhook:8080`.
            assert_eq!(validate_webhook_url(&parsed, raw, false), Ok(()), "{raw}");
        }
    }
}
