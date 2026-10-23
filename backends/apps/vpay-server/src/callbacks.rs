//! `vpay-server callbacks ensure [--check]`, and the projection both it and
//! `vpay-server worker` build the registration plan with (RFC-0013).
//!
//! The pass itself is `vpay_worker::callbacks::run_pass`, one implementation
//! for the command and the `ensure_callbacks` job, under one lock. What is
//! here is what only a binary can do: read the deployment's YAML into a
//! [`CallbackPlan`], and print.
//!
//! **Not part of serving.** Nothing in `vpay-server`'s boot asks a rail about
//! endpoints — RFC-0013 § 2: a boot that waits on a provider is a deployment
//! the provider can take down.

use std::collections::BTreeMap;

use anyhow::Context as _;
use vpay_config::{CallbackRegistration, CallbacksCommand, Config, ServerArgs};
use vpay_provider::EndpointState;
use vpay_worker::{CallbackPlan, RailCheck, RailPass, Registration};

use crate::StartupError;

/// The deployment's registration plan: its name, and each **enabled** rail's
/// `providers[].callbacks.register` — or, where the operator set none, the
/// default the rail's adapter declares
/// (`ProviderAdapter::default_callback_registration`): `ensure` unless the
/// adapter says otherwise, `off` for `notchpay` since 2026-10-23.
///
/// Every enabled rail is listed, `PerRequest` ones included: which rails are
/// asked is the adapter's capability, read by the pass, never a list kept
/// here, and so is which rails are written to unasked. A rail with
/// `enabled: false` is left out — an operator who turned a rail off has not
/// asked vpay to write to its account. A rail with no linked adapter keeps
/// RFC-0013's `ensure`; boot refuses one before this runs, and the pass
/// only ever asks rails it has an adapter for.
pub(crate) fn callback_plan(config: &Config, adapters: &vpay_worker::Adapters) -> CallbackPlan {
    CallbackPlan {
        deployment: config.deployment.name.clone(),
        rails: config
            .providers
            .iter()
            .filter(|provider| provider.enabled)
            .map(|provider| {
                let configured = provider.callbacks.and_then(|callbacks| callbacks.register);
                let register = configured.map_or_else(
                    || {
                        adapters
                            .get(provider.code.as_str())
                            .map_or(Registration::Ensure, |adapter| {
                                Registration::from(adapter.default_callback_registration())
                            })
                    },
                    registration,
                );
                if configured.is_none() && register == Registration::Off {
                    tracing::info!(
                        rail = %provider.code,
                        "providers[].callbacks.register is unset and this rail's adapter \
                         defaults to `off`: vpay will not create its callback endpoint. \
                         Register vpay's callback URL in the rail's dashboard, or set \
                         `register: ensure`; charges settle by polling either way"
                    );
                }
                (provider.code.clone(), register)
            })
            .collect::<BTreeMap<_, _>>(),
    }
}

/// The YAML's enum onto the worker's. Two enums because `vpay-worker` reads
/// no configuration document (ADR-0003), and the conversion is total.
const fn registration(register: CallbackRegistration) -> Registration {
    match register {
        CallbackRegistration::Ensure => Registration::Ensure,
        CallbackRegistration::Check => Registration::Check,
        CallbackRegistration::Off => Registration::Off,
    }
}

/// Runs `vpay-server callbacks …`.
///
/// # Errors
///
/// A configuration that will not load or join its adapters (exit `78`), a
/// database that will not open (exit `69` or `78`), and — once the pass has
/// run and printed — any rail left with no endpoint at vpay's callback URL,
/// or one the pass could not find out about.
pub(crate) async fn run(args: &ServerArgs, command: CallbacksCommand) -> anyhow::Result<()> {
    match command {
        CallbacksCommand::Ensure { check } => ensure(args, check).await,
    }
}

