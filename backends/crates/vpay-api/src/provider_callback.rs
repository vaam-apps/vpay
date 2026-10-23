//! `POST /provider/{code}/callback` — the one route on this process a
//! **payment rail** calls, and the only unauthenticated route that is not
//! authenticated by *something*.
//!
//! STATUS: implemented since Step 8 (lane C). Before it, both adapters'
//! `parse_callback` existed and nothing in a running vpay called either, so
//! `X-Callback-Url`/`notif_url` were sent to a host that answered 404 and
//! settlement was polling-only. `docs/flows/reconciler.md` §"Callbacks are
//! hints" described this route as a design; this module is that design.
//!
//! # It never writes charge or intent state, and that is the whole design
//!
//! AGENTS.md: "Callbacks are hints. `parse_callback` returns identifiers
//! only, never a status. The authenticated status query is the only thing
//! that moves money." MTN signs nothing and sends no shared secret
//! (`docs/flows/adapter-mtn-momo.md` §"The callback is unsigned and
//! unauthenticated"), Orange's `notif_token` is not compared, and Notch
//! Pay's signature is checked only where a signing secret is configured
//! (below), so anyone who can reach this URL can post anything to it. What
//! this handler is therefore allowed to touch is exactly one row:
//! **the charge's `poll_charge` job**, which it enqueues if none is queued
//! and then brings forward to now — one transaction, two statements, the
//! enqueue `ON CONFLICT DO NOTHING` so an existing job is untouched by it —
//! so the worker asks the rail, over an authenticated status query,
//! immediately rather than at the poll ladder's next rung
//! (`vpay_worker::poll_delay(0)` is ten seconds).
//!
//! _(Corrected 2026-09-23. This paragraph said the handler could only "bring
//! an already-queued `poll_charge` job forward to now". It has also enqueued
//! the job when none was queued since it was mounted in Step 8 — step 4 of
//! `callback`'s own docs, a job an operator deleted or one already
//! finished. The enqueue is due at Postgres' own `now()` since the same day
//! (ADR-0026); it was this host's clock until then. The pull-forward always
//! was the database's.)_
//!
//! # The rail's own check (RFC-0014, built 2026-10-23)
//!
//! Between finding the charge and moving its poll, the route asks the rail's
//! adapter whether the callback is the rail's
//! ([`ProviderAdapter::verify_callback`]), handing it the headers, the raw
//! body and the rail's configuration. The verdict decides **one thing**:
//! whether the poll is pulled forward. A `Verified` callback moves no more
//! than an unchecked one did before; an `Invalid` one moves nothing, and the
//! poll ladder still asks the rail at its next rung. Notch Pay is the only
//! rail that checks (`x-notch-signature`, HMAC-SHA256), and only where
//! `credentials.callback_signing_secret` is configured; every other rail's
//! adapter answers `NotChecked`, which is the behaviour this route had
//! before the check existed.
//!
//! Each verdict is counted on `vpay_provider_callbacks_total{provider,
//! verdict}`. `Invalid` and `Unavailable` log at `warn` with the adapter's
//! fixed sentence; no header value, body byte or secret is ever logged.
//!
//! # What an anonymous caller can and cannot get out of it
//!
//! Three things bound what a POST here is worth, and the fourth paragraph is
//! the one that does not:
//!
//! * the `dedupe_key` is `poll:<charge id>`, and a unique index on it means
//!   a flood of callbacks about one charge is one row, forever
//!   (`docs/flows/reconciler.md`: "the `dedupe_key` is what stops duplicate
//!   callbacks becoming a job storm");
//! * a job that is **already due within [`PULL_FORWARD_FLOOR`]** — the poll
//!   ladder's fastest rung, ten seconds — is left where it is, as is one
//!   already leased or dead-lettered
//!   ([`vpay_db::TxRepositories::pull_forward_in_tx`]). A charge the queue
//!   was about to ask about anyway therefore costs a caller nothing at all:
//!   the request is two statements against `jobs` and no rail request;
//! * `CallbackRef::ref_extra` is **discarded**. Orange's `parse_callback`
//!   carries a `notif_token` and sometimes a `pay_token` out of the
//!   notification, and writing either onto the charge would be taking rail
//!   key material from an unauthenticated request — the one thing this
//!   route must not do. Repairing a lost `ref_extra` from a callback needs
//!   the stored `notif_token` compared against the received one first, and
//!   that is not built; `docs/status.md` says so.
//!
//! On a rail whose check runs (Notch Pay, with its secret configured), a
//! fourth bound holds: a POST that is not signed by the rail moves nothing at
//! all. The paragraph below is about every other rail, and about a genuine
//! callback replayed: Notch Pay's signature carries no timestamp, so a
//! captured one verifies again (RFC-0014 open question 4).
//!
//! **What is *not* bounded, stated plainly.** Until Step 8's review this
//! module claimed a hostile caller was "bounded by what the ladder was going
//! to do anyway". It is not. A POST naming a live charge that is parked
//! further out than the floor still causes one real, authenticated
//! `query_status` within about a second, and the ladder's rungs grow (20 s,
//! 30 s, 45 s, …) while the floor stays at ten, so a caller who repeats can
//! hold one charge at roughly one rail request per worker claim. The floor
//! removes the cheapest version of that — a charge sitting on the first rung
//! cannot be accelerated at all — and nothing else here is a rate limit:
//! **there is none**, and `docs/status.md` says so. What is left standing
//! between the route and rail traffic is that a caller must first know a v4
//! `provider_reference_id` belonging to a live charge *on this deployment*,
//! that the work is one status query which settles the charge the rail names
//! or nothing at all, and that the body is bounded at 16 KiB.
//!
//! # The answers, and why every 202 is indistinguishable
//!
//! | Case | Answer |
//! |---|---|
//! | `code` names no adapter this process links | `404`, byte-identical to the router's own fallback |
//! | the body is not a notification this rail could have sent | `400`, plus a `warn` carrying the adapter's own reason |
//! | the reference names no charge here | `202` |
//! | the reference names a charge, and the rail's check refuses it | `202`, plus a `warn` |
//! | the reference names a charge, and the check admits it | `202` |
//!
//! The last three are the same response on purpose: an unauthenticated
//! endpoint that answered differently for "this charge exists" would be an
//! oracle for exactly that question, which is the same argument
//! [`crate::browser`] makes about its uniform 404. That argument is enough
//! on its own. The unknown reference is logged at `info`, which is where an
//! operator debugging a misregistered callback host will find it. A refused
//! signature answers `202` for a second reason RFC-0014 § 3 gives: its
//! ordinary cause is this deployment's own stale secret, and a rail told
//! `401` would retry a callback vpay can never accept.
//!
//! _(This section was "The four answers, and why the two 202s are
//! indistinguishable" until 2026-10-23, when the check added the fourth
//! row.)_
//!
//! _(This gave two reasons until 2026-10-21. The other was that "a rail that
//! gets anything other than a 2xx retries — MTN and Orange both do, on their
//! own schedules", so a `404` would buy a retry loop. MTN's own callback
//! documentation (<https://momodeveloper.mtn.com/api-documentation/callback>,
//! read 2026-10-20) says the opposite: MTN sends a callback once, has no
//! retry mechanism, and recommends polling — which is what the poll ladder
//! already does whether or not a callback arrives. No official Orange Money
//! Web Payment source says either way; this repository's Orange knowledge is
//! a reconstruction from community SDKs (`docs/flows/adapter-orange-money.md`).
//! The oracle argument never depended on retries, so the answer did not
//! move. RFC-0013 § 11, item 1, is where this was found. Of the rails linked
//! here, only Notch Pay's reference mentions retrying: RFC-0013 § 12, item
//! 12, quotes it asking for a `200`, and whether Notch Pay treats this
//! route's `202` as delivered is open there.)_
//!
//! A `404` for an unknown *rail code* is a different thing and stays: it is
//! a statement about this deployment's route table, which is public — the
//! same information a merchant gets from `payment_method_types`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::routing::post;
use serde_json::json;
use vpay_core::metrics::PROVIDER_CALLBACKS_TOTAL;
use vpay_core::{Currency, Money};
use vpay_db::{ChargeRow, Repositories, TxOutcome, UnitOfWork as _};
use vpay_provider::{
    CallbackCheck, CallbackRequest, CallbackVerdict, ChargeRef, ProviderAdapter, RefExtra,
};

