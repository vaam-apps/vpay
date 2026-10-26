//! The merchants' Ed25519 keys, and the resolver the signed transport
//! verifies against (VP-2).
//!
//! `merchant_clients[].cose_keys` is YAML (ADR-0003, ADR-0010), read once at
//! boot into a [`MerchantKeyRegistry`]; [`MerchantKeyResolver`] puts the
//! kill switch in front of it. Two questions get asked, at two moments of a
//! signed request, and this module answers both:
//!
//! 1. *Which keys might have produced this message?* — cratestack's
//!    [`CoseVerifierResolver::resolve`], asked with the `kid` in the COSE
//!    header before any signature is checked.
//! 2. *Whose is the key that verified?* —
//!    [`MerchantKeyResolver::merchant_for_thumbprint`], asked with the
//!    thumbprint the opened envelope records, to learn the merchant and its
//!    scopes.
//!
//! # YAML adds, the database only subtracts
//!
//! Identity is YAML's and revocation is `disabled_clients`', exactly as for the
//! OP's client store (`op::clients`, ADR-0010): a key that is not in the file
//! is not a key, and nothing in the database can put one there. A client
//! with a `disabled_clients` row resolves to **nothing**, for both questions,
//! so a disabled merchant's signature is a message from nobody — the same
//! `401` as an unknown key, and no oracle for whether a merchant exists. The
//! lookup runs after the in-memory one, so an unknown `kid` (what a
//! credential-stuffing request looks like) never reaches Postgres.
//!
//! A kill-switch lookup that *fails* is [`CratestackError::Internal`], which
//! the envelope turns into a `500`. The trait's contract spells it out:
//! `Ok(vec![])` means "no such key", and returning it for an outage would
//! make a database failure look like a revocation to the merchant and like a
//! merchant problem to the operator. Fail closed, and say which.
//!
//! # Not mounted yet
//!
//! Nothing in `router()` builds one of these, and `/v1` still authenticates
//! with the bearer token. The `EnvelopeLayer` that will call it is VP-3; the OP
//! and its JWKS are deleted in VP-4. Until then this module is exercised by
//! its own tests alone.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use cratestack_core::CratestackError;
use cratestack_cose::thumbprint::{KID_LEN, kid_from_thumbprint};
use cratestack_cose::{CoseAlg, CoseVerifierResolver, CoseVerifyKey};
use vpay_config::{Config, ConfigError};
use vpay_db::{DbError, DisabledClients};

/// One registered key and the merchant it belongs to.
#[derive(Debug, Clone)]
struct Entry {
    client_id: String,
    merchant_id: String,
    scopes: Vec<String>,
    label: Option<String>,
    key: CoseVerifyKey,
}

/// The merchant a verified key belongs to, as `/v1` needs it.
///
/// A projection, for [`ResourceConfig`](crate::v1::ResourceConfig)'s reason:
/// what a request path may depend on is exactly what is here, and a handler
/// holding the whole registration could read another merchant's keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMerchant {
    /// The credential: what `disabled_clients` is keyed on and what a log line
    /// names.
    pub client_id: String,
    /// The tenant every `/v1` query is filtered by.
    pub merchant_id: String,
    /// The scopes YAML grants this registration (`payments:read`,
    /// `payments:write`; `v1::required_scopes` decides what a method needs).
    pub scopes: Vec<String>,
    /// The operator's name for the key that verified, if it has one.
    pub label: Option<String>,
    /// The key's RFC 9679 thumbprint — what was looked up.
    pub thumbprint: [u8; 32],
}

/// Every merchant's Ed25519 keys, indexed by thumbprint and by `kid`.
///
/// Immutable for the life of the process (ADR-0003: configuration loads once,
/// with no hot reload), so a plain map behind an `Arc` and no lock.
#[derive(Debug, Clone, Default)]
pub struct MerchantKeyRegistry {
    by_thumbprint: HashMap<[u8; 32], Entry>,
    /// A `kid` is only the first eight bytes of a thumbprint, so several keys
    /// can share one (about 2^32 keys for a birthday collision, but the trait
    /// requires handling it). Values are thumbprints, in registration order.
    by_kid: HashMap<[u8; KID_LEN], Vec<[u8; 32]>>,
}

