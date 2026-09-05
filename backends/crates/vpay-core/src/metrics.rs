//! Every metric name vpay emits, and their descriptions — the names only,
//! never a recorder.
//!
//! Installing a process-wide recorder is an *application* decision, so this
//! module owns the vocabulary and each binary owns the exporter it renders
//! through; [`describe_all`] is the one call that connects the two, and it is
//! a no-op until a recorder exists. Why a library must not install one, why
//! the names are `const`s, and where each of the twelve is emitted:
//! [docs/reference/vpay-core.md § metrics](../../../../docs/reference/vpay-core.md#metrics).
//!
//! # The list
//!
//! This block is the specification (`docs/plans/2026-09-03-step6-deployment.md`
//! §3, transcribed verbatim). [`ALL`] must match it exactly, and
//! `the_module_doc_list_and_the_all_constant_agree` below reads *this file*
//! to prove it — so a metric added to the code without a line here, or a line
//! here with no metric, fails the build.
//!
//! ```text
//! vpay_build_info{version,git_sha}                                  gauge
//! vpay_http_requests_total{route,method,status}                     counter
//! vpay_http_request_duration_seconds{route,method,status}           histogram
//! vpay_provider_requests_total{provider,operation,error_kind}       counter
//! vpay_provider_request_duration_seconds{provider,operation}        histogram
//! vpay_charge_transitions_total{provider,from,to}                   counter
//! vpay_jobs_claimed_total{kind}                                     counter
//! vpay_jobs_completed_total{kind,outcome}                           counter
//! vpay_jobs_oldest_claimable_age_seconds                            gauge
//! vpay_webhook_deliveries_total{outcome}                            counter
//! vpay_error_events_total{category,code,severity}                   counter
//! vpay_alert_events_total{category,code}                            counter
//! vpay_account_holder_lookups_total{outcome}                        counter
//! ```
//!
//! The last line is the only one that is **not** in that plan document: it
//! landed with `GET /v1/account_holders` (issue #47), after it was written.
//! It is here rather than folded into `vpay_http_requests_total` because the
//! four outcomes a lookup has are not four status codes — `found` and
//! `not_found` are both `200`, and telling them apart is the whole question
//! an operator asks of this route.
//!
//! A described-but-unrecorded metric never appears in a scrape (the
//! Prometheus exporter renders registered *handles*, not descriptions), so a
//! name can be declared here before its seam exists without making a
//! deployment look more instrumented than it is.

use metrics::{Unit, counter, describe_counter, describe_gauge, describe_histogram, gauge};

use crate::error::{Classify, Severity};

/// `1`, labelled with this build's version and git sha.
///
/// The Prometheus "info metric" idiom: the value carries nothing, the labels
/// are the payload, so `vpay_build_info` joined against any other series
/// answers "which build produced this". Set once per process at startup by
/// [`record_build_info`].
pub const BUILD_INFO: &str = "vpay_build_info";

/// HTTP responses served, by route pattern, method and status.
///
/// Both `route` and `method` are bounded labels over unbounded inputs — the
/// axum path *pattern* (never the concrete path) with `route="unmatched"`,
/// and the ten methods `http::Method` names with `method="other"`. Why each
/// bound exists is in
/// [docs/reference/vpay-core.md § http request labels](../../../../docs/reference/vpay-core.md#http-request-labels).
///
/// Emitted by `vpay_api`'s `track_http_metrics` middleware and by nothing
/// else. The observability listener's own `/livez` and `/metrics` are **not**
/// counted: a scraper polling every 15s would otherwise be the largest
/// traffic source on the series.
pub const HTTP_REQUESTS_TOTAL: &str = "vpay_http_requests_total";

/// Wall-clock seconds per HTTP response, same labels and same seam as
/// [`HTTP_REQUESTS_TOTAL`].
///
/// Measured around the *inner* service: it excludes the request-id and trace
/// layers above it and includes routing, authentication, the handler and the
/// error renderer — the span an operator can act on.
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "vpay_http_request_duration_seconds";