use crate::ResourceConfig;
use crate::error::ApiError;
use crate::v1::payment_intents::{POLL_CHARGE_KIND, poll_dedupe_key};

/// Where this nest is mounted on the outer router.
///
/// `/provider`, outside `/v1`, because it is not part of the merchant API:
/// no SDK calls it, it carries no version the way a merchant-facing resource
/// does, and putting it under `/v1` would place an unauthenticated route
/// inside the prefix whose entire boundary is "everything here needs a
/// bearer token". It is also the path already written down — the derived
/// `ProviderConfig::callback_url` is
/// `{deployment.public_base_url}/provider/{code}/callback`
/// (`vpay_config::ProviderHost::effective_callback_url`), which both
/// adapters have been sending since Step 3.
///
/// `pub` so a test outside this crate can reach the route without
/// re-spelling the prefix, and so the one place it is written is the one
/// place a reader has to look.
pub const PROVIDER_NEST: &str = "/provider";

/// The route inside [`PROVIDER_NEST`], as axum spells the pattern.
///
/// Also the `route` label every `vpay_http_requests_total` series for this
/// surface carries — the *pattern*, so a rail code nobody linked cannot mint
/// a time series (`crate::track_http_metrics`, `crate::UNMATCHED_ROUTE`).
pub const PROVIDER_CALLBACK_ROUTE: &str = "/{code}/callback";

