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
//! that moves money." Both rails sign nothing and send no shared secret
//! (`docs/flows/adapter-mtn-momo.md` §"The callback is unsigned and
//! unauthenticated"), so anyone who can reach this URL can post anything to
//! it. What this handler is therefore allowed to do is exactly one thing:
//! **bring an already-queued `poll_charge` job forward to now**, so the
//! worker asks the rail — over an authenticated status query — immediately
//! rather than at the poll ladder's next rung
//! (`vpay_worker::poll_delay(0)` is ten seconds).
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
//! # The four answers, and why the two 202s are indistinguishable
//!
//! | Case | Answer |
//! |---|---|
//! | `code` names no adapter this process links | `404`, byte-identical to the router's own fallback |
//! | the body is not a notification this rail could have sent | `400`, plus a `warn` carrying the adapter's own reason |
//! | the reference names no charge here | `202` |
//! | the reference names a charge | `202` |
//!
//! The last two are the same response on purpose, and for two reasons. A
//! rail that gets anything other than a 2xx retries — MTN and Orange both
//! do, on their own schedules — so answering `404` to a reference we do not
//! recognise buys a retry loop that can never succeed and costs a rail
//! operator a support ticket. And an unauthenticated endpoint that answered
//! differently for "this charge exists" would be an oracle for exactly that
//! question, which is the same argument [`crate::browser`] makes about its
//! uniform 404. The unknown reference is logged at `info`, which is where an
//! operator debugging a misregistered callback host will find it.
//!
//! A `404` for an unknown *rail code* is a different thing and stays: it is
//! a statement about this deployment's route table, which is public — the
//! same information a merchant gets from `payment_method_types` — and a rail
//! whose code is not linked here is not a rail that is going to retry.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{Method, StatusCode, Uri};
use axum::routing::post;
use serde_json::json;
use time::OffsetDateTime;
use vpay_db::{Repositories, TxOutcome, UnitOfWork as _};
use vpay_provider::ProviderAdapter;

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
/// 4. in one transaction, **two statements**: enqueue the poll (a no-op when
///    it already exists) and pull it forward, unless it is already due
///    within [`PULL_FORWARD_FLOOR`].
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
/// * step 4 is two statements and not one, which matters for what the route
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
/// the rail *should* retry — the poll was not moved and nothing else will
/// move it before the next rung.
async fn callback(
    State(adapters): State<Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>>,
    State(repositories): State<Arc<dyn Repositories>>,
    Path(code): Path<String>,
    method: Method,
    uri: Uri,
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
             anyway so the rail stops retrying"
        );
        return Ok(StatusCode::ACCEPTED);
    };

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
                    OffsetDateTime::now_utc(),
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
        "a rail callback moved a charge's poll forward"
    );
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use super::*;

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