/// Rail calls, by provider code, port operation and failure kind.
///
/// `error_kind` is `""` on success and otherwise [`Classify::code`] — the
/// same vocabulary the `provider_requests.error_kind` column stores, so a
/// Prometheus alert and the SQL an operator runs against that table use one
/// set of words.
///
/// Emitted by `vpay_provider::Measured`, the port decorator every adapter is
/// wrapped in — one seam for every rail, and outside the adapter crates, so
/// instrumenting a rail is not a provider-code branch (ADR-0002).
///
/// **It counts port calls, not wire requests**, and the difference is worth
/// knowing before reading a graph of it:
/// [docs/reference/vpay-core.md § port calls, not wire requests](../../../../docs/reference/vpay-core.md#port-calls-not-wire-requests).
pub const PROVIDER_REQUESTS_TOTAL: &str = "vpay_provider_requests_total";

/// Wall-clock seconds per rail call. Deliberately not labelled by
/// `error_kind`: a latency histogram split by every failure mode is mostly
/// empty buckets, and "is this rail slow" does not depend on why a call
/// failed. Same seam and same port-call caveat as
/// [`PROVIDER_REQUESTS_TOTAL`].
pub const PROVIDER_REQUEST_DURATION_SECONDS: &str = "vpay_provider_request_duration_seconds";

/// Charge state transitions, by rail and by the pair of states.
///
/// The `from`/`to` pair rather than a single `state` label because the
/// interesting quantity is the *edge*: "how many charges went
/// `submitted` → `failed`" is an alertable rate, "how many are `failed`" is a
/// database query.
///
/// Emitted through `vpay_db::charges::record_transition`, for the six
/// statements that can move `charges.state` and for nothing else — the
/// database layer rather than the worker's settlement points, because a
/// confirm opens and submits a charge inside `vpay-api` and a worker-mounted
/// metric would silently miss it. **Counted after the transition commits**,
/// and every label is read off the row the database returned; the one caveat,
/// on the `from` label alone, is in
/// [docs/reference/vpay-core.md § charge transitions are counted after commit](../../../../docs/reference/vpay-core.md#charge-transitions-are-counted-after-commit).
pub const CHARGE_TRANSITIONS_TOTAL: &str = "vpay_charge_transitions_total";

/// Jobs claimed off the queue by this worker, by `jobs.kind`.
///
/// Emitted by `vpay_worker::run_once`, on the arm where a claim returned a
/// row.
pub const JOBS_CLAIMED_TOTAL: &str = "vpay_jobs_claimed_total";

/// Jobs settled by this worker, by `jobs.kind` and by what ended the lease.
///
/// See [`job_outcome`] for the label values.
pub const JOBS_COMPLETED_TOTAL: &str = "vpay_jobs_completed_total";

/// How far behind the queue is: `now - min(run_at)` over unleased, unparked
/// rows, in seconds.
///
/// A property of the *table*, so every replica reports the same value and a
/// deployment can alert on it without summing across pods. Zero when the
/// queue holds nothing runnable, and left unwritten when the read itself
/// failed.
///
/// **The name says `claimable`, and the value goes negative on a healthy
/// idle deployment** — it is "seconds until (negative) or since (positive)
/// the next queued job was due". A `> 300` alert is unaffected; a dashboard
/// that renders it as an "age" is not. See
/// [docs/reference/vpay-core.md § the queue gauge goes negative](../../../../docs/reference/vpay-core.md#the-queue-gauge-goes-negative).
pub const JOBS_OLDEST_CLAIMABLE_AGE_SECONDS: &str = "vpay_jobs_oldest_claimable_age_seconds";

/// Webhook delivery attempts, by outcome.
///
/// Emitted from `vpay_worker::webhooks::handle_deliver` at the two points a
/// delivery attempt's outcome becomes durable — after `record_success`'s
/// compare-and-swap actually changed a row, and after `record_attempt`
/// commits, where the ladder index decides [`webhook_outcome::RETRY`] or
/// [`webhook_outcome::EXHAUSTED`]. Both are single `UPDATE`s against the
/// pool, so "after `.await?` returns `Ok`" *is* "after commit".
pub const WEBHOOK_DELIVERIES_TOTAL: &str = "vpay_webhook_deliveries_total";

