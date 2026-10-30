//! `auth.methods`: which ways of authenticating a deployment permits
//! (ADR-0037, VPA-1).
//!
//! vpay serves more than one application, and what each of them authenticates
//! with is a property of the deployment, not of the binary. The YAML key is
//!
//! ```yaml
//! auth:
//!   methods: [keypair, machine_tokens, hmac_webhooks, human_login]
//! ```
//!
//! and **absent means all four**: a config written before the key existed gets
//! exactly what it always got. Order is irrelevant, a duplicate is refused,
//! and so are an empty list, a null (`methods:` with every item commented out
//! reads as one) and an unknown name. Absent and null are different things on
//! purpose: the first is an operator who never narrowed the list, the second
//! one who tried to and left it empty, and treating the second as the first
//! would open every door the edit was meant to shut.
//!
//! | Method | What it permits | Its block |
//! |---|---|---|
//! | `keypair` | signed `/v1` requests and sealed answers; the console (`/dash/v1`, `/svc/v1`) | `merchant_clients[].cose_keys`, `dashboard`, `service_clients` |
//! | `machine_tokens` | `Authorization: Bearer` on `/v1` and the `/v1/oauth` token endpoint that mints it; the MCP endpoint | `merchant_clients[].jwks`, `grant_types`, `allowed_audiences` |
//! | `hmac_webhooks` | merchant webhook endpoints signed with a shared secret (`Vpay-Signature`) | `merchant_clients[].webhooks` |
//! | `human_login` | staff sign-in by password and TOTP, and the authorization-code grant that turns it into a `/dash/v1` session; single sign-on through an OpenID Connect provider when `staff_auth.sso` is written | `staff_auth`, `dashboard_client` |
//!
//! # Permitted is not served
//!
//! A method on the list is *permitted*. A method with a block runs only when
//! the block is there, and a block there for a method that is not permitted is
//! refused ([`ConfigError::AuthMethodDisabled`]): dormant configuration is how
//! an operator comes to believe a door is shut that is open, or open that is
//! shut. [`AuthMethods::served`] is the list a client may be told.
//!
//! `human_login` follows `machine_tokens`' rules exactly. It runs when it is
//! permitted **and** both of its blocks are present
//! ([`AuthMethods::human_login_served`]); a block present with the method not
//! permitted is [`ConfigError::AuthMethodDisabled`], and the method named
//! explicitly with a block missing is [`ConfigError::AuthMethodNotConfigured`].
//! The absent default permits it, and a deployment that never wrote its blocks
//! simply does not serve it: `/dash/v1` stays a signed-only surface, the staff
//! sign-in routes are not mounted and `staff add` mints no password. A
//! deployment that wrote **one** of the two blocks has half a login; that is
//! refused as [`ConfigError::AuthMethodNotConfigured`] whether or not
//! `auth.methods` was written, because a half-configured door is the dormant
//! configuration this module exists to refuse.
//!
//! # Rules
//!
//! One [`ConfigError`] variant each:
//!
//! 1. an empty list or a null, [`ConfigError::NoAuthMethods`];
//! 2. an unknown name, [`ConfigError::UnknownAuthMethod`];
//! 3. a duplicate, [`ConfigError::DuplicateAuthMethod`];
//! 4. nothing can authenticate a `/v1` caller (no `keypair` and no
//!    `machine_tokens`), [`ConfigError::NoWayIn`];
//! 5. `deployment.surfaces` names `mcp` without `machine_tokens`,
//!    [`ConfigError::McpNeedsMachineTokens`];
//! 6. `machine_tokens` named explicitly with no merchant that has a `jwks`, or
//!    `human_login` with a block missing (or with only one of its two blocks
//!    written), [`ConfigError::AuthMethodNotConfigured`];
//! 7. a block present for a method that is not permitted,
//!    [`ConfigError::AuthMethodDisabled`] (`staff_auth` and `dashboard_client`
//!    without `human_login`, as `jwks` without `machine_tokens`).
//!
//! The per-merchant half of the same contract lives in `validate_all`'s merchant
//! rules and is stated here because it is the part an operator reads this page
//! for: a merchant needs **a credential**, `cose_keys` or `jwks`
//! ([`ConfigError::MerchantNoCredential`]), and `grant_types` and
//! `allowed_audiences` only if it has a `jwks` and `machine_tokens` is permitted.
//! A merchant with `cose_keys` alone boots under the default and simply cannot
//! mint or present a token.
//!
//! # Examples
//!
//! ```
//! use vpay_config::{AuthConfig, AuthMethod};
//!
//! let absent: AuthConfig = serde_yaml_ng::from_str("{}").expect("an absent key");
//! let all = absent.resolve().expect("absent is always legal");
//! assert!(all.permits(AuthMethod::Keypair) && all.permits(AuthMethod::MachineTokens));
//!
//! let keypair_only: AuthConfig =
//!     serde_yaml_ng::from_str("methods: [keypair]").expect("the vaam shape");
//! let only = keypair_only.resolve().expect("a legal list");
//! assert!(only.permits(AuthMethod::Keypair));
//! assert!(!only.permits(AuthMethod::MachineTokens));
//!
//! // `methods:` with every item commented out is a null: not the default.
//! let null: AuthConfig = serde_yaml_ng::from_str("methods: ~").expect("a null parses");
//! assert!(null.resolve().is_err());
//! ```

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::{ConfigError, EnabledSurfaces};

