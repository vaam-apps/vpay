//! Every metric name vpay emits, and their descriptions — the names only,
//! never a recorder.
//!
//! # Why a library describes but does not install
//!
//! Installing a process-wide recorder is an *application* decision, the same
//! one `install_crypto_provider()` is in both `main.rs` files: a library that
//! calls `metrics::set_global_recorder` takes it out of the binary's hands
//! and makes two linked libraries a startup panic. So this module owns the
//! vocabulary — the names, their units, their help text — and each binary
//! owns the exporter it renders them through. [`describe_all`] is the one
//! call that connects the two, and it is a no-op until a recorder exists,
//! which is why every caller runs it immediately *after* installing one.
//!
//! # Why the names are `const`s and not string literals at call sites
//!
//! A typo in a metric name is invisible: nothing fails, a dashboard is
//! simply empty, and the gap is discovered during the incident the dashboard
//! existed for. A `const` makes the typo a compile error. The same reasoning
//! applies to [`job_outcome`]'s and [`webhook_outcome`]'s label *values*,
//! which are closed vocabularies an alerting rule matches on exactly.
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
//! ```
//!
//! # Where each of these is emitted
//!
//! Every one of the twelve has exactly one seam, named on each constant
//! below. [`WEBHOOK_DELIVERIES_TOTAL`] was the last to gain one: it is
//! recorded by `vpay_worker::webhooks` after a delivery attempt's outcome
//! has been committed. A described-but-unrecorded metric never appears in a
//! scrape (the Prometheus exporter renders registered *handles*, not
//! descriptions), which is why a name can be declared here before its seam
//! exists without making a deployment look more instrumented than it is.

use metrics::{Unit, counter, describe_counter, describe_gauge, describe_histogram, gauge};

use crate::error::{Classify, Severity};

/// `1`, labelled with this build's version and git sha.
///
/// The Prometheus "info metric" idiom: the value carries nothing, the labels
/// are the payload, so `vpay_build_info` joined against any other series
/// answers "which build produced this". Set once per process at startup.
///
/// Emitted by [`record_build_info`], called from each binary's
/// `install_recorder`.
pub const BUILD_INFO: &str = "vpay_build_info";

/// HTTP responses served, by route pattern, method and status.
///
/// `route` is the axum path *pattern* (`/v1/payment_intents/{id}`), never
/// the concrete path: a label whose cardinality grows with the number of
/// payment intents would eventually be the largest thing in the metrics
/// store. A request that matched no route carries `route="unmatched"`,
/// which is a bounded label for an unbounded set of paths.
///
/// `method` is bounded the same way: the ten methods `http::Method` names (`QUERY` included)
/// verbatim, anything else `method="other"` (`vpay_api`'s `OTHER_METHOD`).
/// A method is a free-form token in RFC 9110 §9.1 and `http::Method` parses
/// an unknown one rather than rejecting it, so an unauthenticated caller
/// sending `M12345` would otherwise mint a series per request — the route
/// label's hole, on the next label along.
///
/// Emitted by `vpay_api`'s `track_http_metrics` middleware — one function,
/// mounted in `vpay_api::router` — and by nothing else. The observability
/// listener's own `/livez` and `/metrics` are **not** counted: they are a
/// different router on a different port, and a scraper polling every 15s
/// would otherwise be the largest traffic source on the series.
pub const HTTP_REQUESTS_TOTAL: &str = "vpay_http_requests_total";

/// Wall-clock seconds per HTTP response, same labels and same seam as
/// [`HTTP_REQUESTS_TOTAL`].
///
/// Measured around the *inner* service, so it excludes the request-id and
/// trace layers above it and includes routing, authentication, the handler
/// and the error renderer. That is the span an operator can act on.
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "vpay_http_request_duration_seconds";