/// Classified errors, by ADR-0011 [`Category`](crate::error::Category),
/// [`Classify::code`] and [`Severity`].
///
/// Emitted by [`record_error_event`] from the three places that log a
/// *classified* error at its own severity.
pub const ERROR_EVENTS_TOTAL: &str = "vpay_error_events_total";

/// The subset of [`ERROR_EVENTS_TOTAL`] that woke someone up —
/// [`Severity::Page`] only.
///
/// A separate counter rather than a query over the one above, because the
/// alert rule that pages on it must not depend on a label selector staying
/// correct: `vpay_alert_events_total` increasing at all is the condition.
/// Same seam — literally the same function call — so the two cannot disagree
/// about what a page is.
pub const ALERT_EVENTS_TOTAL: &str = "vpay_alert_events_total";

/// Account-holder lookups served by `GET /v1/account_holders`, by outcome.
///
/// See [`account_holder_outcome`] for the four values, and
/// `docs/flows/account-holder-lookup.md` for the route.
///
/// **One label, and it can only ever be one of four constants.** The number
/// looked up is a person's phone number and the answer is a person's name;
/// neither may become a label, because a Prometheus label is retained,
/// queryable and shipped wherever the scrape goes — which would turn the
/// metric into the name-harvesting record the route exists not to keep. The
/// merchant is not a label either, for the same reason it is not audited
/// today: see the flow doc's reserved decision.
///
/// Emitted by `vpay_api::v1::account_holders::retrieve`, once per request,
/// on every path including the refusals — a merchant asking a rail that
/// cannot answer is exactly what an operator wants the rate of.
pub const ACCOUNT_HOLDER_LOOKUPS_TOTAL: &str = "vpay_account_holder_lookups_total";

/// Every name in this module, in the order the module doc lists them.
///
/// Public so a binary's own test can assert its `/metrics` output against the
/// vocabulary rather than against a copy of it, which
/// `backends/apps/vpay-server/tests/cli.rs` does. A series this list does not
/// declare has no description, no runbook and no alert.
///
/// ```
/// use vpay_core::metrics::{ALL, BUILD_INFO, ERROR_EVENTS_TOTAL};
///
/// assert!(ALL.contains(&BUILD_INFO));
/// assert!(ALL.contains(&ERROR_EVENTS_TOTAL));
/// // Namespaced, and syntactically a Prometheus name: a scrape into a
/// // shared Prometheus cannot collide with another exporter's series.
/// for name in ALL {
///     assert!(name.starts_with("vpay_"), "{name}");
///     assert!(
///         name.bytes()
///             .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
///         "{name}"
///     );
/// }
/// ```
pub const ALL: &[&str] = &[
    BUILD_INFO,
    HTTP_REQUESTS_TOTAL,
    HTTP_REQUEST_DURATION_SECONDS,
    PROVIDER_REQUESTS_TOTAL,
    PROVIDER_REQUEST_DURATION_SECONDS,
    CHARGE_TRANSITIONS_TOTAL,
    JOBS_CLAIMED_TOTAL,
    JOBS_COMPLETED_TOTAL,
    JOBS_OLDEST_CLAIMABLE_AGE_SECONDS,
    WEBHOOK_DELIVERIES_TOTAL,
    ERROR_EVENTS_TOTAL,
    ALERT_EVENTS_TOTAL,
    ACCOUNT_HOLDER_LOOKUPS_TOTAL,
];

