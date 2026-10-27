//! The management surface's two registrations (VP-6a): the one tenant the
//! console reads ([`Dashboard`]) and the services allowed to act on it
//! ([`ServiceClient`]).
//!
//! Neither is a credential for a person. A person who reads `/dash/v1` holds a
//! console key in the database (`console_keys`, paired through `/svc/v1` by a
//! service holding [`ServiceCapability::ConsolePairing`], or by `vpay-server
//! staff pair-key`); what this file decides is which tenant that key reads and
//! which services may pair one.
//!
//! # Where service keys live, and why not with the merchants
//!
//! `service_clients[].cose_keys` is YAML, in the shape of
//! `merchant_clients[].cose_keys` (VP-2) and read by the same code
//! ([`crate::read_cose_keys`]), but a **separate list**: a merchant's key must
//! never reach `/svc/v1`, and the cheapest way to make that structural is for
//! the two registries to be two lists that two resolvers read, with boot
//! refusing one key in both (`DuplicateCoseKey` spans them). A capability is a
//! field of this file and nothing else: no HTTP route, no database row and no
//! environment variable can grant one, so granting `console_pairing` is a
//! reviewed change to configuration, the way adding a merchant is (ADR-0003,
//! ADR-0010).
//!
//! The kill switch is the merchants' (`disabled_clients`, keyed by
//! `client_id`): a disabled service resolves to no key.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::oauth::{CoseKey, MerchantClient};
use crate::{ConfigError, read_cose_keys};

/// The `dashboard:` block: the one tenant `/dash/v1` reads
/// (`docs/flows/dashboard-auth.md`).
///
/// **Required, and deliberately singular.** ADR-0008's dashboard observes
/// records; a staff surface that could name any tenant would be an
/// authorisation decision taken per request against a list nothing validates.
/// One deployment, one tenant, fixed in YAML and checked at boot against the
/// registered merchants ([`ConfigError::DashboardUnknownMerchant`]), so a
/// typo is a refused deployment and not a dashboard that renders an empty
/// table and looks like a merchant with no payments.
///
/// A key paired through `/svc/v1` is bound to this tenant and is never an
/// admin (owner ruling R2). An admin (`vpay-server staff add --admin`, then
/// `staff pair-key`) may name another registered merchant with
/// `?merchant_id=` (ADR-0018); that is a property of the staff row, not of
/// this block.
///
/// ```yaml
/// dashboard:
///   merchant_id: acme-cameroon-tenant
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "snake_case")]
pub struct Dashboard {
    /// The tenant every `/dash/v1` query is filtered by.
    #[garde(length(min = 1))]
    pub merchant_id: String,
}

/// What a registered service may do. A closed set: a capability this binary
/// does not know fails to parse, so a typo is a refused deployment and not a
/// service that quietly may not do what its operator wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceCapability {
    /// Pair and revoke console keys (`POST /svc/v1/console_keys`,
    /// `DELETE /svc/v1/console_keys/{thumbprint}`). Vaam's: its admin phone
    /// vouches for a person, and vaam says so to vpay.
    ConsolePairing,
}

impl ServiceCapability {
    /// The spelling the configuration and logs use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConsolePairing => "console_pairing",
        }
    }
}

/// One `service_clients` entry: a registered service, its signing keys, and
/// the capabilities they carry.
///
/// ```yaml
/// service_clients:
///   - client_id: vaam
///     capabilities: [console_pairing]
///     cose_keys:
///       - public_key: 11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo
///         label: vaam-2026-10
/// ```
#[derive(Clone, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "snake_case")]
pub struct ServiceClient {
    /// The name the kill switch (`disabled_clients`) and the logs know it by.
    /// One namespace with the merchants' `client_id`.
    #[garde(length(min = 1))]
    pub client_id: String,
    /// What this service may do. Non-empty.
    #[garde(skip)]
    #[serde(default)]
    pub capabilities: Vec<ServiceCapability>,
    /// The Ed25519 public keys it signs `/svc/v1` requests with, in
    /// [`MerchantClient::cose_keys`]'s shape. Non-empty.
    #[garde(skip)]
    #[serde(default)]
    pub cose_keys: Vec<CoseKey>,
    /// Must never be set; it exists so a file that carries a secret is
    /// refused rather than ignored ([`ConfigError::ClientSecretPresent`]).
    #[garde(skip)]
    #[serde(default)]
    pub client_secret: Option<String>,
}