impl MerchantKeyRegistry {
    /// Reads every merchant's `cose_keys` out of a loaded [`Config`].
    ///
    /// Parses with the same code `Config::validate_all` ran at boot
    /// (`MerchantClient::cose_verify_keys`), and refuses a thumbprint
    /// registered twice again rather than trusting that it ran: a `Config`
    /// built in code and never validated must not produce a registry in which
    /// one key resolves to whichever merchant came last.
    ///
    /// # Errors
    ///
    /// The [`ConfigError`] `validate_all` would have raised —
    /// [`ConfigError::MalformedCoseKey`] or [`ConfigError::DuplicateCoseKey`].
    /// Unreachable for a `Config` that came from a file.
    pub fn from_config(config: &Config) -> Result<Self, ConfigError> {
        let mut registry = Self::default();
        for merchant in &config.merchant_clients {
            for registered in merchant.cose_verify_keys()? {
                let thumbprint = registered.key.thumbprint();
                let entry = Entry {
                    client_id: merchant.client_id.clone(),
                    merchant_id: merchant.merchant_id.clone(),
                    scopes: merchant.scopes.clone(),
                    label: registered.label,
                    key: registered.key,
                };
                if let Some(first) = registry.by_thumbprint.insert(thumbprint, entry) {
                    return Err(ConfigError::DuplicateCoseKey {
                        thumbprint: thumbprint.iter().map(|b| format!("{b:02x}")).collect(),
                        first: first.client_id,
                        second: merchant.client_id.clone(),
                    });
                }
                registry
                    .by_kid
                    .entry(kid_from_thumbprint(&thumbprint))
                    .or_default()
                    .push(thumbprint);
            }
        }
        Ok(registry)
    }

    /// How many keys are registered, across all merchants.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_thumbprint.len()
    }

    /// Whether no merchant has registered a key — the state of every
    /// deployment until its operators add `cose_keys`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_thumbprint.is_empty()
    }

    /// The entries whose `kid` is `kid`, in registration order. Empty for a
    /// `kid` of the wrong length, which no key has.
    fn candidates(&self, kid: &[u8]) -> impl Iterator<Item = &Entry> {
        <[u8; KID_LEN]>::try_from(kid)
            .ok()
            .and_then(|kid| self.by_kid.get(&kid))
            .into_iter()
            .flatten()
            .filter_map(|thumbprint| self.by_thumbprint.get(thumbprint))
    }
}

/// Resolves a signature's key and its merchant, with the kill switch in front.
///
/// Cheap to clone: two `Arc`s.
#[derive(Clone)]
pub struct MerchantKeyResolver {
    registry: Arc<MerchantKeyRegistry>,
    disabled: Arc<dyn DisabledClients>,
}

impl std::fmt::Debug for MerchantKeyResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MerchantKeyResolver")
            .field("keys", &self.registry.len())
            .finish_non_exhaustive()
    }
}

impl MerchantKeyResolver {
    /// `disabled` is the kill-switch query layer; the server hands it the same
    /// repositories the OP's client store reads (`Arc<dyn Repositories>`
    /// coerces: `DisabledClients` is one of its supertraits).
    #[must_use]
    pub fn new(registry: Arc<MerchantKeyRegistry>, disabled: Arc<dyn DisabledClients>) -> Self {
        Self { registry, disabled }
    }

    /// The merchant that registered the key with this RFC 9679 thumbprint, or
    /// `None` if no merchant did **or** the merchant is disabled.
    ///
    /// # Errors
    ///
    /// [`DbError`] if the `disabled_clients` lookup fails, and only then: an
    /// unknown thumbprint is answered from memory. A caller must refuse the
    /// request on `Err`; admitting it would make a database outage a
    /// revocation bypass.
    pub async fn merchant_for_thumbprint(
        &self,
        thumbprint: &[u8; 32],
    ) -> Result<Option<ResolvedMerchant>, DbError> {
        let Some(entry) = self.registry.by_thumbprint.get(thumbprint) else {
            return Ok(None);
        };
        if self.is_disabled(&entry.client_id).await? {
            return Ok(None);
        }
        Ok(Some(ResolvedMerchant {
            client_id: entry.client_id.clone(),
            merchant_id: entry.merchant_id.clone(),
            scopes: entry.scopes.clone(),
            label: entry.label.clone(),
            thumbprint: *thumbprint,
        }))
    }

    async fn is_disabled(&self, client_id: &str) -> Result<bool, DbError> {
        let disabled = self.disabled.is_client_disabled(client_id).await?;
        if disabled {
            tracing::warn!(
                client_id,
                "refusing a configured merchant key: the client is disabled via the \
                 disabled_clients kill switch"
            );
        }
        Ok(disabled)
    }
}

