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
//! and so are an empty list and an unknown name.
//!
//! | Method | What it permits | Its block |
//! |---|---|---|
//! | `keypair` | signed `/v1` requests and sealed answers; the console (`/dash/v1`, `/svc/v1`) | `merchant_clients[].cose_keys`, `dashboard`, `service_clients` |
//! | `machine_tokens` | `Authorization: Bearer` on `/v1` and the `/v1/oauth` token endpoint that mints it; the MCP endpoint | `merchant_clients[].jwks`, `grant_types`, `allowed_audiences` |
//! | `hmac_webhooks` | merchant webhook endpoints signed with a shared secret (`Vpay-Signature`) | `merchant_clients[].webhooks` |
//! | `human_login` | staff sign-in by password, TOTP or SSO | `staff_auth`, `dashboard_client` (**retired in this release**) |
//!
//! # Permitted is not served
//!
//! A method on the list is *permitted*. A method with a block runs only when
//! the block is there, and a block there for a method that is not permitted is
//! refused ([`ConfigError::AuthMethodDisabled`]): dormant configuration is how
//! an operator comes to believe a door is shut that is open, or open that is
//! shut. [`AuthMethods::served`] is the list a client may be told.
//!
//! `human_login` is the one permitted method nothing serves yet: its blocks
//! are retired keys and its restoration is VPA-3. Until then naming it
//! explicitly is refused ([`ConfigError::AuthMethodUnavailable`]) and the
//! absent default permits it without serving it.
//!
//! # Rules
//!
//! One [`ConfigError`] variant each:
//!
//! 1. an empty list, [`ConfigError::NoAuthMethods`];
//! 2. an unknown name, [`ConfigError::UnknownAuthMethod`];
//! 3. a duplicate, [`ConfigError::DuplicateAuthMethod`];
//! 4. `human_login` named explicitly, [`ConfigError::AuthMethodUnavailable`];
//! 5. nothing can authenticate a `/v1` caller (no `keypair` and no
//!    `machine_tokens`), [`ConfigError::NoWayIn`];
//! 6. `deployment.surfaces` names `mcp` without `machine_tokens`,
//!    [`ConfigError::McpNeedsMachineTokens`];
//! 7. `machine_tokens` named explicitly with no merchant that has a `jwks`,
//!    [`ConfigError::AuthMethodNotConfigured`];
//! 8. a block present for a method that is not permitted,
//!    [`ConfigError::AuthMethodDisabled`].
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AuthConfig {
    /// `None`, absent from the YAML, permits every method. `Some` is exactly
    /// the list.
    #[serde(default)]
    pub methods: Option<Vec<String>>,
}

impl AuthConfig {
    /// Resolves `auth.methods` into the set a deployment permits.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoAuthMethods`], [`ConfigError::UnknownAuthMethod`],
    /// [`ConfigError::DuplicateAuthMethod`] and
    /// [`ConfigError::AuthMethodUnavailable`] (rules 1 to 4 in the module
    /// docs). The cross-block rules need the whole [`Config`] and live in
    /// [`validate_auth_methods`].
    pub fn resolve(&self) -> Result<AuthMethods, ConfigError> {
        let Some(names) = &self.methods else {
            return Ok(AuthMethods {
                permitted: AuthMethod::ALL.into_iter().collect(),
                explicit: false,
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
        if permitted.contains(&AuthMethod::HumanLogin) {
            return Err(ConfigError::AuthMethodUnavailable(AuthMethod::HumanLogin));
        }
        Ok(AuthMethods {
            permitted,
            explicit: true,
        })
    }
}

/// The resolved, validated set of permitted methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthMethods {
    permitted: BTreeSet<AuthMethod>,
    explicit: bool,
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

    /// The permitted methods this release actually serves: what
    /// `GET /.well-known/vpay-auth` tells a client.
    ///
    /// Everything permitted except `human_login`, whose blocks are retired
    /// keys until VPA-3 restores them. **This is the one place that changes
    /// when it does.**
    pub fn served(&self) -> impl Iterator<Item = AuthMethod> + '_ {
        self.permitted()
            .filter(|method| *method != AuthMethod::HumanLogin)
    }
}

impl Config {
    /// Resolves `auth.methods` ([`AuthConfig::resolve`]).
    ///
    /// # Errors
    ///
    /// Rules 1 to 4 of the module docs; unreachable for a `Config` that came
    /// from a file, since `validate_all` ran the same call.
    pub fn auth_methods(&self) -> Result<AuthMethods, ConfigError> {
        self.auth.resolve()
    }
}

/// Rules 5 to 8 of the module docs, over the whole document.
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

    let disabled =
        |method: AuthMethod, key: String| ConfigError::AuthMethodDisabled { method, key };
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