/// The largest callback body this route will read, in bytes.
///
/// 16 KiB. Both rails' documented notifications are a few hundred bytes
/// (`docs/flows/adapter-mtn-momo.md`, `docs/flows/adapter-orange-money.md`),
/// so this is two orders of magnitude of headroom for a rail that adds
/// fields — and still small enough that an anonymous caller cannot make this
/// process buffer a megabyte per connection. Deliberately tighter than
/// `crate::V1_BODY_LIMIT_BYTES`'s 64 KiB: that bound sizes a merchant's
/// metadata map, and nothing here has one.
///
/// `RequestBodyLimitLayer` rather than axum's `DefaultBodyLimit` for the
/// reason the `/v1` nest gives: the former bounds the body itself, so it
/// applies whether or not an extractor ever reads it.
pub(crate) const CALLBACK_BODY_LIMIT_BYTES: usize = 16 * 1024;

/// How close to due a charge's poll job may already be and still be left
/// alone by a callback.
///
/// Ten seconds, which is the poll ladder's fastest rung —
/// `vpay_worker::poll_delay(0)`. A job due inside it is about to run, so
/// moving it buys the rail nothing measurable and costs an unauthenticated
/// caller nothing either: it is what turns "POST about a freshly confirmed
/// charge" from one rail request into zero.
///
/// **Written out here rather than read from `poll_delay`** because the
/// dependency runs the other way — `vpay-worker` links `vpay-api`, not the
/// reverse — so this crate cannot name that function without a cycle. The
/// join is asserted where both crates are linked:
/// `the_pull_forward_floor_is_the_poll_ladders_first_rung` in
/// `backends/tests/integration/tests/provider_callback.rs` fails if the
/// ladder's first rung ever moves away from this number.
///
/// `pub` for that test, and because it is the number an operator reading
/// `docs/reference/vpay-api.md` §"The rail callback route" is told about.
pub const PULL_FORWARD_FLOOR: std::time::Duration = std::time::Duration::from_secs(10);

/// The `/provider` router.
///
/// Carries its own `.fallback` for the reason `crate::router`'s docs record
/// about the OP and browser nests: without one, axum flattens this nest's
/// single route into the outer path table and registers no catch-all, so
/// `/provider/anything_else` would fall through to the outer router. Here
/// that happens to answer the same 404, which is precisely why the fallback
/// is written down rather than relied on — the outer fallback is one
/// `.nest()` away from being something else, and a rail POSTing to a
/// mistyped path should be answered by the router that owns the prefix.
///
/// **No `CorsLayer`.** The caller is a rail's own backend, not a browser;
/// there is no origin to allow and nothing here reads a cookie. `/v1/browser`
/// is the only nest that carries one, and
/// `cors_is_mounted_on_the_browser_nest_and_on_no_other` fails if that stops
/// being true.
pub(crate) fn routes() -> Router<crate::AppState> {
    Router::new()
        .route(PROVIDER_CALLBACK_ROUTE, post(callback))
        .fallback(crate::not_found)
}

