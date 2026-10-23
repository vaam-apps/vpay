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
use vpay_provider::{AcceptedExposure, CallbackUrlMode, Measured, ProviderAdapter};

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
/// Four, and all are [`Category::Configuration`] — exit **78**, "fix the
/// deploy" — at every call site:
///
/// * [`ConfigError::ProviderWithoutAdapter`] for a configured rail the
///   calling binary links no code for;
/// * [`ConfigError::IncoherentCapabilities`] for a configured rail whose
///   adapter declares `supports_partial_refunds` without `supports_refunds`
///   (issue #61, 2026-09-10) — see the check itself for why boot is where
///   that is caught;
/// * [`ConfigError::IncoherentCallbackMode`] for a configured rail whose
///   adapter declares `callback_url_mode: Registered` without
///   `delivers_callbacks` (RFC-0013, 2026-10-22);
/// * [`ConfigError::CallbacksOnPerRequestRail`] for a `callbacks` block on a
///   rail whose adapter is told its callback URL on every request
///   (RFC-0013 § 2, 2026-10-22).
///
/// _(The first line said "Two, and both" until 2026-10-22.)_
///
/// **This paragraph said the first was "now the *only* error this returns"
/// until the second one landed**, which is the drift a `# Errors` section
/// exists to make visible. It also returned [`ConfigError::Validation`] for
/// "a currency exponent that does not fit the column" until migration 0032
/// widened `currencies.exponent` to `BIGINT` and `CurrencySeed::exponent` to
/// `i64`. Every `u32` fits an `i64`, so that arm became unreachable *by
/// type* rather than merely unreached, and a `try_from` kept for it would
/// have been an error path no input could take — see the conversion below.
pub fn boot_seeds(
    config: &Config,
    adapters: &BTreeMap<String, Box<dyn ProviderAdapter>>,
) -> Result<(Vec<CurrencySeed>, Vec<ProviderSeed>), ConfigError> {
    let currencies = config
        .currencies
        .iter()
        .map(|entry| {
            // Infallible, and deliberately spelled as such. `entry.exponent`
            // is a `u32` and the column is `BIGINT` since migration 0032, so
            // there is no value this can refuse; a `try_from` here would be
            // an error branch nothing could reach and no test could cover.
            // The real bound is still two layers up and unchanged —
            // `Config::validate_all` pins each currency's exponent to
            // `vpay_core::Currency::exponent`'s canonical table, and
            // `currencies_exponent_range_check` refuses anything outside
            // `0..=4` at the database.
            Ok(CurrencySeed {
                code: entry.code.to_ascii_uppercase(),
                exponent: i64::from(entry.exponent),
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
            // Issue #61. Refused here, where it is a `ConfigError` and the
            // process exits 78, rather than two steps later where migration
            // 0002's `partial_refunds_imply_refunds` CHECK refuses the row
            // and the process exits 1 — "page someone", about a database
            // that is working perfectly. Both numbers are measured, in
            // `main`'s order and against a real Postgres, by
            // `backends/tests/integration/tests/boot_coherence.rs`.
            //
            // Every *configured* rail, `enabled` or not: `enabled` is a
            // column on the row this seed becomes, not a reason to skip
            // writing it. A rail this binary links but the YAML does not
            // name writes no row at all, and `vpay-server`'s own
            // `no_adapter_advertises_partial_without_full_refunds` covers
            // the whole linked list.
            if !capabilities.is_coherent() {
                return Err(ConfigError::IncoherentCapabilities {
                    code: provider.code.clone(),
                });
            }
            // RFC-0013 § 8. A linking mistake on the line above's terms, and
            // a separate check only because `callback_url_mode` is not
            // persisted and so has no CHECK to mirror.
            if !capabilities.callbacks_are_coherent() {
                return Err(ConfigError::IncoherentCallbackMode {
                    code: provider.code.clone(),
                });
            }
            // RFC-0013 § 2: a setting that does nothing is one somebody
            // believes in. Every configured rail, `enabled` or not, on the
            // line above's reasoning.
            if provider.callbacks.is_some()
                && capabilities.callback_url_mode == CallbackUrlMode::PerRequest
            {
                return Err(ConfigError::CallbacksOnPerRequestRail {
                    code: provider.code.clone(),
                });
            }
            // ADR-0029 D6. **Enabled** rails only, unlike the two above: a
            // rail no method type reaches is unreachable by a payer's
            // choice, which matters only on a rail that can take payments.
            if provider.enabled && !capabilities.method_types_are_coherent() {
                return Err(ConfigError::RailServesNoMethodType {
                    code: provider.code.clone(),
                });
            }
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

    refuse_weights_on_unserved_types(config, adapters)?;

    Ok((currencies, providers))
}

/// `routing.weights[]` entries naming a method type the rail's adapter does
/// not serve (RFC-0008, 2026-10-23 revision). `Config::validate_all` has
/// already refused an unknown rail, an unknown type and a duplicate; this is
/// the half that needs the linked code.
///
/// # Errors
///
/// [`ConfigError::RoutingWeightOnAnUnservedType`].
fn refuse_weights_on_unserved_types(
    config: &Config,
    adapters: &BTreeMap<String, Box<dyn ProviderAdapter>>,
) -> Result<(), ConfigError> {
    for entry in &config.routing.weights {
        let Some(spelling) = entry.method_type.as_deref() else {
            continue;
        };
        let served =
            vpay_core::routing::MethodType::from_wire(spelling).is_some_and(|method_type| {
                adapters.get(&entry.rail).is_some_and(|adapter| {
                    adapter.capabilities().method_types.contains(method_type)
                })
            });
        if !served {
            return Err(ConfigError::RoutingWeightOnAnUnservedType {
                rail: entry.rail.clone(),
                method_type: spelling.to_owned(),
            });
        }
    }
    Ok(())
}

/// Every **configured** rail whose adapter declares an
/// [`AcceptedExposure`], with that declaration — ADR-0031's condition 4.
///
/// Configured, `enabled` or not: a disabled rail's keys are still loaded and
/// still the merchant's, and the operator who wrote the block is the one who
/// should read the warning. A rail this binary links but the YAML does not
/// name has no keys here and is not listed.
///
/// Pure, so the rule can be tested without a subscriber; the logging is
/// [`warn_about_accepted_exposures`]. A configured code with no linked
/// adapter is skipped rather than reported: [`boot_seeds`] refuses it, and
/// both binaries call that first.
#[must_use]
pub fn accepted_exposures(
    config: &Config,
    adapters: &BTreeMap<String, Box<dyn ProviderAdapter>>,
) -> Vec<(String, AcceptedExposure)> {
    config
        .providers
        .iter()
        .filter_map(|provider| {
            let exposure = adapters.get(&provider.code)?.accepted_exposure()?;
            Some((provider.code.clone(), exposure))
        })
        .collect()
}

/// Logs ADR-0031's boot warning, once per configured rail that declares an
/// exposure: what the key the provider calls public lets anyone who holds it
/// do on the merchant's account, and the page that explains it.
///
/// `warn!` in both modes, because the maintainer asked for a warning ("warn
/// the developer about the risks in prod"). In livemode the line says so in
/// capitals and adds that real money is at stake; that is the "louder", and
/// an operator filtering on `livemode=true` finds it. A livemode boot only
/// gets here at all if the rail's block carries its acknowledgement
/// (ADR-0031 condition 4 as amended on 2026-10-20, enforced by
/// `vpay-config`'s rail-key table); the acknowledgement admits the rail and
/// does not silence this.
///
/// **Never prints a key.** The declaration names the credential by its
/// configuration key, and nothing here reads `credentials`.
pub fn warn_about_accepted_exposures(
    config: &Config,
    adapters: &BTreeMap<String, Box<dyn ProviderAdapter>>,
) {
    let livemode = config.deployment.livemode;
    for (code, exposure) in accepted_exposures(config, adapters) {
        let operations = exposure.operations.join("; ");
        if livemode {
            tracing::warn!(
                rail = %code,
                livemode,
                credential = exposure.credential,
                document = exposure.document,
                "LIVEMODE, REAL MONEY: rail {code} is admitted under ADR-0031. Its status read is \
                 authenticated by credentials.{credential}, a key the provider calls PUBLIC. \
                 Anyone who holds that key can: {operations}. vpay keeps it server-side and \
                 sends the private key on every call; it cannot stop a leaked or front-end-\
                 embedded copy being used against this merchant's account. This deployment \
                 acknowledged that exposure in the rail's settings; the acknowledgement admits \
                 the rail, it does not make the key private. Read {document} \
                 (\"Production risks\").",
                credential = exposure.credential,
                document = exposure.document,
            );
        } else {
            tracing::warn!(
                rail = %code,
                livemode,
                credential = exposure.credential,
                document = exposure.document,
                "rail {code} is admitted under ADR-0031: its status read is authenticated by \
                 credentials.{credential}, a key the provider calls public. Anyone who holds \
                 that key can: {operations}. In production this is the merchant's accepted \
                 exposure, and a livemode boot refuses this rail until its settings acknowledge \
                 it; see {document} (\"Production risks\").",
                credential = exposure.credential,
                document = exposure.document,
            );
        }
    }
}

/// The `providers.flow` label for a flow shape.
///
/// Spelled here rather than through `serde` because the column is read and
/// written as a `String` (Step 2's D4) and [`ProviderFlow`] has no
/// `as_wire_str` of its own the way `IntentStatus` and `ChargeState` do. A
/// label nothing downstream accepts is a failed boot, not a silently stored
/// typo — which is why this is a `match` and not a `to_lowercase` of the
/// variant name.
///
/// **What refuses a bad label has moved twice, and the second move changed
/// the advice.** It was migration 0002's `CREATE TYPE provider_flow AS ENUM
/// ('push', 'redirect')` refusing the cast; migration 0032 replaced the enum
/// type with `providers_flow_enum_check` refusing the row (the type had to
/// go because `cratestack`'s generated row decoders read an enum column with
/// `try_get::<String>()`, so a native enum column fails to decode on every
/// read through that layer); and since 2026-09-06 `ConfigReconcile::reconcile`
/// refuses the *seed*, because `CreateProviderInput::flow` is the schema's
/// `ProviderFlow` enum rather than a string. A bad label is therefore
/// `DbError::ProviderFlowUnknown` — `Category::Configuration`, exit **78**
/// ("fix the deploy") — where this comment said `DbError::Query` and meant
/// exit `69` ("wait for Postgres"). The CHECK is still in the database and
/// still refuses a writer that is not `reconcile`.
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
/// Every mode of `vpay-server` — `serve`, `worker` and `staff add` — does this
/// first, and the two that bind a port do it before they bind anything: a
/// process that binds a port before proving the database is reachable and up
/// to date would start accepting connections it cannot serve correctly, and
/// `/healthz` runs a real `SELECT 1`. (This said "both binaries" until
/// 2026-09-23; there has been one binary since 2026-09-07, issue #77.)
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
        RefundDestination, RefundTarget, Refunded, SubmitSteps, Submitted,
    };

    use super::*;

    /// A rail with a code and a declared capability set, and nothing else.
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
    ///
    /// It carries the whole [`Capabilities`] rather than a [`ProviderFlow`]
    /// alone since issue #61 gave [`boot_seeds`] a capability rule to
    /// enforce. The incoherent rail is then this same type with two booleans
    /// changed, which is the difference under test; a second `#[cfg(test)]`
    /// adapter — the shape this started as — left its other four fields free
    /// to drift away from the rail every other case in this module uses.
    #[derive(Debug)]
    struct TestRail {
        code: &'static str,
        capabilities: Capabilities,
        /// What the rail declares under ADR-0031; `None` on every rail but
        /// the one the exposure cases build.
        exposure: Option<AcceptedExposure>,
    }

    /// What a rail that contradicts itself about nothing declares: both
    /// refund shapes, callbacks, no allowlist. Only `flow` varies between
    /// the two rails [`two_rails`] builds, because the flow is the only part
    /// of a capability set the join projects into a column.
    const fn coherent(flow: ProviderFlow) -> Capabilities {
        Capabilities {
            flow,
            supports_refunds: true,
            supports_partial_refunds: true,
            delivers_callbacks: true,
            requires_ip_allowlist: false,
            supports_account_holder_lookup: false,
            // A `#[cfg(test)]` fixture: the value is inert here (this rail
            // refuses every refund) and `Origin` is the shape that needs no
            // payee, so the fixture asks the port for nothing it does not
            // have.
            refund_destination: RefundDestination::Origin,
            callback_url_mode: vpay_provider::CallbackUrlMode::PerRequest,
            submit_steps: vpay_provider::SubmitSteps::One,
            method_types: vpay_core::routing::MethodTypes::of(&vpay_core::routing::MethodType::ALL),
        }
    }

    #[async_trait::async_trait]
    impl ProviderAdapter for TestRail {
        fn code(&self) -> &'static str {
            self.code
        }

        fn capabilities(&self) -> Capabilities {
            self.capabilities
        }

        fn accepted_exposure(&self) -> Option<AcceptedExposure> {
            self.exposure
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
            _destination: Option<&RefundTarget>,
            _config: &ProviderConfig,
        ) -> Result<Refunded, ProviderError> {
            Err(ProviderError::Unsupported)
        }
    }

    fn config_with(codes: &[&str]) -> Config {
        Config {
            deployment: Deployment {
                name: "boot-tests".to_owned(),
                livemode: false,
                public_base_url: "http://localhost:8080".to_owned(),
                surfaces: None,
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
                    display_name: None,
                    callbacks: None,
                })
                .collect(),
            currencies: vec![CurrencyEntry {
                code: "xaf".to_owned(),
                exponent: 0,
            }],
            merchant_clients: Vec::new(),
            webhooks: vpay_config::WebhookPolicy::default(),
            checkout: vpay_config::CheckoutConfig::default(),
            dashboard_client: None,
            staff_auth: vpay_config::StaffAuth::default(),
            routing: Default::default(),
        }
    }

    fn two_rails() -> BTreeMap<String, Box<dyn ProviderAdapter>> {
        adapters_by_code(vec![
            Box::new(TestRail {
                code: "mtn_momo",
                capabilities: coherent(ProviderFlow::Push),
                exposure: None,
            }),
            Box::new(TestRail {
                code: "orange_money",
                capabilities: coherent(ProviderFlow::Redirect),
                exposure: None,
            }),
        ])
    }

    /// What the exposure cases' rail declares. Names, never values.
    const EXPOSURE: AcceptedExposure = AcceptedExposure {
        credential: "public_key",
        operations: &["read any payment", "cancel any pending payment"],
        document: "docs/flows/adapter-notchpay.md",
    };

    /// `mtn_momo` declares nothing; `exposed` declares [`EXPOSURE`];
    /// `linked_only` declares it too and is never configured.
    fn rails_with_an_exposure() -> BTreeMap<String, Box<dyn ProviderAdapter>> {
        let rail = |code, exposure| -> Box<dyn ProviderAdapter> {
            Box::new(TestRail {
                code,
                capabilities: coherent(ProviderFlow::Push),
                exposure,
            })
        };
        adapters_by_code(vec![
            rail("mtn_momo", None),
            rail("exposed", Some(EXPOSURE)),
            rail("linked_only", Some(EXPOSURE)),
        ])
    }

    /// ADR-0031 condition 4: every configured rail that declares an
    /// exposure, `enabled` or not, and nothing else — not a rail that
    /// declares none, and not one the binary links and the YAML omits. The
    /// adapters come back through `adapters_by_code`, so this also holds
    /// `Measured` to forwarding the declaration.
    #[test]
    fn only_a_configured_rail_that_declares_an_exposure_is_warned_about() {
        let adapters = rails_with_an_exposure();
        let mut config = config_with(&["mtn_momo", "exposed"]);
        assert_eq!(
            accepted_exposures(&config, &adapters),
            vec![("exposed".to_owned(), EXPOSURE)]
        );
        for provider in &mut config.providers {
            provider.enabled = false;
        }
        assert_eq!(
            accepted_exposures(&config, &adapters),
            vec![("exposed".to_owned(), EXPOSURE)],
            "a disabled rail's keys are still loaded"
        );
        assert!(accepted_exposures(&config_with(&["mtn_momo"]), &adapters).is_empty());
    }

    /// What an operator reads, in both modes: a `WARN` naming the rail, the
    /// credential by its key, every operation and the page; louder in
    /// livemode; and never a credential's value.
    #[test]
    fn the_boot_warning_names_the_exposure_louder_in_livemode_and_never_a_key() {
        let adapters = rails_with_an_exposure();
        for livemode in [false, true] {
            let mut config = config_with(&["exposed"]);
            config.deployment.livemode = livemode;
            config
                .providers
                .first_mut()
                .expect("one rail configured")
                .credentials
                .insert("public_key".to_owned(), "public-key-test_0001".to_owned());
            let ((), log) = crate::test_log::with_captured_log(|| {
                warn_about_accepted_exposures(&config, &adapters);
            });
            assert!(log.contains("WARN"), "{log}");
            for expected in [
                "exposed",
                "ADR-0031",
                "credentials.public_key",
                "read any payment",
                "cancel any pending payment",
                "docs/flows/adapter-notchpay.md",
            ] {
                assert!(log.contains(expected), "{expected} missing from {log}");
            }
            assert_eq!(log.contains("LIVEMODE, REAL MONEY"), livemode, "{log}");
            // Livemode only boots acknowledged, and the line still says the
            // key is not private; a sandbox is told livemode will refuse.
            assert_eq!(
                log.contains("does not make the key private"),
                livemode,
                "{log}"
            );
            assert_eq!(log.contains("livemode boot refuses"), !livemode, "{log}");
            assert!(
                !log.contains("public-key-test_0001"),
                "a key reached the log: {log}"
            );
        }
        let ((), silent) = crate::test_log::with_captured_log(|| {
            warn_about_accepted_exposures(&config_with(&["mtn_momo"]), &adapters);
        });
        assert!(silent.is_empty(), "{silent}");
    }

    /// The join both binaries run, end to end: every configured rail is
    /// seeded with the *adapter's* capabilities and the *config's* enabled
    /// flag, and the currency code is uppercased for migration 0001's
    /// `code_is_iso4217_shape` CHECK (the YAML above spells it lowercase on
    /// purpose).
    /// ADR-0029 D6: an enabled rail must be reachable by some payment-method
    /// type; a disabled one may serve none, since nothing can be charged on
    /// it. Mutation-checked: deleting the refusal fails the first assertion.
    #[test]
    fn an_enabled_rail_serving_no_method_type_is_refused_and_a_disabled_one_is_not() {
        let adapters = adapters_by_code(vec![Box::new(TestRail {
            code: "mtn_momo",
            capabilities: Capabilities {
                method_types: vpay_core::routing::MethodTypes::NONE,
                ..coherent(ProviderFlow::Push)
            },
            exposure: None,
        }) as Box<dyn ProviderAdapter>]);
        let mut config = config_with(&["mtn_momo"]);
        let error = boot_seeds(&config, &adapters).expect_err("an unreachable rail");
        assert!(
            matches!(&error, ConfigError::RailServesNoMethodType { code } if code == "mtn_momo"),
            "{error:?}"
        );
        for provider in &mut config.providers {
            provider.enabled = false;
        }
        boot_seeds(&config, &adapters).expect("a disabled rail may serve nothing");
    }

    /// RFC-0008's 2026-10-23 revision: a weight on a type the rail's adapter
    /// does not serve could never apply. A weight for every type, and one on
    /// a type it serves, boot. Mutation-checked: deleting the refusal fails
    /// the first assertion.
    #[test]
    fn a_weight_on_a_type_the_rail_does_not_serve_is_refused_at_boot() {
        let adapters = adapters_by_code(vec![Box::new(TestRail {
            code: "mtn_momo",
            capabilities: Capabilities {
                method_types: vpay_core::routing::MethodTypes::of(&[
                    vpay_core::routing::MethodType::MtnWallet,
                ]),
                ..coherent(ProviderFlow::Push)
            },
            exposure: None,
        }) as Box<dyn ProviderAdapter>]);
        let weight = |method_type: Option<&str>| vpay_config::RoutingWeight {
            rail: "mtn_momo".to_owned(),
            method_type: method_type.map(str::to_owned),
            weight: 200,
        };
        let mut config = config_with(&["mtn_momo"]);
        config.routing.weights = vec![weight(Some("orange_wallet"))];
        let error = boot_seeds(&config, &adapters).expect_err("an unserved type");
        assert!(
            matches!(
                &error,
                ConfigError::RoutingWeightOnAnUnservedType { rail, method_type }
                    if rail == "mtn_momo" && method_type == "orange_wallet"
            ),
            "{error:?}"
        );
        assert!(!error.to_string().contains("  "), "{error}");

        config.routing.weights = vec![weight(None), weight(Some("mtn_wallet"))];
        boot_seeds(&config, &adapters).expect("weights the rail can use");
    }

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

    /// The labels `providers_flow_enum_check` accepts — migration 0002's
    /// `provider_flow` enum members until migration 0032 replaced the type
    /// with the CHECK, and the two variants `schemas/vpay.cstack`'s
    /// `ProviderFlow` enum can name since the provider pass moved onto
    /// CrateStack. A typo here fails boot in every deployment at once, as
    /// `DbError::ProviderFlowUnknown` (exit 78) rather than the
    /// `DbError::Query` this comment claimed;
    /// `a_flow_label_the_schema_cannot_name_is_refused_by_reconcile_before_any_row_is_written`
    /// in `vpay-db/tests/repositories.rs` is what proves `reconcile` refuses
    /// one, and
    /// `an_unknown_provider_flow_is_refused_by_the_check_that_replaced_the_enum_type`
    /// in `postgres_smoke.rs` that the database still refuses one from any
    /// other writer.
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
            method_type: None,
            amount: Money::new(5_000, vpay_core::Currency::Xaf).expect("5000 is non-negative"),
            payer_ref: None,
            ref_extra: std::collections::BTreeMap::new(),
            return_url: None,
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

    /// Boot step 4 refuses a configured rail whose adapter contradicts
    /// itself, and says so as a **configuration** failure — exit `78`, "fix
    /// the deploy" — rather than letting the seed reach migration 0002's
    /// `partial_refunds_imply_refunds` CHECK, which answers exit `1` about a
    /// database that is working perfectly (issue #61). Both numbers are
    /// measured against a real Postgres, in `main`'s order, by
    /// `boot_refuses_an_incoherent_rail_as_78_before_the_check_can_answer_it_as_1`
    /// in `backends/tests/integration/tests/boot_coherence.rs`; what is
    /// checked here is the refusal itself, which needs no container.
    ///
    /// **The message is asserted, not only the variant.** The message is the
    /// whole of what an operator gets, and a variant match alone passes with
    /// the provider code and the rule deleted from it — measured, by
    /// emptying `#[error(…)]` to `"configuration error"` and watching the
    /// original of this test stay green.
    ///
    /// **Both `enabled` values**, because `enabled` is a column on the row
    /// this seed becomes and not a reason to skip writing it: a disabled
    /// incoherent rail reaches the same CHECK, so narrowing the guard to
    /// `provider.enabled && !coherent` would be a hole rather than an
    /// optimisation.
    #[test]
    fn a_provider_with_incoherent_capabilities_is_a_config_error() {
        for enabled in [true, false] {
            let adapters = adapters_by_code(vec![Box::new(TestRail {
                code: "broken_rail",
                capabilities: Capabilities {
                    supports_refunds: false,
                    supports_partial_refunds: true,
                    ..coherent(ProviderFlow::Push)
                },
                exposure: None,
            })]);
            let mut config = config_with(&["broken_rail"]);
            config
                .providers
                .get_mut(0)
                .expect("the fixture configures one rail")
                .enabled = enabled;

            let error = boot_seeds(&config, &adapters)
                .expect_err("a rail that refunds partially but not at all must be refused");

            let ConfigError::IncoherentCapabilities { code } = &error else {
                panic!("expected IncoherentCapabilities, got {error:?} (enabled: {enabled})");
            };
            assert_eq!(code, "broken_rail");

            let message = error.to_string();
            assert!(
                message.contains("broken_rail"),
                "the message must name the rail an operator has to go and look at: {message}"
            );
            assert!(
                message.contains("partial_refunds_imply_refunds"),
                "the message must name the rule that was broken, spelled the way the migration \
                 and the database's own error spell it: {message}"
            );
        }
    }

    /// RFC-0013 § 8: a rail whose callback URL is registered with the
    /// account must be one that calls back. Refused at boot, on the line
    /// above's terms, with a message naming the rule.
    #[test]
    fn a_registered_callback_url_on_a_rail_that_never_calls_back_is_a_config_error() {
        let adapters = adapters_by_code(vec![Box::new(TestRail {
            code: "silent_rail",
            capabilities: Capabilities {
                delivers_callbacks: false,
                callback_url_mode: CallbackUrlMode::Registered,
                submit_steps: SubmitSteps::One,
                method_types: vpay_core::routing::MethodTypes::of(
                    &vpay_core::routing::MethodType::ALL,
                ),
                ..coherent(ProviderFlow::Push)
            },
            exposure: None,
        })]);
        let error = boot_seeds(&config_with(&["silent_rail"]), &adapters)
            .expect_err("a registered URL on a rail that never calls back must be refused");
        let ConfigError::IncoherentCallbackMode { code } = &error else {
            panic!("expected IncoherentCallbackMode, got {error:?}");
        };
        assert_eq!(code, "silent_rail");
        let message = error.to_string();
        assert!(message.contains("silent_rail"), "{message}");
        assert!(message.contains("callbacks_are_coherent"), "{message}");
        assert!(
            !message.contains("  "),
            "a wrapped #[error] literal must not keep its indentation: {message}"
        );
    }

    /// RFC-0013 § 2 and § 10's "Boot" line: a `callbacks` block on a rail
    /// that is told its URL on every request refuses to boot — whatever it
    /// says, `off` included, because the block itself is the mistake. The
    /// same block on a `Registered` rail boots.
    #[test]
    fn a_callbacks_block_on_a_per_request_rail_is_a_config_error() {
        for register in [
            vpay_config::CallbackRegistration::Ensure,
            vpay_config::CallbackRegistration::Check,
            vpay_config::CallbackRegistration::Off,
        ] {
            let mut config = config_with(&["mtn_momo"]);
            config
                .providers
                .get_mut(0)
                .expect("the fixture configures one rail")
                .callbacks = Some(vpay_config::ProviderCallbacks { register });
            let adapters = adapters_by_code(vec![Box::new(TestRail {
                code: "mtn_momo",
                capabilities: coherent(ProviderFlow::Push),
                exposure: None,
            })]);
            let error = boot_seeds(&config, &adapters)
                .expect_err("a callbacks block on a per-request rail must be refused");
            let ConfigError::CallbacksOnPerRequestRail { code } = &error else {
                panic!("expected CallbacksOnPerRequestRail, got {error:?} ({register:?})");
            };
            assert_eq!(code, "mtn_momo");
            assert!(
                error.to_string().contains("remove the callbacks block"),
                "the message must say what the fix is: {error}"
            );
            assert!(
                !error.to_string().contains("  "),
                "a wrapped #[error] literal must not keep its indentation: {error}"
            );

            let registered = adapters_by_code(vec![Box::new(TestRail {
                code: "mtn_momo",
                capabilities: Capabilities {
                    callback_url_mode: CallbackUrlMode::Registered,
                    submit_steps: SubmitSteps::One,
                    method_types: vpay_core::routing::MethodTypes::of(
                        &vpay_core::routing::MethodType::ALL,
                    ),
                    ..coherent(ProviderFlow::Push)
                },
                exposure: None,
            })]);
            assert!(
                boot_seeds(&config, &registered).is_ok(),
                "the same block on a registered rail is the setting working ({register:?})"
            );
        }
    }
}