/// The `outcome` label on [`JOBS_COMPLETED_TOTAL`].
///
/// A closed vocabulary, spelled once, because an alerting rule matches these
/// exactly and a rule that matches nothing looks identical to a system with
/// no failures. **Four values, where the design document's list names three**
/// — [`LOST`](job_outcome::LOST) postdates it and is not folded into the
/// others, for the reason in
/// [docs/reference/vpay-core.md § four job outcomes, not three](../../../../docs/reference/vpay-core.md#four-job-outcomes-not-three).
///
/// ```
/// use vpay_core::metrics::job_outcome;
///
/// let all = [
///     job_outcome::TERMINAL,
///     job_outcome::RETRY,
///     job_outcome::DEAD_LETTER,
///     job_outcome::LOST,
/// ];
/// assert_eq!(all, ["terminal", "retry", "dead_letter", "lost"]);
/// ```
pub mod job_outcome {
    /// The queue is done with this job: it was `DELETE`d. Includes a job that
    /// failed terminally — a declined charge is finished business for the
    /// queue.
    pub const TERMINAL: &str = "terminal";
    /// Released with a future `run_at`; something will run it again.
    pub const RETRY: &str = "retry";
    /// Parked at `run_at = 'infinity'` for a human.
    pub const DEAD_LETTER: &str = "dead_letter";
    /// The lease moved on before this worker could settle: the guarded write
    /// matched no row. Not an error, and not silently one of the above.
    pub const LOST: &str = "lost";
}

/// The `outcome` label on [`WEBHOOK_DELIVERIES_TOTAL`].
///
/// A closed vocabulary, spelled once, for the same reason [`job_outcome`] is:
/// `docs/runbooks/webhook-delivery-failures.md` and any future
/// `PrometheusRule` over this series match these values literally.
///
/// ```
/// use vpay_core::metrics::webhook_outcome;
///
/// let all = [
///     webhook_outcome::SUCCEEDED,
///     webhook_outcome::RETRY,
///     webhook_outcome::EXHAUSTED,
/// ];
/// assert_eq!(all, ["succeeded", "retry", "exhausted"]);
/// ```
pub mod webhook_outcome {
    /// The receiver answered 2xx and `record_success`'s compare-and-swap
    /// matched — the delivery is done.
    pub const SUCCEEDED: &str = "succeeded";
    /// The receiver refused, or nothing came back, and the retry ladder still
    /// has a rung left.
    pub const RETRY: &str = "retry";
    /// The retry ladder is spent and the row is parked at
    /// `state = 'exhausted'` for a human
    /// (`docs/runbooks/webhook-delivery-failures.md`).
    pub const EXHAUSTED: &str = "exhausted";
}

/// The `operation` label on [`PROVIDER_REQUESTS_TOTAL`] and
/// [`PROVIDER_REQUEST_DURATION_SECONDS`]: the three
/// `vpay_provider::ProviderAdapter` methods that can reach a rail.
///
/// Spelled here beside [`job_outcome`] because a closed label vocabulary
/// belongs with the names it labels — `vpay-core` does not depend on
/// `vpay-provider`, so these are strings that happen to match three method
/// names, and `vpay_provider::measured`'s own test is what keeps them
/// matching. `parse_callback` is deliberately absent: it parses bytes and
/// touches no rail.
///
/// ```
/// use vpay_core::metrics::provider_operation;
///
/// let all = [
///     provider_operation::SUBMIT,
///     provider_operation::QUERY_STATUS,
///     provider_operation::REFUND,
///     provider_operation::ACCOUNT_HOLDER_NAME,
/// ];
/// assert_eq!(all, ["submit", "query_status", "refund", "account_holder_name"]);
/// ```
pub mod provider_operation {
    /// `ProviderAdapter::submit` — opening a charge on the rail.
    pub const SUBMIT: &str = "submit";
    /// `ProviderAdapter::query_status` — the authenticated status query, the
    /// only call that moves money in vpay's model.
    pub const QUERY_STATUS: &str = "query_status";
    /// `ProviderAdapter::refund`.
    pub const REFUND: &str = "refund";
    /// `ProviderAdapter::account_holder_name` — the stateless identity read
    /// (issue #47). It moves no money and touches no charge, but it is a
    /// call to the rail over the same credential, so it belongs in the same
    /// series: a rail that has started refusing our subscription key refuses
    /// this too, and an operator should see one rate, not two.
    pub const ACCOUNT_HOLDER_NAME: &str = "account_holder_name";
}