/// One way of authenticating, as `auth.methods` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// A signed request from a registered Ed25519 key (ADR-0033, ADR-0036).
    Keypair,
    /// A bearer token the deployment's own OP minted from a
    /// `private_key_jwt` assertion (ADR-0010).
    MachineTokens,
    /// A webhook signed with a shared secret (`Vpay-Signature`).
    HmacWebhooks,
    /// A person's password, TOTP or SSO sign-in to the console.
    HumanLogin,
}

impl AuthMethod {
    /// Every method, in the order `auth.methods` documents them.
    pub const ALL: [Self; 4] = [
        Self::Keypair,
        Self::MachineTokens,
        Self::HmacWebhooks,
        Self::HumanLogin,
    ];

    /// The YAML spelling, which is also the discovery document's.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Keypair => "keypair",
            Self::MachineTokens => "machine_tokens",
            Self::HmacWebhooks => "hmac_webhooks",
            Self::HumanLogin => "human_login",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|method| method.as_str() == name)
    }
}

impl fmt::Display for AuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `auth:` block of a config file.
///
/// `methods` is kept as the strings the operator wrote so that an unknown name
/// is a specific [`ConfigError::UnknownAuthMethod`] naming the legal set,
/// rather than a serde shape error; [`Self::resolve`] is the one place they
/// become [`AuthMethod`]s.
///
/// A YAML null is read as an **empty list**, not as absent, so that
/// `methods:` with its items commented out is [`ConfigError::NoAuthMethods`]
/// and not a silent all-four.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AuthConfig {
    /// `None`, absent from the YAML, permits every method. `Some` is exactly
    /// the list, and a null in the YAML is `Some(vec![])`.
    #[serde(
        default,
        deserialize_with = "present_methods",
        skip_serializing_if = "Option::is_none"
    )]
    pub methods: Option<Vec<String>>,
}

/// Reads the key when it is *present*: a list as itself, a null as the empty
/// list. `#[serde(default)]` is what makes an *absent* key `None`, and never
/// reaches this function, so the two stay distinguishable.
fn present_methods<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(
        Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default(),
    ))
}