#[async_trait]
impl CoseVerifierResolver for MerchantKeyResolver {
    /// Every key of an enabled merchant whose `kid` is `kid`.
    ///
    /// `alg` filters (`CoseVerifyKey::supports`), so asking for anything but
    /// Ed25519 — ESP256, or either HMAC, which a shared secret would make —
    /// finds nothing: the registry holds no key of those types and has no way
    /// to be given one.
    ///
    /// An unknown `kid` is `Ok(vec![])`, never `Err` (the trait's contract: an
    /// `Err` is a `500`, and answering "no such key" with one would be the
    /// oracle ADR 0006 §10 forbids).
    async fn resolve(
        &self,
        kid: &[u8],
        alg: CoseAlg,
    ) -> Result<Vec<CoseVerifyKey>, CratestackError> {
        // A `BTreeMap` so one client is asked about once however many of its
        // keys share the `kid`, in a stable order.
        let mut verdicts: BTreeMap<&str, bool> = BTreeMap::new();
        let mut keys = Vec::new();
        for entry in self.registry.candidates(kid) {
            if !entry.key.supports(alg) {
                continue;
            }
            let disabled = match verdicts.get(entry.client_id.as_str()) {
                Some(disabled) => *disabled,
                None => {
                    let disabled = self.is_disabled(&entry.client_id).await.map_err(|error| {
                        tracing::error!(
                            %error,
                            client_id = %entry.client_id,
                            "disabled_clients lookup failed; refusing key resolution rather than \
                             assuming the client is enabled"
                        );
                        CratestackError::Internal("disabled_clients lookup failed".to_owned())
                    })?;
                    verdicts.insert(entry.client_id.as_str(), disabled);
                    disabled
                }
            };
            if !disabled {
                keys.push(entry.key.clone());
            }
        }
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use cratestack_cose::Ed25519Signer;
    use vpay_config::CoseKey;
    use vpay_db::Repositories;

    use super::*;
    use crate::test_fixtures::{config_with, merchant};

    /// A kill switch held in memory, counting how often it is asked.
    ///
    /// A fake of the *port* (`DisabledClients`), in a `#[cfg(test)]` module of
    /// the crate that owns the consumer: no shipping graph reaches it
    /// (`verify-no-mocks`), and the real implementation's own behaviour is
    /// proven against Postgres in `vpay-db`'s suite.
    #[derive(Default)]
    struct KillSwitch {
        disabled: Mutex<HashSet<String>>,
        failing: bool,
        asked: AtomicUsize,
    }

    impl KillSwitch {
        fn disabling(client_ids: &[&str]) -> Self {
            Self {
                disabled: Mutex::new(client_ids.iter().map(|id| (*id).to_owned()).collect()),
                ..Self::default()
            }
        }

        fn failing() -> Self {
            Self {
                failing: true,
                ..Self::default()
            }
        }

        fn asked(&self) -> usize {
            self.asked.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DisabledClients for KillSwitch {
        async fn is_client_disabled(&self, client_id: &str) -> Result<bool, DbError> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            if self.failing {
                return Err(DbError::Query(sqlx::Error::PoolClosed));
            }
            Ok(self
                .disabled
                .lock()
                .is_ok_and(|disabled| disabled.contains(client_id)))
        }

        async fn disable_client(&self, client_id: &str, _: Option<&str>) -> Result<(), DbError> {
            if let Ok(mut disabled) = self.disabled.lock() {
                disabled.insert(client_id.to_owned());
            }
            Ok(())
        }

        async fn enable_client(&self, client_id: &str) -> Result<(), DbError> {
            if let Ok(mut disabled) = self.disabled.lock() {
                disabled.remove(client_id);
            }
            Ok(())
        }
    }

    /// A real Ed25519 key per seed: the public half is what YAML holds, and
    /// the verification key is what the resolver must hand back.
    fn key(seed: u8) -> (CoseKey, CoseVerifyKey) {
        let verify = Ed25519Signer::from_seed(&[seed; 32]).verify_key();
        let bytes = verify.ed25519_bytes().expect("an Ed25519 key has 32 bytes");
        let entry = CoseKey {
            public_key: URL_SAFE_NO_PAD.encode(bytes),
            label: Some(format!("key-{seed}")),
        };
        (entry, verify)
    }

    fn registered(
        client_id: &str,
        scopes: &[&str],
        keys: &[&CoseKey],
    ) -> vpay_config::MerchantClient {
        let mut client = merchant(client_id, scopes);
        client.cose_keys = keys.iter().map(|entry| (*entry).clone()).collect();
        client
    }

    struct World {
        acme_key: CoseVerifyKey,
        acme_second_key: CoseVerifyKey,
        beta_key: CoseVerifyKey,
        registry: Arc<MerchantKeyRegistry>,
    }

    /// Two merchants: `acme` with two keys (a rotation in flight) and read +
    /// write scopes, `beta` with one key and read only.
    fn world() -> World {
        let (acme_entry, acme_key) = key(1);
        let (acme_second_entry, acme_second_key) = key(2);
        let (beta_entry, beta_key) = key(3);
        let config = config_with(
            "https://api.vpay.test",
            vec![
                registered(
                    "acme",
                    &["payments:write", "payments:read"],
                    &[&acme_entry, &acme_second_entry],
                ),
                registered("beta", &["payments:read"], &[&beta_entry]),
            ],
        );
        World {
            acme_key,
            acme_second_key,
            beta_key,
            registry: Arc::new(MerchantKeyRegistry::from_config(&config).expect("valid keys")),
        }
    }

    fn resolver(world: &World, switch: &Arc<KillSwitch>) -> MerchantKeyResolver {
        MerchantKeyResolver::new(Arc::clone(&world.registry), switch.clone())
    }

    #[tokio::test]
    async fn a_known_key_resolves_to_its_merchant_and_its_scopes() {
        let world = world();
        let switch = Arc::new(KillSwitch::default());
        let resolver = resolver(&world, &switch);

        let acme = resolver
            .merchant_for_thumbprint(&world.acme_second_key.thumbprint())
            .await
            .expect("the lookup works")
            .expect("a registered key resolves");
        assert_eq!(acme.client_id, "acme");
        // The tenant is not the credential (`MerchantClient::merchant_id`).
        assert_eq!(acme.merchant_id, "acme-tenant");
        assert_eq!(acme.scopes, ["payments:write", "payments:read"]);
        // The label of the key that verified, not of the registration's first.
        assert_eq!(acme.label.as_deref(), Some("key-2"));
        assert_eq!(acme.thumbprint, world.acme_second_key.thumbprint());

        let beta = resolver
            .merchant_for_thumbprint(&world.beta_key.thumbprint())
            .await
            .expect("the lookup works")
            .expect("a registered key resolves");
        assert_eq!(beta.merchant_id, "beta-tenant");
        assert_eq!(beta.scopes, ["payments:read"]);

        // And the verifier-resolver half returns the key itself, by `kid`.
        let keys = resolver
            .resolve(&world.beta_key.kid(), CoseAlg::Ed25519)
            .await
            .expect("resolves");
        assert_eq!(keys, vec![world.beta_key.clone()]);
    }

    #[tokio::test]
    async fn an_unknown_kid_resolves_to_nothing_and_never_asks_the_database() {
        let world = world();
        let switch = Arc::new(KillSwitch::default());
        let resolver = resolver(&world, &switch);

        let (_, stranger) = key(99);
        assert_eq!(
            resolver
                .resolve(&stranger.kid(), CoseAlg::Ed25519)
                .await
                .expect("an unknown kid is Ok(empty), never Err"),
            vec![]
        );
        assert_eq!(
            resolver
                .merchant_for_thumbprint(&stranger.thumbprint())
                .await
                .expect("an unknown thumbprint is Ok(None), never Err"),
            None
        );
        // A kid of the wrong length can be nobody's.
        for kid in [&[][..], &[0; 7], &[0; 9], &[0; 32]] {
            assert_eq!(
                resolver
                    .resolve(kid, CoseAlg::Ed25519)
                    .await
                    .expect("resolves"),
                vec![]
            );
        }
        assert_eq!(
            switch.asked(),
            0,
            "an unregistered key is answered from memory: the kill switch is the \
             database's, and a stranger must not be able to load it"
        );
    }

    #[tokio::test]
    async fn a_disabled_client_resolves_to_nothing_and_the_others_do_not() {
        let world = world();
        let switch = Arc::new(KillSwitch::disabling(&["acme"]));
        let resolver = resolver(&world, &switch);

        for key in [&world.acme_key, &world.acme_second_key] {
            assert_eq!(
                resolver
                    .resolve(&key.kid(), CoseAlg::Ed25519)
                    .await
                    .expect("a disabled client is not an error"),
                vec![],
                "a disabled client's key must resolve to nothing"
            );
            assert_eq!(
                resolver
                    .merchant_for_thumbprint(&key.thumbprint())
                    .await
                    .expect("a disabled client is not an error"),
                None
            );
        }
        // `beta` is untouched by `acme`'s revocation.
        assert_eq!(
            resolver
                .resolve(&world.beta_key.kid(), CoseAlg::Ed25519)
                .await
                .expect("resolves"),
            vec![world.beta_key.clone()]
        );

        // Revocation is live, not read once at boot: the switch is flipped
        // under a running resolver and the answer follows.
        switch.enable_client("acme").await.expect("enables");
        assert_eq!(
            resolver
                .resolve(&world.acme_key.kid(), CoseAlg::Ed25519)
                .await
                .expect("resolves"),
            vec![world.acme_key.clone()]
        );
        switch.disable_client("acme", None).await.expect("disables");
        assert_eq!(
            resolver
                .resolve(&world.acme_key.kid(), CoseAlg::Ed25519)
                .await
                .expect("resolves"),
            vec![]
        );
    }

    /// The other direction of fail-closed: a database that cannot say whether
    /// the client is disabled must not produce a key (and must not produce
    /// "no such key" either).
    #[tokio::test]
    async fn a_failed_kill_switch_lookup_is_an_error_not_a_key_and_not_a_miss() {
        let world = world();
        let switch = Arc::new(KillSwitch::failing());
        let resolver = resolver(&world, &switch);

        let outcome = resolver
            .resolve(&world.acme_key.kid(), CoseAlg::Ed25519)
            .await;
        assert!(
            matches!(outcome, Err(CratestackError::Internal(_))),
            "a failed lookup must be a 500, got {outcome:?}"
        );
        assert!(
            resolver
                .merchant_for_thumbprint(&world.acme_key.thumbprint())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn only_ed25519_is_ever_resolved() {
        let world = world();
        let switch = Arc::new(KillSwitch::default());
        let resolver = resolver(&world, &switch);

        for alg in [CoseAlg::Esp256, CoseAlg::Hmac256_64, CoseAlg::Hmac256_256] {
            assert_eq!(
                resolver
                    .resolve(&world.acme_key.kid(), alg)
                    .await
                    .expect("resolves"),
                vec![],
                "{alg:?}"
            );
        }
    }

    /// Several keys under one `kid` (an 8-byte collision) are all returned and
    /// the client is asked about once. A real collision needs about 2^32 keys,
    /// so this one is forced by filing a second key under the first's bucket.
    #[tokio::test]
    async fn keys_sharing_a_kid_are_all_returned_and_the_client_is_asked_once() {
        let mut world = world();
        let first = world.acme_key.thumbprint();
        let second = world.acme_second_key.thumbprint();
        let mut registry = (*world.registry).clone();
        registry
            .by_kid
            .insert(kid_from_thumbprint(&first), vec![first, second]);
        world.registry = Arc::new(registry);
        let switch = Arc::new(KillSwitch::default());
        let resolver = resolver(&world, &switch);

        let keys = resolver
            .resolve(&world.acme_key.kid(), CoseAlg::Ed25519)
            .await
            .expect("resolves");
        assert_eq!(
            keys,
            vec![world.acme_key.clone(), world.acme_second_key.clone()]
        );
        assert_eq!(switch.asked(), 1);
    }

    #[test]
    fn a_registry_is_empty_until_a_merchant_registers_a_key() {
        let config = config_with("https://api.vpay.test", vec![merchant("acme", &[])]);
        let registry = MerchantKeyRegistry::from_config(&config).expect("no keys is valid");
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert_eq!(world().registry.len(), 3);
    }

    /// `Config::validate_all` has already refused these for a file, but a
    /// `Config` can be built in code, and a registry in which one key names
    /// two merchants must not exist.
    #[test]
    fn building_a_registry_refuses_what_boot_refuses() {
        let (entry, _) = key(1);
        let config = config_with(
            "https://api.vpay.test",
            vec![
                registered("acme", &[], &[&entry]),
                registered("beta", &[], &[&entry]),
            ],
        );
        assert!(matches!(
            MerchantKeyRegistry::from_config(&config),
            Err(ConfigError::DuplicateCoseKey { first, second, .. })
                if first == "acme" && second == "beta"
        ));

        let malformed = CoseKey {
            public_key: "AAAA".to_owned(),
            label: None,
        };
        let config = config_with(
            "https://api.vpay.test",
            vec![registered("acme", &[], &[&malformed])],
        );
        assert!(matches!(
            MerchantKeyRegistry::from_config(&config),
            Err(ConfigError::MalformedCoseKey { index: 0, .. })
        ));
    }

    /// The server hands the resolver the repositories it already holds. If
    /// `DisabledClients` ever stopped being a supertrait of `Repositories`,
    /// this stops compiling rather than the wiring ticket discovering it.
    #[test]
    fn repositories_coerce_to_the_kill_switch_the_resolver_takes() {
        fn takes(repositories: Arc<dyn Repositories>) -> Arc<dyn DisabledClients> {
            repositories
        }
        let _ = takes;
    }
}
