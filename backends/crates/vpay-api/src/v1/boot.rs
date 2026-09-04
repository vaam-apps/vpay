//! The boot steps both binaries run before either serves anything: the
//! adapters this binary links keyed by `providers.code`, the YAML loaded and
//! validated, the join between the two, and the reference-table reconcile.
//!
//! Re-exported as [`crate::boot`], which is the name to call it by; it lives
//! under `v1` for historical reasons only.
//!
//! Why `vpay-api` is the home for it, what stays per-binary, why one
//! implementation rather than two, and why every adapter comes back wrapped in
//! [`vpay_provider::Measured`]:
//! [docs/reference/vpay-api.md § boot](../../../../../docs/reference/vpay-api.md#boot-bootrs).
//! The ordering these steps have to be called in is
//! [docs/reference/vpay-config.md § the boot sequence](../../../../../docs/reference/vpay-config.md#the-boot-sequence).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use vpay_config::{Config, ConfigError};
use vpay_core::ProviderFlow;
use vpay_core::error::{Category, Classify};
use vpay_db::{CurrencySeed, DbError, ProviderSeed, Repositories};
use vpay_provider::{Measured, ProviderAdapter};

/// A binary's linked adapters, keyed by `providers.code`.
///
/// The map — rather than the `Vec` a binary's own `adapters()` returns — is
/// what both the boot-time join below and `RouterDeps::adapters` need: a
/// `confirm` resolves a rail by the `payment_method_data[type]` a caller
/// sent, and boot resolves one by the `providers[].code` the YAML names.
///
/// Keyed by [`ProviderAdapter::code`] rather than by a string the caller
/// supplies, so the key can never disagree with the adapter's own idea of
/// what it is. Two adapters claiming one code would silently collapse to
/// one entry — that is a linking mistake inside a single binary, caught by
/// `vpay-server`'s `both_mvp_rails_are_linked` and by the panic-free
/// assertion in this module's tests, not something a running deployment can
/// cause.
///
/// # Every adapter comes back wrapped in [`Measured`]
///
/// This is where `vpay_provider_requests_total` and
/// `vpay_provider_request_duration_seconds` get their one seam. The wrap
/// happens here rather than in either binary's `adapters()` list because
/// that list is deliberately duplicated per binary (Step 2's D6) and a
/// metric mounted in a duplicated list is one the copies eventually
/// disagree about — the same argument this module's header makes about the
/// derivation below. Everything that resolves a rail goes through this
/// function: both `main`s, and the integration suite's own harness.
///
/// [`Measured`] delegates every method to the adapter it wraps and returns
/// it as a plain `Box<dyn ProviderAdapter>`, so no caller can tell — or
/// branch on — whether it is holding a wrapper. It is not a substitute for
/// a rail and adds no code path that exists only outside production
/// (ADR-0006); it is the shipping process measuring itself.
///
/// The conformance suite constructs adapters directly and is therefore
/// *not* measured, which is correct: it exercises one adapter against a
/// stub, and its counts would say nothing about a deployment.
#[must_use]
pub fn adapters_by_code(
    adapters: Vec<Box<dyn ProviderAdapter>>,
) -> BTreeMap<String, Box<dyn ProviderAdapter>> {
    adapters
        .into_iter()
        .map(|adapter| (adapter.code().to_owned(), Measured::wrap(adapter)))
        .collect()
}