/// Rail calls, by provider code, port operation and failure kind.
///
/// `error_kind` is `""` on success and otherwise [`Classify::code`] — the
/// same vocabulary the `provider_requests.error_kind` column stores, so a
/// Prometheus alert and the SQL an operator runs against that table use one
/// set of words.
///
/// Emitted by `vpay_provider::Measured`, the port decorator every adapter is
/// wrapped in by `vpay_api::v1::boot::adapters_by_code`. That is one seam
/// for both rails and for every rail added later, and it is *outside* the
/// adapter crates, so instrumenting a rail is not a provider-code branch
/// (ADR-0002).
///
/// **It counts port calls, not wire requests, and the difference is worth
/// knowing.** One `submit` on Orange Money mints an access token and then
/// posts the payment — two HTTP requests, one increment. A `submit` refused
/// before the socket is opened (a missing credential, a payer-less push
/// charge) is also one increment, with that refusal's `error_kind`. The
/// question this metric answers is "how are calls to this rail going",
/// which is a port-level question; `provider_requests` in Postgres is the
/// per-attempt record.
pub const PROVIDER_REQUESTS_TOTAL: &str = "vpay_provider_requests_total";

/// Wall-clock seconds per rail call. Deliberately not labelled by
/// `error_kind`: a latency histogram split by every failure mode is mostly
/// empty buckets, and the question it answers ("is this rail slow") does not
/// depend on why a call failed. Same seam as [`PROVIDER_REQUESTS_TOTAL`],
/// and the same port-call-not-wire-request caveat.
pub const PROVIDER_REQUEST_DURATION_SECONDS: &str = "vpay_provider_request_duration_seconds";

/// Charge state transitions, by rail and by the pair of states.
///
/// The `from`/`to` pair rather than a single `state` label because the
/// interesting quantity is the *edge* — "how many charges went
/// `submitted` → `failed`" is an alertable rate; "how many are `failed`" is
/// a database query.
///
/// Emitted through `vpay_db::charges::record_transition`, for the six
/// statements that can move `charges.state` and for nothing else — three
/// in `vpay_db::charges`, three in `vpay_db::settlement`. The database
/// layer rather than the worker's settlement points because *every*
/// transition passes through these functions and only some of them pass
/// through the worker: a confirm opens and submits a charge inside
/// `vpay-api`, and a metric mounted on the worker would silently miss it.
///
/// **Counted after the transition commits, never inside the transaction
/// that made it.** `vpay_db::settlement`'s three own their transaction and
/// record after their own `COMMIT`; `vpay_db::charges`' three run inside a
/// *caller's* transaction, so they return their row and the caller records
/// after `tx.commit()` (`charges::record_opened`,
/// `charges::record_left_submitting`). Nothing inside a transaction can
/// know whether it will be committed, and a counter claiming a charge that
/// a `ROLLBACK` erased is worse than one that is a moment late.
///
/// Every label is read back off the row the database returned, never off
/// the caller's copy, so a transition that did not actually fire — a
/// compare-and-swap that matched nothing — cannot be counted. The one
/// caveat is on `from` alone, and it is on
/// `vpay_db::settlement::apply_succeeded`/`apply_failed`: those two read
/// the previous state through a sub-select in `RETURNING`, which sees the
/// statement's snapshot, so under a concurrent live-state move the label
/// can name the rung the charge was on a moment earlier. `to` and
/// `provider` are exact in every case.
pub const CHARGE_TRANSITIONS_TOTAL: &str = "vpay_charge_transitions_total";

/// Jobs claimed off the queue by this worker, by `jobs.kind`.
///
/// Emitted by `vpay_worker::run_once`, on the arm where a claim returned a
/// row.
pub const JOBS_CLAIMED_TOTAL: &str = "vpay_jobs_claimed_total";

/// Jobs settled by this worker, by `jobs.kind` and by what ended the lease.
///
/// See [`job_outcome`] for the label values and for the one that the design
/// document's list does not mention.
pub const JOBS_COMPLETED_TOTAL: &str = "vpay_jobs_completed_total";