/// One pass, in the foreground, then one line per rail on stdout.
///
/// Configuration and the adapter join first, as every mode's boot does, so a
/// `callbacks` block on a per-request rail is refused here exactly as the
/// worker refuses it. **No reconcile**: the command writes nothing to vpay's
/// tables, and the advisory lock needs only a connection.
async fn ensure(args: &ServerArgs, check: bool) -> anyhow::Result<()> {
    let config =
        vpay_api::boot::load_config(args.common.config.as_deref(), &args.common.profile)
            .context("loading and validating configuration (--config / VPAY_CONFIG, ADR-0003)")?;

    let http = vpay_provider::http::client_with_timeouts(
        vpay_provider::DEFAULT_CONNECT_TIMEOUT,
        vpay_provider::DEFAULT_REQUEST_TIMEOUT,
    )
    .context("building the outbound HTTP client every rail adapter shares")?;
    let adapters = vpay_api::boot::adapters_by_code(vpay_server::adapters(http));
    // For its refusals only: the seeds are the reconcile's, and this command
    // does not reconcile.
    vpay_api::boot::boot_seeds(&config, &adapters)?;
    vpay_api::boot::warn_about_accepted_exposures(&config, &adapters);

    let database_url = args
        .common
        .database_url
        .as_deref()
        .ok_or(StartupError::MissingDatabaseUrl)?;
    let repositories = vpay_api::boot::open_migrated_database(database_url).await?;

    let resource_config = vpay_api::ResourceConfig::from_config(&config)
        .context("projecting the deployment configuration onto the provider port")?;
    let rails = crate::worker::project_rails(&resource_config, &adapters);
    let plan = callback_plan(&config, &adapters);

    let passes =
        vpay_worker::run_pass(repositories.as_ref(), &adapters, &rails, &plan, check).await;
    if passes.is_empty() {
        tracing::info!(
            "no configured rail registers its callback URL with the account; there is nothing \
             to check"
        );
    }
    // stdout, one line per rail, never through `tracing`: `staff add`'s
    // reasoning, and `logs_to_stderr` puts this subcommand's logs on stderr
    // so these lines are the whole of stdout.
    #[allow(
        clippy::print_stdout,
        reason = "the per-rail report is this command's output, for a runbook to read; \
                  `install_process_defaults` puts a subcommand's logs on stderr"
    )]
    for pass in &passes {
        println!("{}", line(pass));
    }

    let without: Vec<&str> = passes
        .iter()
        .filter(|pass| !pass.has_endpoint())
        .map(|pass| pass.rail.as_str())
        .collect();
    anyhow::ensure!(
        without.is_empty(),
        "{} rail(s) not known to have a callback endpoint at vpay's URL: {}; callbacks will not \
         arrive from them, and their charges settle by polling",
        without.len(),
        without.join(", ")
    );
    Ok(())
}