/// Boot step 4's inputs: what this deployment's YAML says the reference
/// tables should hold, once joined against the adapters the calling binary
/// links.
///
/// Call it **before** the database is touched, as both binaries do: a
/// `providers[]` entry naming a rail with no linked adapter is then exit
/// `78` in milliseconds rather than after a connection and a migration run —
/// the same "cheapest hard failure first" ordering the config load itself
/// follows. `vpay-server/tests/cli.rs`
/// (`a_provider_code_with_no_linked_adapter_is_exit_78`) needs no container
/// precisely because of that placement, so moving the call site below
/// `vpay_db::connect` would break it.
///
/// # Errors
///
/// [`ConfigError::ProviderWithoutAdapter`] for a configured rail the calling
/// binary links no code for; [`ConfigError::Validation`] for a currency
/// exponent that does not fit the column (unreachable while
/// `Config::validate_all` bounds it, and therefore a signal that the bound
/// is gone).
pub fn boot_seeds(
    config: &Config,
    adapters: &BTreeMap<String, Box<dyn ProviderAdapter>>,
) -> Result<(Vec<CurrencySeed>, Vec<ProviderSeed>), ConfigError> {
    let currencies = config
        .currencies
        .iter()
        .map(|entry| {
            let exponent = i32::try_from(entry.exponent).map_err(|_error| {
                ConfigError::Validation(format!(
                    "currency {} declares exponent {}, which does not fit the \
                     `currencies.exponent` column; Config::validate_all bounds it to 0..=4, so \
                     reaching this means that bound is gone",
                    entry.code, entry.exponent
                ))
            })?;
            Ok(CurrencySeed {
                code: entry.code.to_ascii_uppercase(),
                exponent,
            })
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;

    let providers = config
        .providers
        .iter()
        .map(|provider| {
            // The join, and the only place a configured rail meets the code
            // that would have to serve it. Fatal rather than skipped — see
            // `ConfigError::ProviderWithoutAdapter`.
            let adapter = adapters.get(&provider.code).ok_or_else(|| {
                ConfigError::ProviderWithoutAdapter {
                    code: provider.code.clone(),
                    linked: adapters.keys().cloned().collect::<Vec<_>>().join(", "),
                }
            })?;
            let capabilities = adapter.capabilities();
            Ok(ProviderSeed {
                code: provider.code.clone(),
                display_name: display_name_for(&provider.code),
                flow: flow_label(capabilities.flow).to_owned(),
                supports_refunds: capabilities.supports_refunds,
                supports_partial_refunds: capabilities.supports_partial_refunds,
                delivers_callbacks: capabilities.delivers_callbacks,
                requires_ip_allowlist: capabilities.requires_ip_allowlist,
                // The one field the *deployment* owns. Every other field
                // above comes from the adapter, because a capability is a
                // property of the rail's code and not of a config file
                // (ADR-0002); whether a rail is offered right now is the
                // opposite (`ProviderHost::enabled`).
                enabled: provider.enabled,
            })
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;

    Ok((currencies, providers))
}

/// The `provider_flow` enum label for a flow shape.
///
/// Spelled here rather than through `serde` because the column is read and
/// written as a `String` (Step 2's D4) and [`ProviderFlow`] has no
/// `as_wire_str` of its own the way `IntentStatus` and `ChargeState` do. A
/// label that disagreed with migration 0002's `CREATE TYPE provider_flow AS
/// ENUM ('push', 'redirect')` is a `DbError::Query` at boot, not a silently
/// stored typo — which is why this is a `match` and not a `to_lowercase` of
/// the variant name.
///
/// Private: nothing outside [`boot_seeds`] should be turning a flow into a
/// column value, and a `pub` version would be a second vocabulary competing
/// with the enum itself.
const fn flow_label(flow: ProviderFlow) -> &'static str {
    match flow {
        ProviderFlow::Push => "push",
        ProviderFlow::Redirect => "redirect",
    }
}

/// `providers.display_name`, derived from the rail code.
///
/// **Derived, because nothing else in the tree has one.** [`ProviderAdapter`]
/// exposes `code()` and `capabilities()` and no display name, and
/// `ProviderHost` deliberately carries no capability or presentation fields
/// (`host.label` names the *host* — "mtn-sandbox-wiremock" — not the rail,
/// and putting that in front of an operator would be worse than this). So
/// `mtn_momo` becomes `Mtn Momo`: mechanical, obviously derived, and wrong
/// about nothing. When the port grows a real `display_name()`, this should
/// read it instead of transforming a code.
fn display_name_for(code: &str) -> String {
    code.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A boot step that failed before the process could serve anything.
///
/// It exists so the two database steps keep the *distinct* sentences they had
/// when each carried its own `.context(..)` in a binary: `{error:#}` renders
/// `connecting to Postgres: <what Postgres said>`, which is what an operator
/// reads on a failed start. A single combined message would have made "the URL
/// is wrong" and "a migration will not apply" the same line.
///
/// [`Classify`] is delegated to the [`DbError`] underneath, never re-decided
/// here (ADR-0011): a `DbError` that knows it is a deploy problem —
/// `SigningKeyRetired`, say — must exit `78` through this wrapper exactly as it
/// would without one.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    /// The pool could not be opened. Almost always the deployment's
    /// `DATABASE_URL` or a Postgres that is not up yet.
    #[error("connecting to Postgres")]
    Connect(#[source] DbError),
    /// The pool opened and a migration did not apply.
    #[error("running database migrations")]
    Migrate(#[source] DbError),
}

impl Classify for BootError {
    fn category(&self) -> Category {
        match self {
            Self::Connect(error) | Self::Migrate(error) => error.category(),
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Connect(error) | Self::Migrate(error) => error.code(),
        }
    }

    fn retry(&self) -> vpay_core::error::Retry {
        match self {
            Self::Connect(error) | Self::Migrate(error) => error.retry(),
        }
    }

    fn severity(&self) -> vpay_core::error::Severity {
        match self {
            Self::Connect(error) | Self::Migrate(error) => error.severity(),
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::Connect(error) | Self::Migrate(error) => error.public_message(),
        }
    }
}

/// Boot steps 1-3: load `application.yml`, overlay `application-{profile}.yml`,
/// resolve `${ENV}`, validate (ADR-0003).
///
/// Call it **before** the database is touched. Validating a local YAML file
/// needs no network round trip, so a broken config fails in milliseconds
/// instead of after paying for a Postgres connection and a migration run the
/// process is about to throw away.
///
/// The two log lines are here rather than at the call sites so both binaries
/// emit the same fields in the same order — an operator reading a failed boot
/// compares them. Neither line carries the `Config` itself: they are discrete,
/// non-secret fields, so redaction is a property of *what is logged* rather
/// than of `Config`'s `Debug`.
///
/// # Errors
///
/// [`ConfigError`] — a missing or unreadable file, an unresolved `${ENV}`
/// placeholder, or anything `Config::validate_all` refuses. Every variant
/// classifies as [`Category::Configuration`], so a binary exits `78`.
pub fn load_config(path: Option<&Path>, profile: &str) -> Result<Config, ConfigError> {
    // `profile` names a YAML config file, never a code path — ADR-0003.
    tracing::info!(%profile, "deployment profile (selects a config file only)");
    let config = Config::load(path, profile)?;
    tracing::info!(
        deployment = %config.deployment.name,
        livemode = config.deployment.livemode,
        providers = config.providers.len(),
        merchant_clients = config.merchant_clients.len(),
        dashboard_client_configured = config.dashboard_client.is_some(),
        "configuration loaded and validated"
    );
    Ok(config)
}

/// Opens the pool and applies every migration, in that order.
///
/// Both binaries do this before they bind anything: a process that binds a port
/// before proving the database is reachable and up to date would start
/// accepting connections it cannot serve correctly, and `/healthz` runs a real
/// `SELECT 1`.
///
/// # Errors
///
/// [`BootError::Connect`] or [`BootError::Migrate`], each carrying the
/// [`DbError`] underneath so a binary's `exit_code_for` classifies the leaf
/// rather than the step.
pub async fn open_migrated_database(
    database_url: &str,
) -> Result<Arc<dyn Repositories>, BootError> {
    let repositories = vpay_db::connect(database_url)
        .await
        .map_err(BootError::Connect)?;
    repositories
        .run_migrations()
        .await
        .map_err(BootError::Migrate)?;
    tracing::info!("database connected and migrations applied");
    Ok(repositories)
}

/// Boot step 4 (`docs/flows/configuration.md`): make `currencies` and
/// `providers` match this deployment's configuration, in one transaction.
///
/// After the migrations, because the tables have to exist. Fatal on failure at
/// every call site: a `providers` table that still enables a rail an operator
/// removed is a deployment that would keep taking charges on it.
///
/// Safe to call from **both** binaries, and not because it is idempotent —
/// idempotence covers repeating a reconcile, and a rollout *overlaps* two of
/// them. They cannot interleave (the transaction opens by taking
/// `vpay_db::lock_keys::CONFIG_RECONCILE`) and they cannot disagree about what
/// to write (the seeds come from [`boot_seeds`], one derivation). Two processes
/// on *different* YAML each write their own view, last commit winning; the lock
/// makes that one of the two inputs rather than a mixture.
///
/// # Errors
///
/// [`DbError`] if the transaction cannot be taken or the writes fail.
pub async fn reconcile_reference_tables(
    repositories: &dyn Repositories,
    currencies: &[CurrencySeed],
    providers: &[ProviderSeed],
) -> Result<(), DbError> {
    repositories.reconcile(currencies, providers).await?;
    tracing::info!(
        currencies = currencies.len(),
        providers = providers.len(),
        enabled_providers = providers.iter().filter(|seed| seed.enabled).count(),
        "reference tables reconciled from configuration"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use vpay_config::{CurrencyEntry, Deployment, HostEntry, ProviderHost};
    use vpay_core::Money;
    use vpay_provider::{
        CallbackRef, Capabilities, ChargeRef, ChargeStatus, ProviderConfig, ProviderError,
        Submitted,
    };

    use super::*;

    /// A rail with a code and a flow, and nothing else.
    ///
    /// **Not a test double of an adapter.** It implements the real port so
    /// [`boot_seeds`] can be exercised on the two flow shapes without
    /// linking an adapter crate into `vpay-api` (which would invert the
    /// dependency ADR-0002 draws), and every method that would talk to a
    /// rail answers `Unsupported` rather than a plausible success — this
    /// type can only ever be handed to the pure join above. It is
    /// `#[cfg(test)]`, so no shipping binary can reach it.
    ///
    /// `Unsupported` rather than a `NotImplemented` token: the latter is a
    /// *declaration* that a real code path is unbuilt, tracked by
    /// `cargo xtask verify-status` against `docs/status.md`, and a fixture
    /// in a unit test has no business adding a row to that page.
    #[derive(Debug)]
    struct TestRail {
        code: &'static str,
        flow: ProviderFlow,
    }

    #[async_trait::async_trait]
    impl ProviderAdapter for TestRail {
        fn code(&self) -> &'static str {
            self.code
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                flow: self.flow,
                supports_refunds: true,
                supports_partial_refunds: true,
                delivers_callbacks: true,
                requires_ip_allowlist: false,
            }
        }

        async fn submit(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<Submitted, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        async fn query_status(
            &self,
            _charge: &ChargeRef,
            _config: &ProviderConfig,
        ) -> Result<ChargeStatus, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        fn parse_callback(&self, _body: &[u8]) -> Result<CallbackRef, ProviderError> {
            Err(ProviderError::Unsupported)
        }

        async fn refund(
            &self,
            _charge: &ChargeRef,
            _amount: Money,
            _config: &ProviderConfig,
        ) -> Result<Submitted, ProviderError> {
            Err(ProviderError::Unsupported)
        }
    }

    fn config_with(codes: &[&str]) -> Config {
        Config {
            deployment: Deployment {
                name: "boot-tests".to_owned(),
                livemode: false,
                public_base_url: "http://localhost:8080".to_owned(),
            },
            providers: codes
                .iter()
                .map(|code| ProviderHost {
                    code: (*code).to_owned(),
                    enabled: true,
                    host: HostEntry {
                        url: "https://rail.example".to_owned(),
                        label: "rail".to_owned(),
                    },
                    settings: BTreeMap::new(),
                    callback_url: None,
                    currency: "XAF".to_owned(),
                    credentials: BTreeMap::new(),
                })
                .collect(),
            currencies: vec![CurrencyEntry {
                code: "xaf".to_owned(),
                exponent: 0,
            }],
            merchant_clients: Vec::new(),
            webhooks: vpay_config::WebhookPolicy::default(),
            dashboard_client: None,
        }
    }

    fn two_rails() -> BTreeMap<String, Box<dyn ProviderAdapter>> {
        adapters_by_code(vec![
            Box::new(TestRail {
                code: "mtn_momo",
                flow: ProviderFlow::Push,
            }),
            Box::new(TestRail {
                code: "orange_money",
                flow: ProviderFlow::Redirect,
            }),
        ])
    }

    /// The join both binaries run, end to end: every configured rail is
    /// seeded with the *adapter's* capabilities and the *config's* enabled
    /// flag, and the currency code is uppercased for migration 0001's
    /// `code_is_iso4217_shape` CHECK (the YAML above spells it lowercase on
    /// purpose).
    #[test]
    fn boot_seeds_joins_the_yaml_against_the_linked_adapters() {
        let adapters = two_rails();
        let (currencies, providers) =
            boot_seeds(&config_with(&["mtn_momo", "orange_money"]), &adapters)
                .expect("both rails are linked");

        assert_eq!(
            currencies
                .iter()
                .map(|seed| (seed.code.as_str(), seed.exponent))
                .collect::<Vec<_>>(),
            vec![("XAF", 0)]
        );
        assert_eq!(
            providers
                .iter()
                .map(|seed| (
                    seed.code.as_str(),
                    seed.flow.as_str(),
                    seed.display_name.as_str(),
                    seed.enabled
                ))
                .collect::<Vec<_>>(),
            vec![
                ("mtn_momo", "push", "Mtn Momo", true),
                ("orange_money", "redirect", "Orange Money", true),
            ],
            "the flow and the capabilities must come from the adapter, the enabled flag from \
             the YAML"
        );
    }

    /// A rail switched off in the YAML is still seeded — with
    /// `enabled = false`, so its configuration survives while new charges
    /// stop. Dropping it from the seed instead would make boot step 4's
    /// disable pass indistinguishable from "this rail was deleted".
    #[test]
    fn a_disabled_rail_is_seeded_disabled_rather_than_omitted() {
        let adapters = two_rails();
        let mut config = config_with(&["mtn_momo", "orange_money"]);
        // `.get_mut` rather than `[1]`: the workspace denies
        // `clippy::indexing_slicing` in tests too.
        config
            .providers
            .get_mut(1)
            .expect("the fixture configures two rails")
            .enabled = false;

        let (_currencies, providers) = boot_seeds(&config, &adapters).expect("both are linked");
        assert_eq!(
            providers
                .iter()
                .map(|seed| (seed.code.as_str(), seed.enabled))
                .collect::<Vec<_>>(),
            vec![("mtn_momo", true), ("orange_money", false)],
            "a disabled rail is seeded disabled, not dropped — the YAML owns this field alone"
        );
    }

    /// The failure both binaries turn into exit 78, with a message naming
    /// the rail *and* what is linked — the two assertions
    /// `vpay-server/tests/cli.rs` makes on the real process's stderr.
    #[test]
    fn a_configured_rail_with_no_linked_adapter_is_a_named_config_error() {
        let adapters = two_rails();
        let error = boot_seeds(&config_with(&["a_rail_that_does_not_exist"]), &adapters)
            .expect_err("an unlinked rail must be refused");

        match &error {
            ConfigError::ProviderWithoutAdapter { code, linked } => {
                assert_eq!(code, "a_rail_that_does_not_exist");
                assert!(
                    linked.contains("mtn_momo") && linked.contains("orange_money"),
                    "the message must list what IS linked so it is actionable: {linked}"
                );
            }
            other => panic!("expected ProviderWithoutAdapter, got {other:?}"),
        }
    }

    /// `display_name_for` is mechanical, and the inputs that would otherwise
    /// panic or produce a ragged name are the ones worth pinning: an empty
    /// segment from a doubled underscore, and a non-ASCII first character
    /// (`char::to_uppercase` can yield more than one `char`).
    #[test]
    fn a_display_name_is_derived_from_the_code_without_panicking() {
        assert_eq!(display_name_for("mtn_momo"), "Mtn Momo");
        assert_eq!(display_name_for("orange_money"), "Orange Money");
        assert_eq!(display_name_for("a__b"), "A B");
        assert_eq!(display_name_for(""), "");
        assert_eq!(display_name_for("etoile_pay"), "Etoile Pay");
    }

    /// The labels migration 0002's `provider_flow` enum accepts. A typo here
    /// is a `DbError::Query` at boot in every deployment at once.
    #[test]
    fn the_flow_labels_are_the_enum_members_migration_0002_declares() {
        assert_eq!(flow_label(ProviderFlow::Push), "push");
        assert_eq!(flow_label(ProviderFlow::Redirect), "redirect");
    }

    /// The map is keyed by the adapter's own `code()`, which is what lets a
    /// `confirm` resolve `payment_method_data[type]` and boot resolve
    /// `providers[].code` through one structure.
    #[test]
    fn the_adapter_map_is_keyed_by_the_adapters_own_code() {
        let adapters = two_rails();
        assert_eq!(
            adapters.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["mtn_momo", "orange_money"]
        );
        for (key, adapter) in &adapters {
            assert_eq!(key, adapter.code());
        }
    }

    /// **The wiring test for `vpay_provider_requests_total`.** Every adapter
    /// this function returns is measured, whichever binary asked for it.
    ///
    /// `vpay_provider::measured`'s own tests prove the decorator records the
    /// right labels; this proves that a rail resolved the way both `main`s
    /// and the integration harness resolve one *is* decorated. Delete the
    /// `Measured::wrap` above and the scrape below is empty — which is
    /// exactly what a deployment with no rail metrics looks like, and
    /// nothing else in the tree would notice.
    #[test]
    fn every_adapter_this_function_returns_is_measured() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            let adapters = two_rails();
            let adapter = adapters.get("mtn_momo").expect("the rail is in the map");
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a current-thread runtime builds");
            let answered = runtime.block_on(adapter.query_status(&charge(), &config()));
            assert!(
                answered.is_err(),
                "TestRail refuses every call; the decorator forwards that unchanged"
            );
        });
        let scrape = handle.render();

        assert!(
            scrape.contains(
                r#"vpay_provider_requests_total{provider="mtn_momo",operation="query_status",error_kind="operation_unsupported_by_rail"} 1"#
            ),
            "adapters_by_code must return measured adapters: {scrape}"
        );
    }

    /// The narrowest `ChargeRef` the port accepts. Only its existence
    /// matters here — `TestRail` reads none of it.
    fn charge() -> ChargeRef {
        ChargeRef {
            reference_id: uuid::Uuid::nil(),
            amount: Money::new(5_000, vpay_core::Currency::Xaf).expect("5000 is non-negative"),
            payer_ref: None,
            ref_extra: std::collections::BTreeMap::new(),
        }
    }

    /// Likewise the narrowest `ProviderConfig`.
    fn config() -> ProviderConfig {
        ProviderConfig {
            base_url: "https://rail.example".to_owned(),
            callback_url: "https://vpay.example/provider/mtn_momo/callback".to_owned(),
            currency: vpay_core::Currency::Xaf,
            settings: BTreeMap::new(),
            credentials: BTreeMap::new(),
            connect_timeout: vpay_provider::DEFAULT_CONNECT_TIMEOUT,
            request_timeout: vpay_provider::DEFAULT_REQUEST_TIMEOUT,
        }
    }
}