/// How far behind the queue is: `now - min(run_at)` over claimable rows, in
/// seconds.
///
/// A property of the *table*, so every replica reports the same value and a
/// deployment can alert on it without summing across pods.
///
/// **Zero when the queue holds nothing runnable**, which is deliberately
/// *not* what the worker's `job loop gauge` log line does — that leaves the
/// field null, because "nothing to do" and "caught up to the second" are
/// different facts. A Prometheus gauge has no null: it holds its last value
/// until something writes another one. Leaving it unwritten on an empty
/// queue would mean the value from the last backlog stays on the series
/// forever, and an alert thresholded on it would page indefinitely after the
/// backlog cleared. Zero is the lesser inaccuracy, and it is the one that
/// cannot invent an incident.
///
/// It is left *unwritten* when the read itself failed, because then the
/// answer is genuinely unknown; the worker logs a warning in that case.
///
/// # The name says `claimable`; the query is not quite that, and it goes
/// **negative**
///
/// `vpay_db::jobs::oldest_runnable_run_at` is
/// `SELECT min(run_at) FROM jobs WHERE locked_at IS NULL AND run_at <
/// 'infinity'` — every unleased, unparked row, *including ones scheduled in
/// the future*. So on a healthy idle deployment, whose only queued work is
/// the hourly `sweep_expired`, this reads about `-3500`: the next job is
/// nearly an hour away. Observed directly on `just demo`
/// (`vpay_jobs_oldest_claimable_age_seconds -540.01`).
///
/// The name is transcribed verbatim from
/// `docs/plans/2026-09-03-step6-deployment.md` §3 and is not changed here,
/// but "age of the oldest claimable row" is the wrong reading of it. The
/// right one is **"seconds until (negative) or since (positive) the next
/// piece of queued work was due"**, which is the same quantity the worker's
/// `queue_behind_seconds` log field has carried since Step 4. A
/// `> 300`-style alert is unaffected — it is the positive tail that means a
/// backlog — but a dashboard that renders this as an "age" will show
/// negative bars on a perfectly healthy queue, and a `min()`/`abs()`
/// applied to make that look tidier would hide exactly the case the metric
/// exists for.
pub const JOBS_OLDEST_CLAIMABLE_AGE_SECONDS: &str = "vpay_jobs_oldest_claimable_age_seconds";

/// Webhook delivery attempts, by outcome.
///
/// Emitted from `vpay_worker::webhooks::handle_deliver` at the two points a
/// delivery attempt's outcome becomes durable: right after
/// `vpay_db::webhook_deliveries::record_success` commits
/// (`webhook_outcome::SUCCEEDED`, only when that compare-and-swap actually
/// changed the row — a second pass over an already-`succeeded` delivery
/// counts nothing, matching the log line beside it), and right after
/// `record_failure`'s call to
/// `vpay_db::webhook_deliveries::record_attempt` commits, where the ladder
/// index already computed decides `webhook_outcome::RETRY` or
/// `webhook_outcome::EXHAUSTED`. Both writes are single `UPDATE` statements
/// against the pool with no explicit transaction, so "after `.await?`
/// returns `Ok`" *is* "after commit" — Postgres autocommits one statement.
/// See [`webhook_outcome`] for the label vocabulary.
pub const WEBHOOK_DELIVERIES_TOTAL: &str = "vpay_webhook_deliveries_total";

/// Classified errors, by ADR-0011 [`Category`](crate::error::Category),
/// [`Classify::code`] and
/// [`Severity`].
///
/// Emitted by [`record_error_event`], which is called from the three places
/// that log a *classified* error at its own severity: `vpay_api::ApiError`'s
/// `log`, `vpay_worker::handlers`' `log_failure`, and the job loop's
/// queue-unreachable arm. See that function for the `alert = true` log
/// lines this counter deliberately does **not** carry.
pub const ERROR_EVENTS_TOTAL: &str = "vpay_error_events_total";

/// The subset of [`ERROR_EVENTS_TOTAL`] that woke someone up —
/// [`Severity::Page`] only.
///
/// A separate counter rather than a query over the one above, because the
/// alert rule that pages on it must not depend on a label selector staying
/// correct: `vpay_alert_events_total` increasing at all is the condition.
///
/// Same seam as [`ERROR_EVENTS_TOTAL`] — literally the same function call,
/// so the two cannot disagree about what a page is, and neither can
/// disagree with the `alert = true` field, which is set in the same `match`
/// arm.
pub const ALERT_EVENTS_TOTAL: &str = "vpay_alert_events_total";