impl AuthConfig {
    /// Resolves `auth.methods` into the set a deployment permits.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoAuthMethods`], [`ConfigError::UnknownAuthMethod`] and
    /// [`ConfigError::DuplicateAuthMethod`] (rules 1 to 3 in the module docs).
    /// The cross-block rules need the whole [`Config`] and live in
    /// [`validate_auth_methods`]. The set this returns knows nothing of the
    /// blocks, so [`AuthMethods::human_login_served`] is `false`: ask
    /// [`Config::auth_methods`] for the one that does.
    pub fn resolve(&self) -> Result<AuthMethods, ConfigError> {
        let Some(names) = &self.methods else {
            return Ok(AuthMethods {
                permitted: AuthMethod::ALL.into_iter().collect(),
                explicit: false,
                human_login_blocks: false,
            });
        };
        if names.is_empty() {
            return Err(ConfigError::NoAuthMethods);
        }
        let mut permitted = BTreeSet::new();
        for name in names {
            let method = AuthMethod::parse(name)
                .ok_or_else(|| ConfigError::UnknownAuthMethod(name.clone()))?;
            if !permitted.insert(method) {
                return Err(ConfigError::DuplicateAuthMethod(method));
            }
        }
        Ok(AuthMethods {
            permitted,
            explicit: true,
            human_login_blocks: false,
        })
    }
}

/// The resolved, validated set of permitted methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthMethods {
    permitted: BTreeSet<AuthMethod>,
    explicit: bool,
    /// Whether `staff_auth` **and** `dashboard_client` are both present. Set by
    /// [`Config::auth_methods`], the only constructor that has seen the
    /// document; [`AuthConfig::resolve`] leaves it `false`.
    human_login_blocks: bool,
}

impl AuthMethods {
    /// Whether `method` is permitted.
    #[must_use]
    pub fn permits(&self, method: AuthMethod) -> bool {
        self.permitted.contains(&method)
    }

    /// Whether `auth.methods` was written. `false` is the all-four default.
    #[must_use]
    pub fn is_explicit(&self) -> bool {
        self.explicit
    }

    /// Every permitted method, in [`AuthMethod::ALL`] order.
    pub fn permitted(&self) -> impl Iterator<Item = AuthMethod> + '_ {
        AuthMethod::ALL
            .into_iter()
            .filter(|method| self.permits(*method))
    }

    /// Whether `human_login` is **served**: permitted, and both its blocks
    /// (`staff_auth`, `dashboard_client`) present. The one question
    /// the staff routes, the OP's dashboard half, `/dash/v1`'s envelope mode and
    /// `staff add` all ask, so they cannot disagree about it.
    #[must_use]
    pub fn human_login_served(&self) -> bool {
        self.permits(AuthMethod::HumanLogin) && self.human_login_blocks
    }

    /// The permitted methods this deployment actually serves: what
    /// `GET /.well-known/vpay-auth` tells a client.
    ///
    /// Permitted, except that `human_login` also needs its blocks
    /// ([`Self::human_login_served`]). Whether any other method *runs* for a
    /// given merchant still depends on that merchant's blocks (a merchant with
    /// no `jwks` cannot use `machine_tokens` on a deployment that serves it),
    /// so the list is what the deployment offers, not a promise about any one
    /// caller. **`human_login` is the one place this differs from the permitted
    /// list.**
    pub fn served(&self) -> impl Iterator<Item = AuthMethod> + '_ {
        self.permitted().filter(|method| match method {
            AuthMethod::HumanLogin => self.human_login_served(),
            _ => true,
        })
    }
}

impl Config {
    /// Resolves `auth.methods` ([`AuthConfig::resolve`]).
    ///
    /// # Errors
    ///
    /// Rules 1 to 3 of the module docs; unreachable for a `Config` that came
    /// from a file, since `validate_all` ran the same call.
    pub fn auth_methods(&self) -> Result<AuthMethods, ConfigError> {
        let mut methods = self.auth.resolve()?;
        methods.human_login_blocks = self.staff_auth.is_some() && self.dashboard_client.is_some();
        Ok(methods)
    }
}

