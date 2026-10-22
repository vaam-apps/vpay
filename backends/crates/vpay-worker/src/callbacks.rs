//! The callback-registration pass (RFC-0013 §§ 2 and 4): for every rail
//! whose adapter declares `CallbackUrlMode::Registered`, make sure the rail
//! account has one callback endpoint vpay created, at the rail's effective
//! callback URL.
//!
//! **The adapter knows the wire; this module knows when.** What a rail's
//! endpoint list looks like, where the marker goes and what counts as drift
//! are `ProviderAdapter::ensure_callback_endpoint`'s. The schedule, the lock,
//! the marker's spelling and the per-rail setting are here, as the poll
//! ladder is.
//!
//! Two callers run the same [`run_pass`]: the `ensure_callbacks` singleton
//! job ([`handle_ensure_callbacks`]), seeded at every worker start and
//! rescheduled hourly, and `vpay-server callbacks ensure [--check]`, which
//! runs one pass in the foreground. Neither is `vpay-server`'s boot: a boot
//! that waits on a provider is a deployment the provider can take down
//! (RFC-0013 § 2).
//!
//! # Why one rail's check is one transaction
//!
//! The lock is `pg_try_advisory_xact_lock` on
//! `vpay_db::lock_keys::CALLBACK_REGISTRATION`, taken at the top of a
//! transaction that stays open while the adapter lists and, at most once,
//! creates. Transaction-scoped so that a commit, a rollback or a dropped
//! connection releases it, and no path can hand a still-locked connection
//! back to the pool. One transaction **per rail**, not one per pass, so a
//! rail that takes its whole request budget to answer holds a pooled
//! connection for that rail only. The cost is that two concurrent passes may
//! split the rails between them, each reporting [`RailCheck::Locked`] for the
//! ones the other held: the next pass, an hour later, catches up, and no
//! rail is ever checked by two passes at once — which is the property Notch
//! Pay's non-idempotent create needs.
//!
//! # Why the job never fails
//!
//! [`handle_ensure_callbacks`] answers `Ok` on every path. A dead-lettered
//! singleton is parked at `run_at = 'infinity'` with its dedupe key still
//! taken, and `seed_singletons`' `ON CONFLICT DO NOTHING` could then never
//! revive it: registration would stop for good, silently (migration `0023`'s
//! header, RFC-0013 § 2). What a failure changes is the log line, the
//! [`CALLBACK_ENDPOINT_CHECKS_TOTAL`] label and how soon the next pass runs.

use std::collections::BTreeMap;
use std::time::Duration;

use vpay_core::FailureCode;
use vpay_core::metrics::{CALLBACK_ENDPOINT_CHECKS_TOTAL, callback_endpoint_state};
use vpay_db::{DbError, Repositories, TxOutcome, UnitOfWork as _};
use vpay_provider::{CallbackUrlMode, EndpointCheck, EndpointReport, EndpointState, ProviderError};

use crate::error::JobError;
use crate::handlers::{Adapters, RailConfigs};
use crate::jobs::Outcome;

/// How often the pass runs when the last one learned what it asked: hourly,
/// RFC-0013 open question 3's proposal, taken.
///
/// One list per registered rail an hour. What the schedule buys is drift
/// caught within the hour: an endpoint deleted in a dashboard, an event type
/// removed.
pub const ENSURE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How soon the pass runs again after a failure that the next attempt may
/// not meet: a rail that did not answer, or a database that could not open
/// the transaction. Five minutes, RFC-0013 § 2's proposal.
///
/// Not after a refusal: a revoked key or a wrong base URL is fixed by a
/// person, and asking again every five minutes would only repeat the alert.
pub const RETRY_SOON: Duration = Duration::from_secs(5 * 60);

/// The provider account every rail has until RFC-0009 phase B: a deployment
/// holds one account per rail, and its marker says `default`.
///
/// The same word `charges.provider_account_id` will record in phase A
/// (RFC-0009), so that the day a second account exists, the marker of the
/// first one already names it.
pub const DEFAULT_ACCOUNT: &str = "default";

/// `providers[].callbacks.register`, per rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    /// List, and create vpay's endpoint if it is missing. The default
    /// (RFC-0013 open question 1, answered by the maintainer on 2026-10-22).
    Ensure,
    /// List and report; never write. For a key that may read endpoints and
    /// not create them.
    Check,
    /// Never ask this rail about endpoints. The pass skips it.
    Off,
}