/// Accepts one rail notification and pulls that charge's poll forward.
///
/// The module docs carry the reasoning; this is the order, and it is the
/// order the [`ApiError`] answers fall out of:
///
/// 1. resolve the adapter by `code`, or answer the router's own 404;
/// 2. `parse_callback` the body into identifiers, or answer `400`;
/// 3. find the charge under **this rail** and that reference, or answer
///    `202` and log;
/// 4. ask the rail's adapter whether the callback is the rail's
///    ([`verdict_for`]), count the verdict, and answer `202` without moving
///    anything if the verdict refuses (RFC-0014 § 3);
/// 5. in one transaction, **two statements**: enqueue the poll (a no-op when
///    it already exists) and pull it forward, unless it is already due
///    within [`PULL_FORWARD_FLOOR`].
///
/// _(Four steps until 2026-10-23; step 4 is RFC-0014's, and step 5 was
/// step 4.)_
///
/// `Bytes` rather than a typed body: the shape belongs to the rail, and this
/// crate must not learn it — `parse_callback` is where a rail's wire format
/// is allowed to be known (ADR-0002). It is also why the body is not
/// validated as JSON here: a rail that posts form-encoded notifications is a
/// rail whose adapter reads form bytes, and nothing about that reaches this
/// function.
///
/// Four things in the body below are decisions rather than mechanics, and
/// each is written down where a reader will look for it rather than inline:
///
/// * step 1 calls [`crate::not_found`] instead of building a matching
///   [`ApiError`], so "byte-identical to the 404 a mistyped path gets" is
///   structural rather than two literals that agree today;
/// * step 2's `warn` carries the *adapter's* own sentence, because an
///   operator debugging a rail whose notification format moved needs to know
///   what would not parse and this is the only place it is recorded. It never
///   reaches the caller: [`ApiError::InvalidParam`]'s message is a fixed
///   sentence, since the adapter's `context` can quote bytes the caller sent;
/// * step 3 logs at `info`, not `warn` — the ordinary cause is benign (a rail
///   replaying a notification for a database this deployment no longer has, or
///   a callback host shared with another environment);
/// * step 5 is two statements and not one, which matters for what the route
///   can be made to do: `enqueue_in_tx` is `ON CONFLICT DO NOTHING` and
///   writes nothing in the ordinary case, where the confirm already queued
///   the poll and `pull_forward_in_tx` does the work — and the pull-forward
///   is refused outright for a job due within [`PULL_FORWARD_FLOOR`], so the
///   ordinary case for a *freshly confirmed* charge is that neither
///   statement changes a row. It is there for what the ladder
///   cannot cover: a job an operator deleted, or one already finished, where
///   a fresh job at `now()` is what re-asks the rail. `poll_charge`'s own
///   terminal guard makes the second harmless — it answers `Outcome::Done`
///   for a terminal charge and names "a callback that arrived after the
///   answer" as one of the three reasons that happens. The payload is the
///   minimal one the confirm path writes, because this route has no
///   `NotFound` streak to carry and `PollChargePayload` defaults both ladder
///   fields.
///
/// `docs/reference/vpay-api.md` §"The rail callback route" is the long form.
///
/// # Errors
///
/// [`ApiError::UnknownRoute`] for a rail code this process does not link;
/// [`ApiError::InvalidParam`] for a body this rail's adapter cannot read;
/// [`ApiError::Db`] if the enqueue transaction fails, which is the one case
/// a resent notification would help — the poll was not moved and nothing
/// else will move it before the next rung. MTN documents that it never
/// resends one (module docs), so on that rail the next rung is what settles
/// the charge: later, not never.
#[expect(
    clippy::too_many_arguments,
    reason = "an axum handler: each argument is an extractor, and bundling them would need a \
              custom extractor that hides which parts of the request the route reads"
)]
async fn callback(
    State(adapters): State<Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>>,
    State(repositories): State<Arc<dyn Repositories>>,
    State(resources): State<Arc<ResourceConfig>>,
    Path(code): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let Some(adapter) = adapters.get(&code) else {
        return Err(crate::not_found(method, uri).await);
    };

    let reference = match adapter.parse_callback(&body) {
        Ok(reference) => reference,
        Err(error) => {
            tracing::warn!(
                rail = %code,
                error = %error,
                source_chain = %vpay_core::error::source_chain(&error),
                body_bytes = body.len(),
                "a rail callback body could not be read as a notification from this rail"
            );
            return Err(ApiError::invalid_param(
                "body",
                "The request body is not a notification this rail could have sent.",
            ));
        }
    };

    let Some(charge) = repositories
        .get_by_provider_reference(&code, reference.reference_id)
        .await?
    else {
        tracing::info!(
            rail = %code,
            reference_id = %reference.reference_id,
            "a rail callback named a reference this deployment has no charge for; accepting it \
             anyway, with the same 202 a known reference gets"
        );
        return Ok(StatusCode::ACCEPTED);
    };

    let verdict = verdict_for(
        adapter.as_ref(),
        &resources,
        &code,
        &charge,
        &headers,
        &body,
    )
    .await;
    metrics::counter!(
        PROVIDER_CALLBACKS_TOTAL,
        "provider" => code.clone(),
        "verdict" => verdict.label(),
    )
    .increment(1);
    if !verdict.admits_pull_forward() {
        tracing::warn!(
            rail = %code,
            charge_id = %charge.id,
            reference_id = %reference.reference_id,
            verdict = verdict.label(),
            reason = verdict.reason().unwrap_or_default(),
            "a rail callback was refused by this rail's check; the charge's poll was not moved, \
             and the poll ladder still asks the rail at its next rung"
        );
        return Ok(StatusCode::ACCEPTED);
    }

    let dedupe_key = poll_dedupe_key(&charge.id);
    let pulled_forward = repositories
        .transaction(|tx| {
            let dedupe_key = &dedupe_key;
            let charge_id = &charge.id;
            Box::pin(async move {
                tx.enqueue_in_tx(
                    POLL_CHARGE_KIND,
                    dedupe_key,
                    &json!({ "charge_id": charge_id }),
                    std::time::Duration::ZERO,
                )
                .await?;
                let pulled = tx
                    .pull_forward_in_tx(dedupe_key, PULL_FORWARD_FLOOR)
                    .await?;
                Ok::<_, ApiError>(TxOutcome::Commit(pulled))
            })
        })
        .await?
        .into_inner();

    tracing::info!(
        rail = %code,
        charge_id = %charge.id,
        reference_id = %reference.reference_id,
        pulled_forward,
        verdict = verdict.label(),
        "a rail callback moved a charge's poll forward"
    );
    Ok(StatusCode::ACCEPTED)
}