/// Every name in this module, in the order the module doc lists them.
///
/// Public so a binary's own test can assert its `/metrics` output against
/// the vocabulary rather than against a copy of it — which
/// `the_observability_listener_serves_livez_and_metrics_on_its_own_port_only`
/// in `backends/apps/vpay-server/tests/cli.rs` does: it folds the histogram
/// suffixes off every sample line the running server renders and fails if a
/// family is not named here. A series this list does not declare has no
/// description, no runbook and no alert.
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
];

/// The `outcome` label on [`JOBS_COMPLETED_TOTAL`].
///
/// A closed vocabulary, spelled once, because an alerting rule matches these
/// exactly and a rule that matches nothing looks identical to a system with
/// no failures.
///
/// **Four values, where the design document's list names three.**
/// `docs/plans/2026-09-03-step6-deployment.md` §3 writes
/// `terminal|retry|dead_letter`; that list was written before Step 4 landed
/// `vpay_worker::Disposition::Lost` — the case where a worker's lease was
/// reaped mid-job and its answer thrown away. (Named in prose and not
/// linked: `vpay-core` does not depend on `vpay-worker`, and it must not
/// start doing so for a doc link.) Folding `lost` into any of the other
/// three would make a real defect
/// — a lease shorter than a handler — invisible, so it gets its own value
/// and this note rather than a quiet reconciliation.
pub mod job_outcome {
    /// The queue is done with this job: it was `DELETE`d. Includes a job
    /// that failed terminally — a declined charge is finished business for
    /// the queue.
    pub const TERMINAL: &str = "terminal";
    /// Released with a future `run_at`; something will run it again.
    pub const RETRY: &str = "retry";
    /// Parked at `run_at = 'infinity'` for a human.
    pub const DEAD_LETTER: &str = "dead_letter";
    /// The lease moved on before this worker could settle: the guarded write
    /// matched no row. Not an error, and not silently one of the above —
    /// see this module's own doc comment.
    pub const LOST: &str = "lost";
}

/// The `outcome` label on [`WEBHOOK_DELIVERIES_TOTAL`].
///
/// A closed vocabulary, spelled once, for the same reason [`job_outcome`]
/// is: `docs/runbooks/webhook-delivery-failures.md` and any future
/// `PrometheusRule` over this series match these values literally.
pub mod webhook_outcome {
    /// The receiver answered 2xx and `record_success`'s compare-and-swap
    /// matched — the delivery is done.
    pub const SUCCEEDED: &str = "succeeded";
    /// The receiver refused, or nothing came back, and the retry ladder
    /// still has a rung left: `record_failure` computed a `next_attempt_at`.
    pub const RETRY: &str = "retry";
    /// The retry ladder is spent — `vpay_worker::delivery_delay` returned
    /// `None` — and the row is parked at `state = 'exhausted'` for a human
    /// (`docs/runbooks/webhook-delivery-failures.md`). Named in prose and
    /// not linked: `vpay-core` does not depend on `vpay-worker`.
    pub const EXHAUSTED: &str = "exhausted";
}

/// The `operation` label on [`PROVIDER_REQUESTS_TOTAL`] and
/// [`PROVIDER_REQUEST_DURATION_SECONDS`]: the three
/// `vpay_provider::ProviderAdapter` methods that can reach a rail.
///
/// Spelled here beside [`job_outcome`] and for the same reason — a closed
/// label vocabulary belongs with the names it labels, not at the call site
/// — even though the trait itself lives in `vpay-provider`. `vpay-core`
/// does not depend on `vpay-provider`, so this is a set of strings that
/// happens to match three method names; `vpay_provider::measured`'s own
/// test is what keeps them matching.
///
/// `parse_callback` is deliberately absent: it parses bytes and touches no
/// rail, so counting it would put a pure function in a metric an operator
/// reads as "calls to the rail".
pub mod provider_operation {
    /// `ProviderAdapter::submit` — opening a charge on the rail.
    pub const SUBMIT: &str = "submit";
    /// `ProviderAdapter::query_status` — the authenticated status query,
    /// the only call that moves money in vpay's model.
    pub const QUERY_STATUS: &str = "query_status";
    /// `ProviderAdapter::refund`.
    pub const REFUND: &str = "refund";
}