impl Registration {
    /// The YAML spelling, for log lines and the command's output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ensure => "ensure",
            Self::Check => "check",
            Self::Off => "off",
        }
    }
}

/// What the deployment says about registration: its name, which goes into
/// every marker, and each rail's [`Registration`].
///
/// Projected out of configuration by the binary, as [`RailConfigs`] is, so
/// nothing here reads a configuration document's shape (ADR-0003). A rail
/// absent from [`Self::rails`] is not asked: the binary lists every enabled
/// rail, so absence means "not configured here", never "use the default".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbackPlan {
    /// `deployment.name`.
    pub deployment: String,
    /// Each rail's setting, by `providers.code`.
    pub rails: BTreeMap<String, Registration>,
}

impl CallbackPlan {
    /// A plan that registers nothing: every rail is absent.
    ///
    /// For a caller that runs the job loop with no registered rail to
    /// configure — the integration suites, whose stub rails all declare
    /// `PerRequest`. The singleton is still seeded and still runs; it finds
    /// nothing to ask.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The marker that makes an endpoint this deployment's, on `rail`:
    /// `vpay:{deployment}:{rail}:{account}` (RFC-0013 § 3).
    ///
    /// The deployment's name is in it so two deployments sharing one sandbox
    /// account never take each other's endpoints for superseded ones. The
    /// account is [`DEFAULT_ACCOUNT`] until RFC-0009 phase B.
    #[must_use]
    pub fn marker(&self, rail: &str) -> String {
        format!("vpay:{}:{rail}:{DEFAULT_ACCOUNT}", self.deployment)
    }
}

/// What one rail's check came to.
#[derive(Debug)]
pub enum RailCheck {
    /// The adapter listed the account and answered.
    Answered(EndpointReport),
    /// Another pass held the registration lock. Nothing was asked.
    Locked,
    /// The rail is `off`. Nothing was asked. Reported so the command can say
    /// so; never counted.
    Off,
    /// The rail refused, did not answer, or answered something unreadable.
    Provider(ProviderError),
    /// The database could not open, lock or commit the transaction.
    Db(DbError),
}

/// One rail's line in a pass.
#[derive(Debug)]
pub struct RailPass {
    /// `providers.code`.
    pub rail: String,
    /// What the deployment asked for on this rail.
    pub registration: Registration,
    /// The marker the pass looked for.
    pub marker: String,
    /// What it found.
    pub check: RailCheck,
}

impl RailPass {
    /// The [`callback_endpoint_state`] label, or `None` for a rail that is
    /// `off` and so was not checked.
    #[must_use]
    pub const fn state(&self) -> Option<&'static str> {
        match &self.check {
            RailCheck::Answered(report) => Some(match report.state {
                EndpointState::Present { .. } => callback_endpoint_state::PRESENT,
                EndpointState::Created { .. } => callback_endpoint_state::CREATED,
                EndpointState::Drifted { .. } => callback_endpoint_state::DRIFTED,
                EndpointState::Foreign { .. } => callback_endpoint_state::FOREIGN,
                EndpointState::Missing => callback_endpoint_state::MISSING,
            }),
            RailCheck::Locked => Some(callback_endpoint_state::LOCKED),
            RailCheck::Off => None,
            RailCheck::Provider(_) | RailCheck::Db(_) => Some(callback_endpoint_state::FAILED),
        }
    }

    /// Whether the rail is left with an endpoint at vpay's URL, as far as
    /// this pass knows: the command's exit status (RFC-0013 § 2).
    ///
    /// `Drifted` and `Foreign` count: each is an endpoint at vpay's URL, and
    /// each is reported in its own words. A rail that is `off` counts too —
    /// nobody asked the pass to look. `Locked` does not: the pass does not
    /// know, and a command that exits `0` on "did not look" is one an
    /// operator stops reading.
    #[must_use]
    pub const fn has_endpoint(&self) -> bool {
        match &self.check {
            RailCheck::Answered(report) => !matches!(report.state, EndpointState::Missing),
            RailCheck::Off => true,
            RailCheck::Locked | RailCheck::Provider(_) | RailCheck::Db(_) => false,
        }
    }

    /// Whether the next pass should come [`RETRY_SOON`] rather than in an
    /// hour: the rail did not answer, or the database did not.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(
            self.check,
            RailCheck::Provider(ProviderError::Transport { .. }) | RailCheck::Db(_)
        )
    }
}