/// Step 4: the rail's own answer to "is this callback mine?" (RFC-0014
/// § 3).
///
/// The configuration is the rail's, by `code`, enabled or not: a disabled
/// rail's charges are still being settled, and its callbacks still arrive.
/// It is never anything the request names (RFC-0013 § 7). The [`ChargeRef`]
/// is the one the poll would build from the same row.
///
/// Two things this route cannot build are answered `Unavailable` rather than
/// passed to the adapter or skipped: a rail with no loaded configuration,
/// and a row whose currency or amount does not parse. Either one admits no
/// pull-forward, so a check that could not run never counts as one that
/// passed; the poll ladder still runs, and neither can happen to a charge
/// this deployment wrote.
///
/// **A rail that declares a check and answers `NotChecked` fails closed**:
/// `Unavailable`, with an `error` naming the rail. That is an adapter that
/// declares [`CallbackCheck::Signature`] (so boot demands and accepts its
/// secret) and kept the port's default `verify_callback`; without this rule
/// every forged callback on it would pull a poll forward, and nothing but
/// the conformance suite's own list of rails would notice. _(Added
/// 2026-10-23 from RFC-0014's security review.)_
///
/// The rail's configuration is borrowed
/// ([`crate::v1::RailConfig::provider_config_ref`]), never cloned: this is
/// the one place an anonymous request reaches a rail's credentials.
async fn verdict_for(
    adapter: &dyn ProviderAdapter,
    resources: &ResourceConfig,
    code: &str,
    charge: &ChargeRow,
    headers: &HeaderMap,
    body: &[u8],
) -> CallbackVerdict {
    let Some(rail) = resources.rail(code) else {
        return CallbackVerdict::Unavailable("this process holds no configuration for the rail");
    };
    let Some(charge_ref) = charge_ref_of(charge) else {
        return CallbackVerdict::Unavailable(
            "the charge's row does not parse as a charge reference",
        );
    };
    let request = CallbackRequest {
        headers,
        body,
        received_at: SystemTime::now(),
    };
    let verdict = adapter
        .verify_callback(&request, &charge_ref, rail.provider_config_ref())
        .await;
    if verdict == CallbackVerdict::NotChecked
        && adapter.capabilities().callback_check != CallbackCheck::None
    {
        tracing::error!(
            rail = %code,
            "this rail's adapter declares a callback check and answered NotChecked, which is the \
             port's default: its verify_callback is not implemented. The callback is refused \
             (verdict unavailable) and the poll ladder still runs; fix the adapter (RFC-0014)"
        );
        return CallbackVerdict::Unavailable(
            "the rail's adapter declares a callback check and performed none",
        );
    }
    verdict
}