/// The `outcome` label on [`ACCOUNT_HOLDER_LOOKUPS_TOTAL`].
///
/// A closed vocabulary, spelled once, for [`job_outcome`]'s reason. The four
/// values are the four things `GET /v1/account_holders` can answer, and they
/// are deliberately *not* derivable from the HTTP status: the first two are
/// both `200`.
///
/// ```
/// use vpay_core::metrics::account_holder_outcome;
///
/// let all = [
///     account_holder_outcome::FOUND,
///     account_holder_outcome::NOT_FOUND,
///     account_holder_outcome::UNSUPPORTED,
///     account_holder_outcome::ERROR,
/// ];
/// assert_eq!(all, ["found", "not_found", "unsupported", "error"]);
/// ```
pub mod account_holder_outcome {
    /// The rail named a holder. `200` with a `name` and `verified: true`.
    pub const FOUND: &str = "found";
    /// The rail has no record of the number. `200` with a null `name` and
    /// `verified: false` — **not** an error, and the distinction the route
    /// exists to preserve.
    pub const NOT_FOUND: &str = "not_found";
    /// The merchant named a `payment_method_type` whose rail has no
    /// account-holder API. A `400` naming the parameter, decided on the
    /// capability value and never on the rail's code (ADR-0002).
    pub const UNSUPPORTED: &str = "unsupported";
    /// Everything else: the rail could not be reached, refused our
    /// credentials, answered something unreadable, or the deployment is
    /// misconfigured. A classified 4xx/5xx, never a `200` with nulls.
    pub const ERROR: &str = "error";
}

/// This build's commit, or [`UNKNOWN_GIT_SHA`].
///
/// `option_env!` reads the environment rustc was invoked with, and
/// `vpay-core/build.rs` exists for the one line that puts `VPAY_GIT_SHA` in
/// cargo's fingerprint. It never shells out to `git`: the Docker build has no
/// `.git` directory, and a `rev-parse` that succeeded against whatever tree
/// the build machine stood in would be a sha that describes nothing. See
/// [docs/reference/vpay-core.md § the git sha label](../../../../docs/reference/vpay-core.md#the-git-sha-label).
///
/// ```
/// use vpay_core::metrics::{UNKNOWN_GIT_SHA, git_sha};
///
/// // Never empty: an empty label reads as "the build knew" on a dashboard.
/// assert!(!git_sha().is_empty());
/// // `unknown` on any build that was not told a commit — which is every
/// // local `cargo build` and every `just demo`.
/// assert!(git_sha() == UNKNOWN_GIT_SHA || !git_sha().is_empty());
/// ```
#[must_use]
pub fn git_sha() -> &'static str {
    option_env!("VPAY_GIT_SHA").unwrap_or(UNKNOWN_GIT_SHA)
}

/// What [`git_sha`] answers when the build was told no commit.
pub const UNKNOWN_GIT_SHA: &str = "unknown";

/// Stamps [`BUILD_INFO`] at `1` with this build's version and [`git_sha`].
///
/// `version` is the caller's own `env!("CARGO_PKG_VERSION")` rather than this
/// crate's: the interesting number is the *binary's*, so it is passed in
/// rather than assumed.
///
/// Call once, immediately after installing a recorder. A no-op without one.
pub fn record_build_info(version: &'static str) {
    gauge!(BUILD_INFO, "version" => version, "git_sha" => git_sha()).set(1.0);
}

/// Counts one classified error: [`ERROR_EVENTS_TOTAL`] always, and
/// [`ALERT_EVENTS_TOTAL`] as well when its severity is [`Severity::Page`].
///
/// One function rather than a macro call at each logging site, so the two
/// counters cannot disagree about what a page is and neither can disagree
/// with the `alert = true` field an alerting rule reads out of the JSON logs.
///
/// Three call sites, each the point where an error is logged at its own
/// classification. Four *other* log lines carry `alert = true` and do **not**
/// increment these counters, so `increase(vpay_alert_events_total)` is a
/// subset of "log lines with `alert = true`" rather than the whole of it —
/// which is a real gap, recorded rather than papered over, in
/// [docs/reference/vpay-core.md § the alert-events gap](../../../../docs/reference/vpay-core.md#the-alert-events-gap).
pub fn record_error_event<E: Classify + ?Sized>(error: &E) {
    let category = error.category().as_metric_label();
    let code = error.code();
    let severity = error.severity();
    counter!(
        ERROR_EVENTS_TOTAL,
        "category" => category,
        "code" => code,
        "severity" => severity.as_metric_label(),
    )
    .increment(1);
    if severity == Severity::Page {
        counter!(ALERT_EVENTS_TOTAL, "category" => category, "code" => code).increment(1);
    }
}