impl ServiceClient {
    /// Whether this service holds `capability`.
    #[must_use]
    pub fn holds(&self, capability: ServiceCapability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// Public keys print in full (`MerchantClient`'s reason: "which keys did this
/// deployment load?" is the first question when a signed request answers
/// `401`); the secret that must not be there never prints.
impl fmt::Debug for ServiceClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceClient")
            .field("client_id", &self.client_id)
            .field("capabilities", &self.capabilities)
            .field("cose_keys", &self.cose_keys)
            .field(
                "client_secret",
                &self.client_secret.as_deref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

/// `dashboard.merchant_id` names a tenant some merchant registration actually
/// registers.
///
/// A linear scan: the list is the deployment's merchants, this runs once at
/// boot.
///
/// # Errors
///
/// [`ConfigError::DashboardUnknownMerchant`].
pub(crate) fn validate_dashboard(
    dashboard: &Dashboard,
    merchants: &[MerchantClient],
) -> Result<(), ConfigError> {
    if merchants
        .iter()
        .any(|merchant| merchant.merchant_id == dashboard.merchant_id)
    {
        return Ok(());
    }
    Err(ConfigError::DashboardUnknownMerchant {
        merchant_id: dashboard.merchant_id.clone(),
    })
}

/// One service registration's own rules: no secret, at least one capability,
/// at least one readable key, and `console_pairing` only where there is a
/// dashboard to pair into.
///
/// # Errors
///
/// [`ConfigError::ClientSecretPresent`],
/// [`ConfigError::ServiceClientWithoutCapability`],
/// [`ConfigError::ServiceClientWithoutKey`],
/// [`ConfigError::MalformedCoseKey`] and
/// [`ConfigError::ConsolePairingWithoutDashboard`], in that order.
pub(crate) fn validate_service_client(
    service: &ServiceClient,
    dashboard: Option<&Dashboard>,
) -> Result<(), ConfigError> {
    if service.client_secret.is_some() {
        return Err(ConfigError::ClientSecretPresent(service.client_id.clone()));
    }
    if service.capabilities.is_empty() {
        return Err(ConfigError::ServiceClientWithoutCapability {
            client_id: service.client_id.clone(),
        });
    }
    if service.cose_keys.is_empty() {
        return Err(ConfigError::ServiceClientWithoutKey {
            client_id: service.client_id.clone(),
        });
    }
    read_cose_keys(&service.client_id, &service.cose_keys)?;
    if service.holds(ServiceCapability::ConsolePairing) && dashboard.is_none() {
        return Err(ConfigError::ConsolePairingWithoutDashboard {
            client_id: service.client_id.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capability_this_binary_does_not_know_does_not_parse() {
        let parsed: Result<ServiceClient, _> = serde_yaml_ng::from_str(
            "client_id: vaam\ncapabilities: [console_pairing, root]\ncose_keys: []\n",
        );
        assert!(parsed.is_err(), "{parsed:?}");
    }

    #[test]
    fn the_capability_spelling_is_the_configuration_spelling() {
        let parsed: ServiceClient =
            serde_yaml_ng::from_str("client_id: vaam\ncapabilities: [console_pairing]\n")
                .expect("parses");
        assert_eq!(parsed.capabilities, vec![ServiceCapability::ConsolePairing]);
        assert_eq!(
            ServiceCapability::ConsolePairing.as_str(),
            "console_pairing"
        );
    }

    #[test]
    fn a_service_clients_debug_output_never_contains_a_secret() {
        let service = ServiceClient {
            client_id: "vaam".to_owned(),
            capabilities: vec![ServiceCapability::ConsolePairing],
            cose_keys: Vec::new(),
            client_secret: Some("this-should-never-be-here".to_owned()),
        };
        let formatted = format!("{service:?}");
        assert!(
            !formatted.contains("this-should-never-be-here"),
            "{formatted}"
        );
        assert!(formatted.contains("[redacted]"), "{formatted}");
        assert!(formatted.contains("ConsolePairing"), "{formatted}");
    }
}