/// The [`ChargeRef`] `vpay_worker`'s poll builds from the same row, built
/// the same way: the payer's method type as the confirm recorded it, string
/// `ref_extra` values only. `None` for a currency or amount that does not
/// parse, which [`verdict_for`] answers `Unavailable`.
///
/// Written out here rather than shared because the dependency runs the other
/// way (`vpay-worker` links `vpay-api`), on [`PULL_FORWARD_FLOOR`]'s terms.
fn charge_ref_of(charge: &ChargeRow) -> Option<ChargeRef> {
    let currency = Currency::from_code(&charge.currency_code).ok()?;
    let ref_extra: RefExtra = charge
        .provider_ref_extra
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|text| (key.clone(), text.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ChargeRef {
        reference_id: charge.provider_reference_id,
        method_type: charge
            .method_type
            .as_deref()
            .and_then(vpay_core::routing::MethodType::from_wire),
        amount: Money::new(charge.amount, currency).ok()?,
        payer_ref: charge.payer_ref.clone(),
        ref_extra,
        return_url: charge.return_url.clone(),
    })
}

#[cfg(test)]
mod tests {
    use vpay_core::ProviderFlow;
    use vpay_provider::{
        CallbackRef, CallbackUrlMode, Capabilities, ChargeStatus, ProviderConfig, ProviderError,
        RefundDestination, RefundTarget, Refunded, SubmitSteps, Submitted,
    };

    use super::*;

    /// A rail that declares `check` and keeps every default the port has,
    /// `verify_callback` included: the adapter RFC-0014's security review
    /// was about. It talks to no rail; every rail call is `Unsupported`.
    #[derive(Debug)]
    struct DeclaringRail {
        check: CallbackCheck,
    }