/// Registers a `HELP` and `TYPE` line for every name in [`ALL`].
///
/// Call once per process, immediately after installing a recorder — the
/// `metrics` facade discards a description sent before one exists, silently,
/// so ordering here is not cosmetic. Idempotent, and a no-op with no recorder
/// installed.
pub fn describe_all() {
    describe_gauge!(
        BUILD_INFO,
        "Always 1. The labels are the payload: this build's version and git sha."
    );
    describe_counter!(
        HTTP_REQUESTS_TOTAL,
        Unit::Count,
        "HTTP responses served, by route pattern, method and status code."
    );
    describe_histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        Unit::Seconds,
        "Wall-clock seconds to serve one HTTP request, by route pattern, method and status code."
    );
    describe_counter!(
        PROVIDER_REQUESTS_TOTAL,
        Unit::Count,
        "Calls to a payment rail, by provider code and port operation. `error_kind` is empty on \
         success and otherwise the error's Classify::code."
    );
    describe_histogram!(
        PROVIDER_REQUEST_DURATION_SECONDS,
        Unit::Seconds,
        "Wall-clock seconds for one call to a payment rail, by provider code and port operation."
    );
    describe_counter!(
        CHARGE_TRANSITIONS_TOTAL,
        Unit::Count,
        "Charge state transitions, by rail and by the (from, to) pair of states."
    );
    describe_counter!(
        JOBS_CLAIMED_TOTAL,
        Unit::Count,
        "Jobs claimed off the queue by this worker, by job kind."
    );
    describe_counter!(
        JOBS_COMPLETED_TOTAL,
        Unit::Count,
        "Jobs settled by this worker, by job kind and by what ended the lease: terminal, retry, \
         dead_letter or lost."
    );
    describe_gauge!(
        JOBS_OLDEST_CLAIMABLE_AGE_SECONDS,
        Unit::Seconds,
        "Seconds since (positive) or until (negative) the next queued job was due: now minus \
         min(run_at) over every unleased, unparked row. Zero when the queue is empty; not \
         written at all when it could not be read. Negative is normal on an idle deployment."
    );
    describe_counter!(
        WEBHOOK_DELIVERIES_TOTAL,
        Unit::Count,
        "Webhook delivery attempts, by outcome: succeeded, retry or exhausted."
    );
    describe_counter!(
        ERROR_EVENTS_TOTAL,
        Unit::Count,
        "Classified errors, by ADR-0011 category, code and severity."
    );
    describe_counter!(
        ALERT_EVENTS_TOTAL,
        Unit::Count,
        "Classified errors at Severity::Page — the ones that wake someone up."
    );
    describe_counter!(
        ACCOUNT_HOLDER_LOOKUPS_TOTAL,
        Unit::Count,
        "Account-holder lookups served by GET /v1/account_holders, by outcome: found, \
         not_found, unsupported or error. No label carries the number looked up or the name \
         returned."
    );
}

#[cfg(test)]
mod tests {
    use metrics_exporter_prometheus::PrometheusBuilder;

    use super::{
        ALL, UNKNOWN_GIT_SHA, describe_all, git_sha, job_outcome, record_build_info,
        record_error_event, webhook_outcome,
    };
    use crate::error::{Category, Classify};

    /// This file's own source, so the test below reads the module doc rather
    /// than a copy of it. `include_str!` is relative to *this* file.
    const SOURCE: &str = include_str!("metrics.rs");