/// Rules 4 to 7 of the module docs, over the whole document.
///
/// Called first in `Config::validate_all`, after `deployment.surfaces`: a
/// contradiction between what is permitted and what is configured is a
/// property of the deployment, and is better named than whichever per-merchant
/// rule would have tripped on it.
pub(crate) fn validate_auth_methods(
    config: &Config,
    methods: &AuthMethods,
    surfaces: EnabledSurfaces,
) -> Result<(), ConfigError> {
    let keypair = methods.permits(AuthMethod::Keypair);
    let tokens = methods.permits(AuthMethod::MachineTokens);
    let hmac = methods.permits(AuthMethod::HmacWebhooks);

    if !keypair && !tokens {
        return Err(ConfigError::NoWayIn);
    }
    if surfaces.mcp && !tokens {
        return Err(ConfigError::McpNeedsMachineTokens);
    }
    if methods.is_explicit()
        && tokens
        && !config
            .merchant_clients
            .iter()
            .any(|merchant| crate::oauth::jwks_has_at_least_one_key(&merchant.jwks))
    {
        return Err(ConfigError::AuthMethodNotConfigured {
            method: AuthMethod::MachineTokens,
            key: "merchant_clients[].jwks".to_owned(),
        });
    }

    let human = methods.permits(AuthMethod::HumanLogin);
    let disabled =
        |method: AuthMethod, key: String| ConfigError::AuthMethodDisabled { method, key };
    // `human_login`, like `machine_tokens`, is its blocks: both or neither. The
    // method off is rule 7 (a block is dormant configuration); the method
    // permitted is the explicit-naming rule, extended to a half-written pair,
    // because "permitted, one block, so not served" is how an operator comes to
    // believe a sign-in exists that does not.
    if human {
        let staff_auth = config.staff_auth.is_some();
        let dashboard_client = config.dashboard_client.is_some();
        let missing = match (staff_auth, dashboard_client) {
            (true, true) => None,
            (true, false) => Some("dashboard_client"),
            (false, true) => Some("staff_auth"),
            (false, false) => methods.is_explicit().then_some("staff_auth"),
        };
        if let Some(key) = missing {
            return Err(ConfigError::AuthMethodNotConfigured {
                method: AuthMethod::HumanLogin,
                key: key.to_owned(),
            });
        }
    } else {
        if config.staff_auth.is_some() {
            return Err(disabled(AuthMethod::HumanLogin, "staff_auth".to_owned()));
        }
        if config.dashboard_client.is_some() {
            return Err(disabled(
                AuthMethod::HumanLogin,
                "dashboard_client".to_owned(),
            ));
        }
    }
    for merchant in &config.merchant_clients {
        let client = &merchant.client_id;
        if !tokens {
            if merchant.jwks.is_some() {
                return Err(disabled(
                    AuthMethod::MachineTokens,
                    format!("merchant_clients[{client}].jwks"),
                ));
            }
            if !merchant.grant_types.is_empty() {
                return Err(disabled(
                    AuthMethod::MachineTokens,
                    format!("merchant_clients[{client}].grant_types"),
                ));
            }
            if !merchant.allowed_audiences.is_empty() {
                return Err(disabled(
                    AuthMethod::MachineTokens,
                    format!("merchant_clients[{client}].allowed_audiences"),
                ));
            }
        }
        if !keypair && !merchant.cose_keys.is_empty() {
            return Err(disabled(
                AuthMethod::Keypair,
                format!("merchant_clients[{client}].cose_keys"),
            ));
        }
        if !hmac && let Some(endpoint) = merchant.webhooks.first() {
            return Err(disabled(
                AuthMethod::HmacWebhooks,
                format!("merchant_clients[{client}].webhooks[{}]", endpoint.id),
            ));
        }
    }
    if !keypair {
        if config.dashboard.is_some() {
            return Err(disabled(AuthMethod::Keypair, "dashboard".to_owned()));
        }
        if !config.service_clients.is_empty() {
            return Err(disabled(AuthMethod::Keypair, "service_clients".to_owned()));
        }
    }
    Ok(())
}