    #[async_trait::async_trait]
    impl ProviderAdapter for DeclaringRail {
        fn code(&self) -> &'static str {
            "declaring"
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                flow: ProviderFlow::Push,
                supports_refunds: false,
                supports_partial_refunds: false,
                delivers_callbacks: true,
                requires_ip_allowlist: false,
                supports_account_holder_lookup: false,
                refund_destination: RefundDestination::Origin,
                callback_url_mode: CallbackUrlMode::Registered,
                submit_steps: SubmitSteps::One,
                callback_check: self.check,
                method_types: vpay_core::routing::MethodTypes::of(
                    &vpay_core::routing::MethodType::ALL,
                ),
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
            _destination: Option<&RefundTarget>,
            _config: &ProviderConfig,
        ) -> Result<Refunded, ProviderError> {
            Err(ProviderError::Unsupported)
        }
    }

    /// A [`ResourceConfig`] holding `declaring` when `with_rail`, and no rail
    /// at all otherwise.
    fn resources(with_rail: bool) -> ResourceConfig {
        let mut config = crate::test_fixtures::config();
        if with_rail {
            config.providers.push(vpay_config::ProviderHost {
                code: "declaring".to_owned(),
                enabled: true,
                host: vpay_config::HostEntry {
                    url: "https://rail.example".to_owned(),
                    label: "rail".to_owned(),
                },
                settings: BTreeMap::new(),
                callback_url: None,
                currency: "XAF".to_owned(),
                credentials: BTreeMap::new(),
                display_name: None,
                callbacks: None,
            });
        }
        ResourceConfig::from_config(&config).expect("the fixture projects")
    }

    /// A charge on `declaring`, in `currency_code`.
    fn charge(currency_code: &str) -> ChargeRow {
        ChargeRow {
            id: "ch_test".to_owned(),
            payment_intent_id: "pi_test".to_owned(),
            provider_code: "declaring".to_owned(),
            provider_reference_id: uuid::Uuid::nil(),
            provider_ref_extra: None,
            provider_txn_id: None,
            redirect_url: None,
            return_url: None,
            state: vpay_core::ChargeState::INITIAL.as_wire_str().to_owned(),
            amount: 5000,
            currency_code: currency_code.to_owned(),
            payer_ref: None,
            payer_ref_masked: None,
            method_type: None,
            failure_code: None,
            failure_raw: None,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }

    async fn verdict_on(
        check: CallbackCheck,
        resources: &ResourceConfig,
        charge: &ChargeRow,
    ) -> CallbackVerdict {
        verdict_for(
            &DeclaringRail { check },
            resources,
            "declaring",
            charge,
            &HeaderMap::new(),
            b"{}",
        )
        .await
    }

    /// RFC-0014 security review (2026-10-23): a rail that declares a
    /// signature check and kept the port's default `verify_callback` is
    /// refused, not admitted; a rail that declares none is today's
    /// `NotChecked`. Mutation-checked: deleting the rule in `verdict_for`
    /// fails the first assertion.
    #[tokio::test]
    async fn a_declared_check_that_was_not_performed_fails_closed() {
        let resources = resources(true);
        let charge = charge("XAF");
        for check in [CallbackCheck::Signature, CallbackCheck::BodyToken] {
            let verdict = verdict_on(check, &resources, &charge).await;
            assert!(
                matches!(verdict, CallbackVerdict::Unavailable(_)),
                "{check:?}: {verdict:?}"
            );
            assert!(!verdict.admits_pull_forward(), "{check:?}");
        }
        assert_eq!(
            verdict_on(CallbackCheck::None, &resources, &charge).await,
            CallbackVerdict::NotChecked
        );
    }

    /// `verdict_for`'s two `Unavailable` branches, neither of which a
    /// charge this deployment wrote can reach, each admitting no
    /// pull-forward; the route pulls forward only on a verdict that admits
    /// it. Mutation-checked: answering `NotChecked` in either branch fails
    /// here.
    #[tokio::test]
    async fn a_check_the_route_cannot_build_admits_no_pull_forward() {
        let no_rail = verdict_on(CallbackCheck::None, &resources(false), &charge("XAF")).await;
        assert_eq!(
            no_rail,
            CallbackVerdict::Unavailable("this process holds no configuration for the rail")
        );
        assert!(!no_rail.admits_pull_forward());

        let no_currency = verdict_on(CallbackCheck::None, &resources(true), &charge("ZZZ")).await;
        assert_eq!(
            no_currency,
            CallbackVerdict::Unavailable("the charge's row does not parse as a charge reference")
        );
        assert!(!no_currency.admits_pull_forward());
    }

    /// The path a rail is actually told to call, assembled from the two
    /// constants above, must be the one
    /// `vpay_config::ProviderHost::effective_callback_url` derives — the
    /// value that has been going out in `X-Callback-Url` and `notif_url`
    /// since Step 3.
    ///
    /// The two halves are written in different crates and neither compiles
    /// against the other, so this is the join. If the derivation moves, a
    /// rail's callback arrives at a path this router does not serve and
    /// settlement quietly reverts to polling-only, which no other test in
    /// this crate would notice.
    #[test]
    fn the_mounted_path_is_the_one_the_rails_are_told_to_call() {
        let deployment = vpay_config::Deployment {
            name: "test".to_owned(),
            livemode: false,
            public_base_url: "https://api.vpay.test".to_owned(),
            surfaces: None,
        };
        let host = vpay_config::ProviderHost {
            code: "mtn_momo".to_owned(),
            enabled: true,
            host: vpay_config::HostEntry {
                url: "https://rail.example".to_owned(),
                label: "rail".to_owned(),
            },
            settings: BTreeMap::new(),
            callback_url: None,
            currency: "EUR".to_owned(),
            credentials: BTreeMap::new(),
            display_name: None,
            callbacks: None,
        };

        let mounted = format!(
            "{}{}",
            PROVIDER_NEST,
            PROVIDER_CALLBACK_ROUTE.replace("{code}", "mtn_momo")
        );
        assert_eq!(
            host.effective_callback_url(&deployment),
            format!("https://api.vpay.test{mounted}"),
            "the rails are told to call a path this router does not mount"
        );
    }
}