/// This build's commit, or `"unknown"`.
///
/// # Why `option_env!` and a `build.rs` that sets no variable
///
/// `option_env!` reads the environment **rustc was invoked with**, so
/// `VPAY_GIT_SHA=<sha> cargo build` bakes the value in with no code
/// generation at all. What that alone does not do is *rebuild*: cargo's
/// fingerprint for this crate does not include an environment variable it
/// has never been told about, so changing the sha and rebuilding would
/// silently keep the old label. `build.rs` exists for exactly one line —
/// `cargo::rerun-if-env-changed=VPAY_GIT_SHA` — which puts the variable in
/// the fingerprint. That is the whole mechanism, and it is why the build
/// script emits no `rustc-env` of its own: a value passed through twice can
/// disagree with itself.
///
/// # Why it never shells out to `git`
///
/// `backends/Dockerfile` builds in a context with no `.git` directory (the
/// image is `FROM scratch`; the build context is a `COPY` of source trees),
/// and a scratch or vendored build has no repository either. A `git
/// rev-parse` there either fails or — worse — succeeds against whatever
/// tree the build machine happens to be standing in, which is a sha that
/// describes nothing. `"unknown"` is the honest answer to "which commit is
/// this" when nobody told the build.
///
/// The label is therefore `unknown` on every local `cargo build` and every
/// `just demo`, and carries a real sha only where something passes one:
/// `backends/Dockerfile`'s `ARG VPAY_GIT_SHA` and `release.yml`'s
/// `build-args: VPAY_GIT_SHA=${{ github.sha }}`.
#[must_use]
pub fn git_sha() -> &'static str {
    option_env!("VPAY_GIT_SHA").unwrap_or(UNKNOWN_GIT_SHA)
}

/// What [`git_sha`] answers when the build was told no commit.
///
/// A named constant because a test asserts on it and because an operator
/// reading `git_sha="unknown"` on a dashboard should be able to grep for
/// the reason.
pub const UNKNOWN_GIT_SHA: &str = "unknown";

/// Stamps [`BUILD_INFO`] at `1` with this build's version and [`git_sha`].
///
/// `version` is the caller's own `env!("CARGO_PKG_VERSION")` rather than
/// this crate's: the two are the same today (one workspace version) and the
/// interesting number is the *binary's*, so it is passed in rather than
/// assumed.
///
/// Call once, immediately after installing a recorder. A no-op without one.
pub fn record_build_info(version: &'static str) {
    gauge!(BUILD_INFO, "version" => version, "git_sha" => git_sha()).set(1.0);
}

/// Counts one classified error: [`ERROR_EVENTS_TOTAL`] always, and
/// [`ALERT_EVENTS_TOTAL`] as well when its severity is
/// [`Severity::Page`].
///
/// # Why one function rather than a macro call at each logging site
///
/// The two counters must never disagree about what a page is, and neither
/// may disagree with the `alert = true` field an alerting rule reads out of
/// the JSON logs. Written twice, they would drift the first time someone
/// added a severity arm; written here, the only way to increment one is to
/// increment the other, and every caller passes the same error it is about
/// to log.
///
/// # Callers, and the `alert = true` lines that are deliberately not here
///
/// Three call sites, each the point where an error is logged *at its own
/// classification*: `vpay_api::ApiError::log`,
/// `vpay_worker::handlers::log_failure`, and the job loop's
/// "the job queue is not answering" arm.
///
/// Four other log lines in this workspace carry `alert = true` and do
/// **not** increment these counters, which is a real gap and is recorded
/// rather than papered over:
///
/// * `vpay_worker::run_loop::log_disposition` — it re-reports a failure
///   `log_failure` has already counted, and at a *wider* severity net, so
///   counting it would double some incidents and add others with no
///   classification to label them with;
/// * the seed-singletons, release-leases and settlement-contradiction
///   lines, which flag `alert = true` unconditionally and carry no
///   `Classify` value to derive `category`/`code` from.
///
/// So `increase(vpay_alert_events_total)` is a *subset* of "log lines with
/// `alert = true`", not the whole of it. Closing that gap means giving the
/// worker's ad-hoc alerts a classified error to carry, which is a change to
/// the worker's error model rather than to this function.
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
/// so ordering here is not cosmetic.
///
/// Idempotent: describing a name twice overwrites the description with the
/// same text.
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