/// The transaction's failure: a closure handed to `UnitOfWork::transaction`
/// returns one error type, and it must be `From<DbError>`.
enum PassFailure {
    Db(DbError),
    Provider(ProviderError),
}

impl From<DbError> for PassFailure {
    fn from(error: DbError) -> Self {
        Self::Db(error)
    }
}

/// Runs one pass over every registered rail, and answers one [`RailPass`]
/// per rail it considered.
///
/// A rail is considered when its adapter declares
/// `CallbackUrlMode::Registered`, the plan names it and the deployment has a
/// configuration for it. A rail that declares `PerRequest` is never asked
/// anything (RFC-0013 § 1). `check_only` forces `check` on every rail, for
/// `vpay-server callbacks ensure --check`; it never turns an `off` rail on.
///
/// Logs one line per rail and counts it in
/// [`CALLBACK_ENDPOINT_CHECKS_TOTAL`]. Never fails: a failure is a
/// [`RailCheck`], so that one rail's outage cannot hide another rail's
/// answer.
pub async fn run_pass(
    repositories: &dyn Repositories,
    adapters: &Adapters,
    rails: &RailConfigs,
    plan: &CallbackPlan,
    check_only: bool,
) -> Vec<RailPass> {
    let mut passes = Vec::new();
    for (code, adapter) in adapters {
        if adapter.capabilities().callback_url_mode != CallbackUrlMode::Registered {
            continue;
        }
        let (Some(&registration), Some(config)) = (plan.rails.get(code), rails.get(code)) else {
            continue;
        };
        let marker = plan.marker(code);
        let check = if registration == Registration::Off {
            RailCheck::Off
        } else {
            let write = registration == Registration::Ensure && !check_only;
            check_one(repositories, adapter.as_ref(), config, &marker, write).await
        };
        let pass = RailPass {
            rail: code.clone(),
            registration,
            marker,
            check,
        };
        report(&pass, config.callback_url.as_str());
        passes.push(pass);
    }
    passes
}

/// One rail, under the lock, in its own transaction.
async fn check_one(
    repositories: &dyn Repositories,
    adapter: &dyn vpay_provider::ProviderAdapter,
    config: &vpay_provider::ProviderConfig,
    marker: &str,
    write: bool,
) -> RailCheck {
    let check = EndpointCheck { marker, write };
    let answer = repositories
        .transaction(|tx| {
            let check = &check;
            Box::pin(async move {
                if !tx.try_lock_callback_registration().await? {
                    return Ok(TxOutcome::Abandon(None));
                }
                let report = adapter
                    .ensure_callback_endpoint(check, config)
                    .await
                    .map_err(PassFailure::Provider)?;
                Ok::<_, PassFailure>(TxOutcome::Commit(Some(report)))
            })
        })
        .await;
    match answer.map(TxOutcome::into_inner) {
        Ok(Some(report)) => RailCheck::Answered(report),
        Ok(None) => RailCheck::Locked,
        Err(PassFailure::Provider(error)) => RailCheck::Provider(error),
        Err(PassFailure::Db(error)) => RailCheck::Db(error),
    }
}