/// One rail's line: `{rail} {register} {state} {detail}`, space-separated, so
/// a runbook can `awk` it. The detail is the endpoint's id, or what went
/// wrong — never a key: a `ProviderError`'s message carries none.
fn line(pass: &RailPass) -> String {
    let state = pass.state().unwrap_or("off");
    let detail = match &pass.check {
        RailCheck::Answered(report) => match &report.state {
            EndpointState::Present { endpoint_id, .. }
            | EndpointState::Created { endpoint_id }
            | EndpointState::Foreign { endpoint_id } => endpoint_id.clone(),
            EndpointState::Drifted {
                endpoint_id,
                detail,
            } => format!("{endpoint_id} ({detail})"),
            EndpointState::Missing => "-".to_owned(),
        },
        RailCheck::Locked => "another pass holds the lock".to_owned(),
        RailCheck::Off => "-".to_owned(),
        RailCheck::Provider(error) => error.to_string(),
        RailCheck::Db(error) => error.to_string(),
    };
    format!(
        "{} {} {state} {detail}",
        pass.rail,
        pass.registration.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpay_config::{Deployment, HostEntry, ProviderCallbacks, ProviderHost};
    use vpay_provider::EndpointReport;

    fn host(code: &str, enabled: bool, register: Option<CallbackRegistration>) -> ProviderHost {
        ProviderHost {
            code: code.to_owned(),
            enabled,
            host: HostEntry {
                url: format!("https://{code}.example"),
                label: code.to_owned(),
            },
            settings: BTreeMap::new(),
            callback_url: None,
            currency: "XAF".to_owned(),
            credentials: BTreeMap::new(),
            display_name: None,
            callbacks: register.map(|register| ProviderCallbacks {
                register: Some(register),
            }),
        }
    }

    fn config(providers: Vec<ProviderHost>) -> Config {
        Config {
            deployment: Deployment {
                name: "acme-prod".to_owned(),
                livemode: false,
                public_base_url: "https://pay.example".to_owned(),
                surfaces: None,
            },
            providers,
            currencies: Vec::new(),
            merchant_clients: Vec::new(),
            webhooks: vpay_config::WebhookPolicy::default(),
            checkout: vpay_config::CheckoutConfig::default(),
            dashboard_client: None,
            staff_auth: vpay_config::StaffAuth::default(),
            routing: Default::default(),
        }
    }

    /// The adapters `vpay-server` links, each wrapped in `Measured` as boot
    /// wraps them, so a decorator that took the port's default instead of
    /// forwarding Notch Pay's `off` fails here too.
    fn linked_adapters() -> vpay_worker::Adapters {
        let http = vpay_provider::http::client_with_timeouts(
            vpay_provider::DEFAULT_CONNECT_TIMEOUT,
            vpay_provider::DEFAULT_REQUEST_TIMEOUT,
        )
        .expect("the shared client builds without a network");
        vpay_api::boot::adapters_by_code(vpay_server::adapters(http))
    }

    /// An explicit `register` always wins; an unset one is the rail's own
    /// default, which its adapter declares: `off` for `notchpay` (RFC-0013
    /// § 2 as amended 2026-10-23), RFC-0013's `ensure` for every other rail,
    /// and the same for a rail no linked adapter answers for.
    #[test]
    fn the_plan_reads_each_rails_setting_else_its_adapters_default() {
        let adapters = linked_adapters();
        let plan = callback_plan(
            &config(vec![
                host("notchpay", true, None),
                host("mtn_momo", true, None),
                host("second", true, Some(CallbackRegistration::Check)),
                host("third", true, Some(CallbackRegistration::Off)),
                host("fourth", false, Some(CallbackRegistration::Ensure)),
            ]),
            &adapters,
        );
        assert_eq!(plan.deployment, "acme-prod");
        assert_eq!(
            plan.rails,
            BTreeMap::from([
                ("notchpay".to_owned(), Registration::Off),
                ("mtn_momo".to_owned(), Registration::Ensure),
                ("second".to_owned(), Registration::Check),
                ("third".to_owned(), Registration::Off),
            ]),
            "a disabled rail is not in the plan"
        );
        assert_eq!(plan.marker("notchpay"), "vpay:acme-prod:notchpay:default");

        // `callbacks: {}` is the absent block, and an explicit `ensure`
        // overrides Notch Pay's `off`.
        let mut empty_block = host("notchpay", true, None);
        empty_block.callbacks = Some(ProviderCallbacks { register: None });
        assert_eq!(
            callback_plan(&config(vec![empty_block]), &adapters)
                .rails
                .get("notchpay")
                .copied(),
            Some(Registration::Off)
        );
        let asked = callback_plan(
            &config(vec![host(
                "notchpay",
                true,
                Some(CallbackRegistration::Ensure),
            )]),
            &adapters,
        );
        assert_eq!(
            asked.rails.get("notchpay").copied(),
            Some(Registration::Ensure)
        );
    }

    #[test]
    fn a_line_is_rail_setting_state_and_detail() {
        let pass = |check| RailPass {
            rail: "notchpay".to_owned(),
            registration: Registration::Ensure,
            marker: "vpay:acme-prod:notchpay:default".to_owned(),
            check,
        };
        assert_eq!(
            line(&pass(RailCheck::Answered(EndpointReport {
                state: EndpointState::Created {
                    endpoint_id: "wh_1".to_owned(),
                },
                retirable: Vec::new(),
            }))),
            "notchpay ensure created wh_1"
        );
        assert_eq!(
            line(&pass(RailCheck::Answered(EndpointReport {
                state: EndpointState::Missing,
                retirable: Vec::new(),
            }))),
            "notchpay ensure missing -"
        );
        assert_eq!(line(&pass(RailCheck::Off)), "notchpay ensure off -");
        assert_eq!(
            line(&pass(RailCheck::Locked)),
            "notchpay ensure locked another pass holds the lock"
        );
    }
}