    /// The metric names in the module doc's ```` ```text ```` block, in
    /// order.
    ///
    /// Parsing the doc rather than restating it is the whole point: a test
    /// holding its own copy of the list would agree with itself forever
    /// while the documentation drifted.
    fn names_in_the_module_doc() -> Vec<String> {
        let mut names = Vec::new();
        let mut inside = false;
        for line in SOURCE.lines() {
            let Some(body) = line.strip_prefix("//!") else {
                // The doc comment has ended; anything after it is code.
                if inside {
                    break;
                }
                continue;
            };
            let body = body.trim();
            if body == "```text" {
                inside = true;
                continue;
            }
            if inside {
                if body == "```" {
                    break;
                }
                // `vpay_jobs_completed_total{kind,outcome}   counter` — the
                // name is everything up to the labels or the whitespace.
                let name = body
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .split('{')
                    .next()
                    .unwrap_or_default();
                if !name.is_empty() {
                    names.push(name.to_owned());
                }
            }
        }
        names
    }

    /// The decisive test this module exists to make possible: the
    /// specification in the module doc and the `ALL` constant are the same
    /// list, in the same order.
    ///
    /// Deleting a line from the doc block, or a name from `ALL`, fails here.
    #[test]
    fn the_module_doc_list_and_the_all_constant_agree() {
        let documented = names_in_the_module_doc();
        assert!(
            !documented.is_empty(),
            "the module doc's ```text block was not found — did the doc comment move?"
        );
        let declared: Vec<String> = ALL.iter().map(|n| (*n).to_owned()).collect();
        assert_eq!(
            documented, declared,
            "the module doc's list and vpay_core::metrics::ALL have drifted"
        );
    }

    /// Every name is a valid Prometheus metric name and carries the `vpay_`
    /// prefix, so a scrape into a shared Prometheus cannot collide with
    /// another exporter's series.
    #[test]
    fn every_name_is_prefixed_and_syntactically_a_prometheus_name() {
        for name in ALL {
            assert!(
                name.starts_with("vpay_"),
                "{name} is not namespaced to this deployment"
            );
            assert!(
                name.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{name} is not [a-z0-9_]+, which Prometheus requires"
            );
        }
    }