/// One log line and one count for one rail.
///
/// The levels are RFC-0013 § 2's table: a refused key is an `error` with
/// `alert`, a rail that did not answer is a `warn` and comes back sooner, an
/// answer that does not parse is an `error`. A configuration refusal — a
/// wrong base URL, a callback URL the rail will not take — alerts too,
/// because only a person fixes it. The callback URL is logged: it is the
/// deployment's own public address, never a secret.
fn report(pass: &RailPass, callback_url: &str) {
    if let Some(state) = pass.state() {
        metrics::counter!(
            CALLBACK_ENDPOINT_CHECKS_TOTAL,
            "provider" => pass.rail.clone(),
            "state" => state,
        )
        .increment(1);
    }
    let rail = pass.rail.as_str();
    let registration = pass.registration.as_str();
    let marker = pass.marker.as_str();
    match &pass.check {
        RailCheck::Answered(report) => {
            let retirable = report.retirable.len();
            match &report.state {
                EndpointState::Present {
                    endpoint_id,
                    repaired,
                } => tracing::info!(
                    rail,
                    registration,
                    marker,
                    callback_url,
                    endpoint_id = %endpoint_id,
                    repaired,
                    retirable,
                    "the callback endpoint is registered"
                ),
                EndpointState::Created { endpoint_id } => tracing::info!(
                    rail,
                    registration,
                    marker,
                    callback_url,
                    endpoint_id = %endpoint_id,
                    retirable,
                    "registered the callback endpoint with the rail account"
                ),
                EndpointState::Drifted {
                    endpoint_id,
                    detail,
                } => tracing::warn!(
                    rail,
                    registration,
                    marker,
                    callback_url,
                    endpoint_id = %endpoint_id,
                    detail = %detail,
                    retirable,
                    "vpay's callback endpoint is registered but not as vpay registers it, and this \
                     rail cannot put it right; fix it at the provider"
                ),
                EndpointState::Foreign { endpoint_id } => tracing::warn!(
                    rail,
                    registration,
                    marker,
                    callback_url,
                    endpoint_id = %endpoint_id,
                    "an endpoint vpay did not create is registered at vpay's callback URL; it was \
                     left as found, and no second one was created beside it"
                ),
                EndpointState::Missing => tracing::warn!(
                    rail,
                    registration,
                    marker,
                    callback_url,
                    retirable,
                    "no callback endpoint is registered at vpay's callback URL, and this pass \
                     may not create one; callbacks will not arrive, and charges settle by polling"
                ),
            }
            if retirable > 0 {
                tracing::warn!(
                    rail,
                    marker,
                    endpoints = ?report.retirable,
                    "this deployment's marker is on endpoints that are not the current one; vpay \
                     does not retire them on this rail, so each one delivers callbacks a second \
                     time, or to an address this deployment no longer serves"
                );
            }
        }
        RailCheck::Locked => tracing::info!(
            rail,
            registration,
            "another callback registration pass holds the lock; this rail is left to it"
        ),
        RailCheck::Off => tracing::info!(
            rail,
            "callback registration is off for this rail; its endpoint is the operator's to manage"
        ),
        RailCheck::Provider(error) => log_provider_failure(rail, registration, error),
        RailCheck::Db(error) => tracing::error!(
            rail,
            registration,
            error = %error,
            "could not open the callback registration transaction; the pass will run again in \
             five minutes"
        ),
    }
}

fn log_provider_failure(rail: &str, registration: &str, error: &ProviderError) {
    match error {
        ProviderError::Rejected {
            code: FailureCode::ProviderAccountBlocked,
            ..
        } => tracing::error!(
            alert = true,
            rail,
            registration,
            error = %error,
            "the rail refused this deployment's keys while checking its callback endpoint"
        ),
        ProviderError::Config(_) => tracing::error!(
            alert = true,
            rail,
            registration,
            error = %error,
            "the rail refused the callback endpoint check as misconfigured"
        ),
        ProviderError::Transport { .. } => tracing::warn!(
            rail,
            registration,
            error = %error,
            "the rail did not answer the callback endpoint check; the pass will run again in five \
             minutes, and a create whose answer was lost is not repeated before it lists again"
        ),
        ProviderError::NotImplemented(_) | ProviderError::Unsupported => tracing::warn!(
            rail,
            registration,
            error = %error,
            "this rail registers its callback URL with the account, and its adapter cannot do \
             that yet; the endpoint is the operator's to register by hand"
        ),
        ProviderError::Rejected { .. } | ProviderError::Malformed { .. } => tracing::error!(
            rail,
            registration,
            error = %error,
            "the callback endpoint check failed"
        ),
    }
}