    /// No duplicates: two `const`s with the same string would silently merge
    /// two different measurements into one series.
    #[test]
    fn the_names_are_distinct() {
        let mut sorted: Vec<&str> = ALL.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "a metric name is declared twice");
    }

    /// The `outcome` label values are distinct and lower-case, because an
    /// alert rule matches them literally.
    #[test]
    fn the_job_outcome_labels_are_distinct_and_lower_case() {
        let all = [
            job_outcome::TERMINAL,
            job_outcome::RETRY,
            job_outcome::DEAD_LETTER,
            job_outcome::LOST,
        ];
        for value in all {
            assert_eq!(value, value.to_lowercase(), "{value} is not lower-case");
        }
        let mut sorted = all;
        sorted.sort_unstable();
        let before = sorted.len();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(
            before,
            deduped.len(),
            "a job outcome label is declared twice"
        );
    }

    /// The same proof as `the_job_outcome_labels_are_distinct_and_lower_case`,
    /// for [`webhook_outcome`]'s closed vocabulary.
    #[test]
    fn the_webhook_outcome_labels_are_distinct_and_lower_case() {
        let all = [
            webhook_outcome::SUCCEEDED,
            webhook_outcome::RETRY,
            webhook_outcome::EXHAUSTED,
        ];
        for value in all {
            assert_eq!(value, value.to_lowercase(), "{value} is not lower-case");
        }
        let mut sorted = all;
        sorted.sort_unstable();
        let before = sorted.len();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(
            before,
            deduped.len(),
            "a webhook outcome label is declared twice"
        );
    }

    /// An error whose category the test chooses, so the label assertions
    /// below cover a paging severity and a non-paging one without depending
    /// on any particular leaf error's classification staying put.
    #[derive(Debug, thiserror::Error)]
    #[error("a classified failure")]
    struct Classified(Category);

    impl Classify for Classified {
        fn category(&self) -> Category {
            self.0
        }
    }

    /// Renders a real Prometheus scrape of whatever `body` recorded.
    ///
    /// The **shipping** exporter, not a debugging recorder: these
    /// assertions are about the text a scrape returns, so a mis-spelled
    /// label fails here for the same reason a dashboard would be empty. A
    /// *local* recorder rather than the global one, because
    /// `metrics::set_global_recorder` succeeds once per process and a unit
    /// test that used it would make every other test in this binary depend
    /// on the order it ran in.
    fn scrape_of(body: impl FnOnce()) -> String {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, body);
        handle.render()
    }

    /// The mechanism behind `vpay_build_info{git_sha}`, asserted at the one
    /// place a test can: `git_sha()` is `option_env!` at *this crate's*
    /// compile time, so a test cannot set the variable and observe a
    /// change — that would need a rebuild, which is what
    /// `vpay-core/build.rs` exists to trigger.
    ///
    /// What is checked here is that the fallback is the documented word and
    /// that the value is never empty, because an empty label reads as "the
    /// build knew" on a dashboard. The end-to-end half is manual and
    /// recorded in `docs/status.md`:
    ///
    /// ```text
    /// VPAY_GIT_SHA=deadbeef cargo build -p vpay-server
    /// ./target/debug/vpay-server … & curl -s localhost:9090/metrics | grep build_info
    /// # vpay_build_info{version="0.1.0",git_sha="deadbeef"} 1
    /// ```
    #[test]
    fn the_git_sha_label_is_the_build_time_value_or_the_documented_fallback() {
        let sha = git_sha();
        assert!(!sha.is_empty(), "an empty git_sha reads as a real answer");
        assert_eq!(
            sha,
            option_env!("VPAY_GIT_SHA").unwrap_or(UNKNOWN_GIT_SHA),
            "git_sha() must be exactly the build-time variable, with no runtime fallback"
        );
    }

    /// `vpay_build_info` is an info metric: the value is always 1 and the
    /// labels are the payload.
    #[test]
    fn build_info_is_one_and_carries_both_labels() {
        let scrape = scrape_of(|| record_build_info("9.9.9"));
        assert!(
            scrape.contains(&format!(
                r#"vpay_build_info{{version="9.9.9",git_sha="{}"}} 1"#,
                git_sha()
            )),
            "{scrape}"
        );
    }

    /// The decisive test for the pair of error counters: a `Severity::Page`
    /// error increments **both**, with the same category and code on each.
    ///
    /// `Category::Internal` is the one whose default severity is `Page`
    /// (ADR-0011's policy table), so this also pins that the severity used
    /// is the error's own rather than a value passed in beside it.
    #[test]
    fn a_page_severity_error_increments_both_counters() {
        let scrape = scrape_of(|| record_error_event(&Classified(Category::Internal)));

        assert!(
            scrape.contains(
                r#"vpay_error_events_total{category="Internal",code="internal_error",severity="Page"} 1"#
            ),
            "{scrape}"
        );
        assert!(
            scrape.contains(
                r#"vpay_alert_events_total{category="Internal",code="internal_error"} 1"#
            ),
            "a Page must reach the counter an alert rule fires on: {scrape}"
        );
    }

    /// ...and anything below `Page` increments only the first, or
    /// `VpayPageableErrorEvents` would page on every merchant typo.
    #[test]
    fn an_error_below_page_severity_touches_no_alert_counter() {
        let scrape = scrape_of(|| record_error_event(&Classified(Category::InvalidRequest)));

        assert!(
            scrape.contains(
                r#"vpay_error_events_total{category="InvalidRequest",code="invalid_request",severity="Info"} 1"#
            ),
            "{scrape}"
        );
        assert!(
            !scrape.contains("vpay_alert_events_total"),
            "a merchant's malformed request must never page: {scrape}"
        );
    }

    /// `describe_all` with no recorder installed is a no-op, not a panic.
    ///
    /// Worth pinning because it is exactly what happens in every test binary
    /// in this workspace that links `vpay-core` and installs nothing — and
    /// because the failure mode of the alternative (a library installing a
    /// recorder so its describes "work") is the one this module's header
    /// rules out.
    #[test]
    fn describing_without_a_recorder_is_harmless() {
        describe_all();
    }
}