/// The `ensure_callbacks` job: one [`run_pass`], then a reschedule.
///
/// Never `Err`, and never [`Outcome::Done`] — see this module's header for
/// why a singleton that may not dead-letter answers this way.
///
/// # Errors
///
/// None. The `Result` is the dispatch table's shape.
pub async fn handle_ensure_callbacks(
    repositories: &dyn Repositories,
    adapters: &Adapters,
    rails: &RailConfigs,
    plan: &CallbackPlan,
) -> Result<Outcome, JobError> {
    let passes = run_pass(repositories, adapters, rails, plan, false).await;
    let delay = if passes.iter().any(RailPass::is_transient) {
        RETRY_SOON
    } else {
        ENSURE_INTERVAL
    };
    Ok(Outcome::RescheduleAfter(delay))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(check: RailCheck) -> RailPass {
        RailPass {
            rail: "notchpay".to_owned(),
            registration: Registration::Ensure,
            marker: "vpay:prod:notchpay:default".to_owned(),
            check,
        }
    }

    fn answered(state: EndpointState) -> RailCheck {
        RailCheck::Answered(EndpointReport {
            state,
            retirable: Vec::new(),
        })
    }

    #[test]
    fn the_marker_names_the_deployment_the_rail_and_the_default_account() {
        let plan = CallbackPlan {
            deployment: "acme-prod".to_owned(),
            rails: BTreeMap::new(),
        };
        assert_eq!(plan.marker("notchpay"), "vpay:acme-prod:notchpay:default");
        assert_ne!(
            plan.marker("notchpay"),
            CallbackPlan {
                deployment: "acme-staging".to_owned(),
                rails: BTreeMap::new(),
            }
            .marker("notchpay"),
            "two deployments on one sandbox account must not share a marker"
        );
    }

    #[test]
    fn the_settings_spell_themselves_as_the_yaml_does() {
        assert_eq!(Registration::Ensure.as_str(), "ensure");
        assert_eq!(Registration::Check.as_str(), "check");
        assert_eq!(Registration::Off.as_str(), "off");
    }

    /// The command's exit status: non-zero exactly when the pass does not
    /// know of an endpoint at vpay's URL.
    #[test]
    fn a_rail_has_an_endpoint_unless_it_is_missing_or_the_pass_did_not_learn() {
        let id = || "wh_1".to_owned();
        assert!(
            pass(answered(EndpointState::Present {
                endpoint_id: id(),
                repaired: false,
            }))
            .has_endpoint()
        );
        assert!(pass(answered(EndpointState::Created { endpoint_id: id() })).has_endpoint());
        assert!(
            pass(answered(EndpointState::Drifted {
                endpoint_id: id(),
                detail: "events".to_owned(),
            }))
            .has_endpoint()
        );
        assert!(pass(answered(EndpointState::Foreign { endpoint_id: id() })).has_endpoint());
        assert!(pass(RailCheck::Off).has_endpoint());

        assert!(!pass(answered(EndpointState::Missing)).has_endpoint());
        assert!(!pass(RailCheck::Locked).has_endpoint());
        assert!(
            !pass(RailCheck::Provider(ProviderError::Transport {
                context: "timed out".to_owned(),
                source: None,
            }))
            .has_endpoint()
        );
    }

    /// Only a rail that did not answer comes back sooner. A refusal is a
    /// person's to fix; asking every five minutes repeats the alert.
    #[test]
    fn only_a_rail_that_did_not_answer_is_asked_again_sooner() {
        assert!(
            pass(RailCheck::Provider(ProviderError::Transport {
                context: "connection reset".to_owned(),
                source: None,
            }))
            .is_transient()
        );
        assert!(
            !pass(RailCheck::Provider(ProviderError::Rejected {
                code: FailureCode::ProviderAccountBlocked,
                message: "keys refused".to_owned(),
            }))
            .is_transient()
        );
        assert!(!pass(RailCheck::Provider(ProviderError::Config("404".to_owned()))).is_transient());
        assert!(!pass(answered(EndpointState::Missing)).is_transient());
        assert!(!pass(RailCheck::Locked).is_transient());
    }

    #[test]
    fn every_check_but_off_is_counted_under_one_of_the_seven_states() {
        let states = [
            pass(answered(EndpointState::Present {
                endpoint_id: "a".to_owned(),
                repaired: false,
            }))
            .state(),
            pass(answered(EndpointState::Created {
                endpoint_id: "a".to_owned(),
            }))
            .state(),
            pass(answered(EndpointState::Drifted {
                endpoint_id: "a".to_owned(),
                detail: String::new(),
            }))
            .state(),
            pass(answered(EndpointState::Foreign {
                endpoint_id: "a".to_owned(),
            }))
            .state(),
            pass(answered(EndpointState::Missing)).state(),
            pass(RailCheck::Locked).state(),
            pass(RailCheck::Provider(ProviderError::Unsupported)).state(),
        ];
        assert_eq!(
            states,
            [
                Some("present"),
                Some("created"),
                Some("drifted"),
                Some("foreign"),
                Some("missing"),
                Some("locked"),
                Some("failed"),
            ]
        );
        assert_eq!(pass(RailCheck::Off).state(), None);
    }
}
