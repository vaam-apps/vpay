//! Outbound webhooks: the outbox drain, and one POST to one merchant
//! endpoint.
//!
//! Two handlers implementing `docs/flows/webhooks.md`'s two-step outbox.
//! [`handle_fan_out`] turns the `events` backlog into `webhook_deliveries`
//! rows and `deliver_webhook` jobs, one transaction per event;
//! [`handle_deliver`] renders, signs and sends one of them;
//! [`handle_scan_deliveries`] is the backstop behind both.
//!
//! Delivery has its own retry ladder ([`crate::delivery_delay`]) and never
//! consults `Classify`: a merchant's `500` is not a `ProviderError`.
//!
//! The process — both transactions, the abandonment after five passes, the
//! alert-once property, and what the backstop does not recover — is
//! `docs/flows/webhooks.md`. Why this module is shaped the way it is, and how
//! it is tested, is `docs/reference/vpay-worker.md` §"The outbox drain" and
//! §"Delivering one webhook".

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use vpay_api::model::EventObject;
use vpay_core::metrics::{WEBHOOK_DELIVERIES_TOTAL, webhook_outcome};
use vpay_db::{DeliveryRow, EventRow, JobRow, Repositories, TxOutcome, UnitOfWork as _};
use vpay_provider::http::bounded_body;

use crate::error::JobError;
use crate::jobs::{DeliverWebhookPayload, JobKind, Outcome, webhook_dedupe_key};
use crate::signing::signature_header;

/// How many backlog events one drain pass claims. Bounded, so a backlog
/// drains over several passes rather than in one enormous read; the pass
/// reschedules immediately when it comes back full.
const FAN_OUT_PAGE: i64 = 100;

/// How long the drain waits when the backlog is empty — the whole latency
/// budget between a payment settling and its webhook being *enqueued*.
///
/// A poll rather than a `LISTEN/NOTIFY` because the drain must also pick up
/// events written by a process that has since died, which a notification
/// would not deliver.
const FAN_OUT_IDLE: Duration = Duration::from_secs(5);

/// How many failed fan-out passes an event gets before the drain abandons it
/// (`events.fanout_state = 'failed'`, migration 0024).
///
/// Five: large enough that ~25 seconds of Postgres being unavailable does not
/// abandon anything, small enough that a poisoned page clears in under a
/// minute. A `failed` event is a webhook the merchant will never receive and
/// nothing retries it — `docs/flows/webhooks.md` states that cost and
/// `docs/runbooks/webhook-delivery-failures.md` is how one is re-armed.
pub const FANOUT_MAX_ATTEMPTS: i32 = 5;

/// How long a webhook delivery waits to *connect* to a merchant's receiver.
///
/// Deliberately shorter than a rail's connect budget: a receiver that cannot
/// be reached is retried on [`crate::delivery_delay`] within seconds, so
/// holding a worker task open longer buys nothing.
///
/// The client itself is built by `vpay-worker-bin` — this crate must not build
/// one, see [`crate::handlers::WebhookContext`] — but the budget lives beside
/// the handler that spends it, for the reason
/// `docs/reference/vpay-worker.md` §"Two retry ladders" gives.
pub const WEBHOOK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a webhook delivery waits for the whole request/response.
///
/// A merchant handler that takes longer has not acknowledged anything a sender
/// can rely on, and the delivery is retried — which is why
/// `docs/flows/webhooks.md` tells merchants to acknowledge first and work
/// afterwards. Paired with `MAX_ACK_BODY_BYTES`, it is what stops one slow or
/// chatty receiver occupying a worker task indefinitely.
///
/// Single-sourced for [`WEBHOOK_CONNECT_TIMEOUT`]'s reason.
pub const WEBHOOK_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the delivery backstop scans — the same interval
/// `handlers::scan_live_charges` uses for charges, and a *backstop*, so a
/// healthy deployment's pass finds nothing.
const SCAN_DELIVERIES_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// How many outstanding deliveries one backstop pass re-enqueues.
const SCAN_DELIVERIES_BATCH: i64 = 500;

/// The cap on a receiver's acknowledgement body, in bytes.
///
/// Deliberately *not* `vpay_provider::MAX_RAIL_BODY_BYTES`: a receiver's ack
/// is not a rail's answer. Nothing in vpay parses this body — it is read only
/// so an operator can see what the receiver said — so the cap exists purely
/// to stop a misconfigured endpoint streaming an unbounded response into the
/// worker.
const MAX_ACK_BODY_BYTES: usize = 8 * 1024;

/// How much of that body reaches `webhook_deliveries.response_excerpt`.
///
/// Shorter than the column's own 2000-character ceiling, which is the
/// repository's backstop rather than this handler's intent: 512 characters is
/// enough of an HTML error page or a JSON error body to recognise it in a
/// runbook, and the row is a state row read by humans, not a log.
const EXCERPT_CHARS: usize = 512;

/// The header carrying `t=…,v1=…` — `docs/flows/webhooks.md`.
const SIGNATURE_HEADER: &str = "Vpay-Signature";
/// The same value again under Stripe's header name, so a merchant can hand
/// the request straight to `stripe.webhooks.constructEvent` (the official
/// SDKs verify `t=…,v1=…` over `{t}.{body}` with HMAC-SHA256 — byte-identical
/// to this scheme). Official-SDK compatibility is Step 5b; this header is the
/// one piece of it that belongs to the deliverer.
const STRIPE_SIGNATURE_HEADER: &str = "Stripe-Signature";

/// The event's id, repeated as a header.
///
/// A convenience for a merchant deduping in an access log or a proxy, and
/// **not** part of the signed payload: only the body is signed, so a value
/// here is not evidence of anything and a receiver must still read
/// `event.id` out of the verified body.
const EVENT_ID_HEADER: &str = "Vpay-Event-Id";

/// One configured webhook endpoint: where to send, and what to sign with.
///
/// Mirrors `vpay_config::oauth::WebhookEndpoint` rather than being it — the
/// binary projects configuration into this type at boot, exactly as it
/// projects rails into [`crate::RailConfigs`], so a handler cannot depend on
/// the shape of a YAML document or read some *other* field of a merchant's
/// client registration.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The operator-authored id, unique within a merchant, stored on every
    /// delivery row. Not a hash of the URL: an operator fixing a typo'd URL
    /// must not orphan the delivery history (migration 0022).
    pub id: String,
    /// The absolute URL to POST to. Validated at **boot**
    /// (`vpay_config::validate_webhook_url`), never here — see
    /// [`handle_deliver`].
    pub url: String,
    /// The signing secrets, in configuration order. One normally; two during
    /// a rotation, which is why this is a `Vec` and not a `String`.
    pub secrets: Vec<String>,
}

/// Redacts [`Endpoint::secrets`] down to a count: a webhook secret in a log is
/// a forged webhook. `docs/reference/vpay-worker.md` §"Delivering one webhook"
/// says why the count survives.
impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Endpoint")
            .field("id", &self.id)
            .field("url", &self.url)
            .field(
                "secrets",
                &format_args!("[{} redacted]", self.secrets.len()),
            )
            .finish()
    }
}

/// Every merchant's webhook endpoints, by `events.merchant_id` — and
/// deliberately not by `client_id`, which is the key
/// `vpay_api::v1::ResourceConfig` uses. `docs/reference/vpay-worker.md`
/// §"Delivering one webhook" says what the other key would fan out to.
#[derive(Clone, PartialEq, Eq)]
pub struct EndpointRegistry {
    by_merchant_id: BTreeMap<String, Vec<Endpoint>>,
}

/// Redacts every secret, for [`Endpoint`]'s reason. Ids and URLs stay visible:
/// they are already in the delivery rows and in the operator's own
/// configuration, and debugging "why did this merchant get no webhook?" needs
/// the registry the worker actually built.
impl fmt::Debug for EndpointRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointRegistry")
            .field("by_merchant_id", &self.by_merchant_id)
            .finish()
    }
}

impl EndpointRegistry {
    /// Builds the registry from `(merchant_id, endpoints)` pairs — what the
    /// worker binary projects out of `merchant_clients[].webhooks[]` at boot.
    ///
    /// Pairs rather than a map because two merchant clients may name the same
    /// `merchant_id` and still share one set of endpoints, so the pairs are
    /// merged here rather than by every caller.
    ///
    /// Within a merchant the endpoints are sorted by id and duplicates are
    /// dropped, keeping the first — defence in depth, not the guard; boot-time
    /// validation refuses a duplicate `id`. See
    /// `docs/reference/vpay-worker.md` §"Delivering one webhook".
    #[must_use]
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, Vec<Endpoint>)>) -> Self {
        let mut by_merchant_id: BTreeMap<String, Vec<Endpoint>> = BTreeMap::new();
        for (merchant_id, endpoints) in pairs {
            by_merchant_id
                .entry(merchant_id)
                .or_default()
                .extend(endpoints);
        }
        for endpoints in by_merchant_id.values_mut() {
            endpoints.sort_by(|a, b| a.id.cmp(&b.id));
            endpoints.dedup_by(|a, b| a.id == b.id);
        }
        Self { by_merchant_id }
    }

    /// This merchant's endpoints, in id order, or an empty slice.
    ///
    /// An empty slice is a *normal* answer, not a missing entry: a merchant
    /// who has configured no webhooks still has their events fanned out to
    /// nothing and marked done, or the backlog index grows forever.
    #[must_use]
    pub fn for_merchant(&self, merchant_id: &str) -> &[Endpoint] {
        self.by_merchant_id
            .get(merchant_id)
            .map_or(&[], Vec::as_slice)
    }

    /// One endpoint by (merchant, id) — the delivery handler's lookup.
    ///
    /// `None` means configuration no longer describes the endpoint. That is a
    /// real and expected state rather than a broken join —
    /// `webhook_deliveries.endpoint_id` references no table on purpose,
    /// because endpoints are YAML (ADR-0003) — and [`handle_deliver`] records an
    /// ordinary failed attempt for it.
    #[must_use]
    pub fn find(&self, merchant_id: &str, endpoint_id: &str) -> Option<&Endpoint> {
        self.for_merchant(merchant_id)
            .iter()
            .find(|endpoint| endpoint.id == endpoint_id)
    }
}

/// The exact bytes vpay signs and sends for one event.
///
/// Rendered through `vpay_api::model::EventObject`, which is also what
/// `GET /v1/events` serves — sharing that is the point and not an economy.
///
/// # Errors
///
/// [`vpay_api::ApiError::Internal`] if the row cannot be rendered — an
/// `events.data` that is not a JSON object, which migration 0018's
/// `data_is_object` CHECK makes unreachable, or a serialisation failure that
/// cannot arise from a `Value` that came out of Postgres. The caller turns it
/// into [`JobError::Poisoned`]: a delivery whose event will not render cannot
/// be fixed by trying again.
pub fn event_bytes(row: &EventRow) -> Result<Vec<u8>, vpay_api::ApiError> {
    let object = EventObject::try_from(row)?;
    serde_json::to_vec(&object).map_err(|error| {
        vpay_api::ApiError::Internal(format!("event {} did not serialise: {error}", row.id))
    })
}

/// Lowercase hex SHA-256 of the bytes that were signed.
///
/// Written to `webhook_deliveries.payload_sha256` on the first attempt that
/// **rendered and signed a body** — not necessarily attempt 1 — and compared
/// on every later one. The body itself is deliberately not stored; see
/// `docs/reference/vpay-worker.md` §"Delivering one webhook".
#[must_use]
pub fn payload_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Drains the event backlog into delivery rows and delivery jobs.
///
/// One transaction *per event* — never one for the page — containing every
/// [`vpay_db::TxRepositories::create_in_tx`], its
/// [`vpay_db::TxRepositories::enqueue_in_tx`], and the
/// [`vpay_db::TxRepositories::mark_fanned_out_in_tx`] that closes it. A crash
/// mid-page therefore loses nothing and duplicates nothing.
///
/// An event whose merchant has **no** configured endpoints is still marked
/// `done`, with zero deliveries: leaving it `pending` would grow
/// `events_pending_idx` without bound and re-scan the same rows forever.
///
/// The pass reschedules itself immediately when the page came back full *and*
/// something drained, otherwise after the module's idle interval —
/// conditional on progress, because a page of 100 events that all fail is a
/// tight loop against Postgres rather than a backlog draining at full speed.
///
/// One event's failure does not stop the page: it is counted against that
/// event ([`vpay_db::Events::record_fanout_failure`]) and the pass moves on,
/// and [`FANOUT_MAX_ATTEMPTS`] of them abandon it with exactly one alert.
/// `docs/flows/webhooks.md` states both properties in operator terms and
/// `docs/reference/vpay-worker.md` §"The outbox drain" says why the code is
/// arranged to keep them.
///
/// # Errors
///
/// [`JobError::Db`] only for a failure to *read* the backlog — there is
/// nothing to isolate at that point and the page is still there to retry. A
/// per-event failure is logged and swallowed, never returned; so is a failure
/// to record one, which leaves the event `pending` with its old count.
pub async fn handle_fan_out(
    repositories: &dyn Repositories,
    endpoints: &EndpointRegistry,
    job: &JobRow,
) -> Result<Outcome, JobError> {
    let backlog = repositories.pending_page(FAN_OUT_PAGE).await?;
    let page_was_full = i64::try_from(backlog.len()).unwrap_or(i64::MAX) >= FAN_OUT_PAGE;

    let mut drained = 0_usize;
    let mut failed = 0_usize;
    for event in &backlog {
        match fan_out_one(repositories, endpoints, job, event).await {
            Ok(()) => drained = drained.saturating_add(1),
            Err(error) => {
                failed = failed.saturating_add(1);
                record_fan_out_failure(repositories, job, event, &error).await;
            }
        }
    }

    if failed > 0 {
        tracing::warn!(
            job_id = %job.id,
            page = backlog.len(),
            drained,
            failed,
            "the outbox drain finished a page with failures"
        );
    }

    Ok(Outcome::RescheduleAfter(if page_was_full && drained > 0 {
        Duration::ZERO
    } else {
        FAN_OUT_IDLE
    }))
}

/// What one recorded fan-out failure means for the operator.
///
/// Three answers rather than a `bool`, because the third one is real and
/// silently collapsing it into either of the others would be wrong: a
/// concurrent drain can fan the event out between this pass failing on it and
/// this pass counting the failure, and that is neither a retry nor an
/// abandonment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FanOutDisposition {
    /// Attempts remain. `warn`, and deliberately **no** `alert`.
    Retrying,
    /// This failure was the one that spent [`FANOUT_MAX_ATTEMPTS`]. One
    /// `error!(alert = true, …)`, exactly once in the event's life.
    Abandoned,
    /// The event was no longer `pending` when the failure was counted:
    /// another drain fanned it out. Nothing is owed and nobody is woken.
    Claimed,
}

/// Reads [`vpay_db::Events::record_fanout_failure`]'s answer as a
/// disposition.
///
/// Split out of [`handle_fan_out`] because it decides *whether an operator is
/// paged*, and that is worth pinning without a database. The two properties
/// it carries: an alert happens only on the transition to `failed`, and a row
/// that was no longer `pending` produces no alert at all — which together are
/// what make "99 poisoned events cost 99 alerts, not 99 per pass" true.
fn fan_out_disposition(recorded: Option<&vpay_db::events::FanoutFailure>) -> FanOutDisposition {
    match recorded {
        None => FanOutDisposition::Claimed,
        // The vocabulary is the database's (`fanout_state_is_known`), so it
        // is compared and not parsed — the same treatment `DeliveryRow::state`
        // gets in the delivery handler.
        Some(failure) if failure.state == "failed" => FanOutDisposition::Abandoned,
        Some(_) => FanOutDisposition::Retrying,
    }
}

/// Counts one event's fan-out failure and says so at the right volume.
///
/// The count is a separate statement on purpose — see
/// [`vpay_db::Events::record_fanout_failure`]: the transaction whose failure
/// is being counted has rolled back, so a counter inside it would roll back
/// too and the event would be retried forever at zero.
///
/// Nothing here returns an error. The pass has already decided to continue
/// (see [`handle_fan_out`]), and a failure to *count* a failure must not
/// become the thing that stops the drain — it is logged and the event keeps
/// its old count, so the next pass counts again.
async fn record_fan_out_failure(
    repositories: &dyn Repositories,
    job: &JobRow,
    event: &EventRow,
    error: &JobError,
) {
    let recorded = match repositories
        .record_fanout_failure(&event.id, FANOUT_MAX_ATTEMPTS)
        .await
    {
        Ok(recorded) => recorded,
        Err(count_error) => {
            tracing::warn!(
                job_id = %job.id,
                event_id = %event.id,
                merchant_id = %event.merchant_id,
                error = %error,
                count_error = %count_error,
                "an event could not be fanned out, and the failure could not be counted \
                 against it; it stays pending with its previous count"
            );
            return;
        }
    };

    match fan_out_disposition(recorded.as_ref()) {
        FanOutDisposition::Abandoned => {
            // `Severity::Error` with `alert = true`, for the reason
            // `record_failure`'s exhaustion arm uses it: the job itself did
            // nothing wrong and will not fail, so this line is the only thing
            // that will ever get a human to look. It fires once — the flip is
            // guarded on `fanout_state = 'pending'` and a `failed` event
            // never returns to a page. The event id and the merchant are what
            // the runbook needs to find the row and the configuration behind
            // it.
            tracing::error!(
                alert = true,
                job_id = %job.id,
                event_id = %event.id,
                merchant_id = %event.merchant_id,
                event_type = %event.event_type,
                attempts = recorded.as_ref().map_or(0, |failure| failure.attempts),
                error = %error,
                "an event failed fan-out for the last time and has been abandoned \
                 (fanout_state = 'failed'); the merchant will not receive this event and \
                 nothing will retry it"
            );
        }
        FanOutDisposition::Retrying => {
            // No `alert`: this is retried within seconds, and alerting on it
            // is what turned one poisoned event into an unbounded page storm.
            tracing::warn!(
                job_id = %job.id,
                event_id = %event.id,
                merchant_id = %event.merchant_id,
                event_type = %event.event_type,
                attempts = recorded.as_ref().map_or(0, |failure| failure.attempts),
                max_attempts = FANOUT_MAX_ATTEMPTS,
                error = %error,
                "an event could not be fanned out; it stays pending and the rest of the \
                 page continues"
            );
        }
        FanOutDisposition::Claimed => {
            tracing::warn!(
                job_id = %job.id,
                event_id = %event.id,
                merchant_id = %event.merchant_id,
                error = %error,
                "an event could not be fanned out by this pass, but another pass has since \
                 fanned it out; nothing is owed"
            );
        }
    }
}

/// One event's transaction: every delivery, every job, and the flip.
async fn fan_out_one(
    repositories: &dyn Repositories,
    endpoints: &EndpointRegistry,
    job: &JobRow,
    event: &EventRow,
) -> Result<(), JobError> {
    let outcome = repositories
        .transaction(|tx| {
            Box::pin(async move {
                let mut created = 0_usize;
                for endpoint in endpoints.for_merchant(&event.merchant_id) {
                    let delivery_id = tx
                        .create_in_tx(&event.id, &endpoint.id, &endpoint.url)
                        .await?;
                    // `None` means another pass already created this row. It
                    // cannot be one of *our* earlier passes — a pass that got
                    // this far and committed also flipped `fanout_state`, so
                    // the event would not be in the backlog — so it is a
                    // concurrent drain, and the `mark_fanned_out_in_tx` below
                    // will tell us to roll back.
                    let Some(delivery_id) = delivery_id else {
                        continue;
                    };
                    tx.enqueue_in_tx(
                        JobKind::DeliverWebhook.as_wire_str(),
                        &webhook_dedupe_key(delivery_id),
                        &encode(job, &DeliverWebhookPayload::new(delivery_id))?,
                        OffsetDateTime::now_utc(),
                    )
                    .await?;
                    created = created.saturating_add(1);
                }

                // The compare-and-swap that makes the whole transaction safe
                // to replay: `false` means another drain claimed this event
                // while we were building ours, so everything above it was
                // computed against a backlog entry that is no longer ours to
                // claim — abandoned, and that is not a failure.
                Ok::<_, JobError>(if tx.mark_fanned_out_in_tx(&event.id).await? {
                    TxOutcome::Commit(created)
                } else {
                    TxOutcome::Abandon(created)
                })
            })
        })
        .await?;

    if let TxOutcome::Commit(created) = outcome {
        tracing::debug!(
            job_id = %job.id,
            event_id = %event.id,
            event_type = %event.event_type,
            deliveries = created,
            "event fanned out"
        );
    }

    Ok(())
}

/// Re-enqueues a `deliver_webhook` job for every outstanding delivery nothing
/// appears to be driving.
///
/// A backstop, never the mechanism — exactly `handlers::scan_live_charges`,
/// one queue over. Every delivery is *born* with a job inside the fan-out's
/// transaction; this covers only what that transaction cannot keep true
/// afterwards, a job an operator deleted or one lost with the `jobs` table.
/// Every insert is `ON CONFLICT (dedupe_key) DO NOTHING`, so a delivery that
/// already has a job is untouched.
///
/// It deliberately does **not** resurrect a delivery whose job was
/// *dead-lettered*: the parked row still holds the `dedupe_key`, and a
/// `deliver_webhook` job is parked only for reasons no retry fixes. What this
/// pass does instead is name those deliveries in one `warn!` per pass.
/// `docs/flows/webhooks.md` carries both statements and the manual un-park.
///
/// `lease` is `RecoveryPolicy::lease`, which is what keeps the
/// never-attempted arm of [`vpay_db::WebhookDeliveries::pending_due`] from
/// racing the queue on a delivery created moments ago.
///
/// # Errors
///
/// [`JobError::Db`] for any Postgres failure and [`JobError::Poisoned`] for a
/// payload that will not encode. Unlike [`handle_fan_out`] there is nothing to
/// isolate — see `docs/reference/vpay-worker.md` §"The outbox drain" for why
/// this pass may share one transaction across its page and the drain may not.
/// A pass that fails is logged with `alert = true` before the error is
/// returned; the error still propagates, so `Classify::retry` reschedules the
/// pass as it always did.
pub async fn handle_scan_deliveries(
    repositories: &dyn Repositories,
    lease: Duration,
    job: &JobRow,
) -> Result<Outcome, JobError> {
    match scan_deliveries_pass(repositories, lease, job).await {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            tracing::error!(
                alert = true,
                job_id = %job.id,
                error = %error,
                "the webhook delivery backstop failed a pass; while it is failing, a delivery \
                 whose job was lost is owed an attempt nothing will make"
            );
            Err(error)
        }
    }
}

/// The pass itself. Separated from [`handle_scan_deliveries`] only so that
/// every one of its `?`s reaches the alert above — an early return added
/// later cannot bypass it.
async fn scan_deliveries_pass(
    repositories: &dyn Repositories,
    lease: Duration,
    job: &JobRow,
) -> Result<Outcome, JobError> {
    let outstanding = repositories
        .pending_due(lease, SCAN_DELIVERIES_BATCH)
        .await?;

    let mut enqueued = 0_usize;
    let mut untouched: Vec<String> = Vec::new();
    if !outstanding.is_empty() {
        let now = OffsetDateTime::now_utc();
        (enqueued, untouched) = repositories
            .transaction(|tx| {
                // Borrowed, not moved: the log lines below read `outstanding`.
                let outstanding = &outstanding;
                Box::pin(async move {
                    let mut enqueued = 0_usize;
                    let mut untouched: Vec<String> = Vec::new();
                    for delivery in outstanding {
                        let dedupe_key = webhook_dedupe_key(delivery.id);
                        let inserted = tx
                            .enqueue_in_tx(
                                JobKind::DeliverWebhook.as_wire_str(),
                                &dedupe_key,
                                &encode(job, &DeliverWebhookPayload::new(delivery.id))?,
                                now,
                            )
                            .await?;
                        if inserted {
                            enqueued = enqueued.saturating_add(1);
                        } else {
                            // The key was taken. Usually by a perfectly
                            // healthy job this pass raced; sometimes by a
                            // parked one, which is the case below.
                            untouched.push(dedupe_key);
                        }
                    }
                    Ok::<_, JobError>(TxOutcome::Commit((enqueued, untouched)))
                })
            })
            .await?
            .into_inner();
    }

    if enqueued > 0 {
        tracing::warn!(
            job_id = %job.id,
            outstanding = outstanding.len(),
            enqueued,
            "the delivery backstop found webhook deliveries with no delivery job"
        );
    }

    // Asked only about the keys the insert did not take, and only after the
    // transaction has committed: a parked row is a permanent state, so
    // reading it a moment later costs nothing and keeps the write path one
    // transaction.
    let parked = repositories.parked_dedupe_keys(&untouched).await?;
    if !parked.is_empty() {
        let named: Vec<&str> = parked
            .iter()
            .take(PARKED_NAMED_IN_LOG)
            .map(String::as_str)
            .collect();
        tracing::warn!(
            job_id = %job.id,
            parked = parked.len(),
            deliveries = ?named,
            "webhook deliveries are pending with a dead-lettered (parked) delivery job; this \
             backstop cannot recover them, and it will not try — see \
             docs/runbooks/webhook-delivery-failures.md to un-park one after fixing the cause"
        );
    }

    Ok(Outcome::RescheduleAfter(SCAN_DELIVERIES_INTERVAL))
}

/// How many parked dedupe keys the backstop's `warn` names individually.
///
/// The count is always exact; only the list is cut. A pass can read
/// [`SCAN_DELIVERIES_BATCH`] rows, and 500 UUIDs on one line is a log record
/// most collectors truncate in the middle — losing the count as well as the
/// tail. Twenty is enough to start a runbook with; the query in the runbook
/// is what enumerates the rest.
const PARKED_NAMED_IN_LOG: usize = 20;

/// Renders, signs and POSTs one delivery, then records what happened.
///
/// The order is what makes an attempt auditable: render and hash the body
/// *before* the request, compare that digest against the one the first signed
/// attempt stored, and record the outcome whether the receiver answered or
/// not. A transport failure is recorded with `status_code = NULL` — "the
/// request went out and nothing came back", distinct from a heard refusal,
/// and the one distinction this row must never blur.
///
/// The URL is deliberately not checked here and redirects are refused by the
/// client rather than by this function. `docs/reference/vpay-worker.md`
/// §"Delivering one webhook" states what that leaves possible and why.
///
/// # Errors
///
/// [`JobError::Db`] for a Postgres failure. [`JobError::Poisoned`] for a job
/// payload that will not decode, an event the delivery names that does not
/// exist, an event that will not render, and — the one that matters — a
/// rendered body whose digest differs from the one stored by the first
/// attempt that rendered and signed a body. That last case means a renderer
/// changed under a live delivery, which a merchant would see as a webhook
/// whose signature does not verify; dead-lettering it is the only answer that
/// does not send bytes nobody can check.
///
/// A receiver that refuses is **not** an error: it is a recorded failed
/// attempt and `Ok(RescheduleAfter)`, or `Ok(Done)` once the ladder is spent.
pub async fn handle_deliver(
    repositories: &dyn Repositories,
    http: &reqwest::Client,
    endpoints: &EndpointRegistry,
    job: &JobRow,
) -> Result<Outcome, JobError> {
    let payload: DeliverWebhookPayload = decode(job)?;
    let Some((delivery, event)) = owed_delivery(repositories, job, payload.delivery_id).await?
    else {
        return Ok(Outcome::Done);
    };

    let body = event_bytes(&event)
        .map_err(|error| poisoned(job, format!("event `{}`: {error}", event.id)))?;
    let sha = payload_sha256(&body);
    refuse_a_re_rendered_body(job, &delivery, &event, &sha)?;

    let Some(endpoint) = signing_endpoint(endpoints, &event, &delivery) else {
        return record_unsigned(repositories, job, &delivery, &event).await;
    };

    match signed_post(http, &delivery, &event, endpoint, body)
        .send()
        .await
    {
        Ok(response) => {
            record_receiver_answer(
                repositories,
                job,
                &delivery,
                &event,
                endpoint,
                &sha,
                response,
            )
            .await
        }
        Err(error) => record_no_response(repositories, job, &delivery, &sha, &error).await,
    }
}

/// The delivery this job names together with the event behind it, or `None`
/// when nothing is owed.
///
/// `None` twice over, and neither is a failure: the row is gone (the job's
/// only referent has been removed), or the delivery is already settled — by
/// an earlier run of this job whose delete was lost, or by a second worker
/// after a lease was reaped. The compare-and-swap writes make those
/// indistinguishable on purpose.
///
/// # Errors
///
/// [`JobError::Db`] for a Postgres failure, and [`JobError::Poisoned`] for a
/// delivery naming an event that does not exist.
async fn owed_delivery(
    repositories: &dyn Repositories,
    job: &JobRow,
    delivery_id: uuid::Uuid,
) -> Result<Option<(DeliveryRow, EventRow)>, JobError> {
    let Some(delivery) = repositories.get(delivery_id).await? else {
        return Ok(None);
    };
    if delivery.state != "pending" {
        return Ok(None);
    }

    let Some(event) = repositories.get_unscoped(&delivery.event_id).await? else {
        return Err(poisoned(
            job,
            format!(
                "delivery {} names event `{}`, which does not exist",
                delivery.id, delivery.event_id
            ),
        ));
    };
    Ok(Some((delivery, event)))
}

/// The invariant `webhook_deliveries.payload_sha256` exists for: every attempt
/// sends the bytes the first signed attempt signed.
///
/// The digest is written by the first attempt that rendered and signed a body
/// and `COALESCE`d thereafter, so a difference means this build renders the
/// event differently from the build that signed it. `None` is the ordinary
/// answer for a delivery whose only attempts so far were abandoned before
/// rendering, and there is nothing to compare against.
///
/// # Errors
///
/// [`JobError::Poisoned`], which dead-letters: no retry re-renders the body
/// the merchant's receiver was promised.
fn refuse_a_re_rendered_body(
    job: &JobRow,
    delivery: &DeliveryRow,
    event: &EventRow,
    sha: &str,
) -> Result<(), JobError> {
    if let Some(stored) = delivery.payload_sha256.as_deref()
        && stored != sha
    {
        return Err(poisoned(
            job,
            format!(
                "delivery {} re-rendered event `{}` to a different body than the one \
                 the first signed attempt signed (stored digest {stored}, now {sha}); \
                 a renderer changed under a live delivery",
                delivery.id, event.id
            ),
        ));
    }
    Ok(())
}

/// The endpoint this delivery may be signed with, if configuration still
/// describes one and it carries a secret.
///
/// An endpoint with no secret is treated exactly as a missing one: sending
/// unsigned is not an option, because an unsigned webhook is one no receiver
/// may act on.
fn signing_endpoint<'a>(
    endpoints: &'a EndpointRegistry,
    event: &EventRow,
    delivery: &DeliveryRow,
) -> Option<&'a Endpoint> {
    endpoints
        .find(&event.merchant_id, &delivery.endpoint_id)
        .filter(|endpoint| !endpoint.secrets.is_empty())
}

/// Records the one failed attempt that never rendered or signed anything.
///
/// An ordinary failed attempt rather than an exhaustion on the spot: a
/// rollout that briefly serves an older configuration then heals, and a
/// removal that is permanent exhausts through the ladder anyway.
///
/// `None` for the digest, deliberately: nothing was sent, so there is no "the
/// bytes we sent" for `payload_sha256` to be the digest of. Recording `sha`
/// here would stamp the column on an attempt that never left the process, and
/// every later attempt's mismatch check would then be against a body no
/// receiver ever saw.
///
/// # Errors
///
/// [`JobError::Db`] for a Postgres failure.
async fn record_unsigned(
    repositories: &dyn Repositories,
    job: &JobRow,
    delivery: &DeliveryRow,
    event: &EventRow,
) -> Result<Outcome, JobError> {
    tracing::warn!(
        job_id = %job.id,
        delivery_id = %delivery.id,
        event_id = %event.id,
        endpoint_id = %delivery.endpoint_id,
        merchant_id = %event.merchant_id,
        "webhook endpoint is not configured, or has no signing secret; \
         the delivery cannot be signed and will retry"
    );
    record_failure(
        repositories,
        job,
        delivery,
        None,
        None,
        Some("endpoint is not configured with a signing secret"),
    )
    .await
}

/// The request as it goes on the wire: the signed body, the signature under
/// both header names, and the event id.
///
/// Takes the body by value because those bytes are what is signed *and* what
/// is sent — see [`crate::signing`], which refuses to re-serialise.
fn signed_post(
    http: &reqwest::Client,
    delivery: &DeliveryRow,
    event: &EventRow,
    endpoint: &Endpoint,
    body: Vec<u8>,
) -> reqwest::RequestBuilder {
    let header = signature_header(&body, OffsetDateTime::now_utc(), &endpoint.secrets);
    http.post(&delivery.url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(SIGNATURE_HEADER, header.clone())
        .header(STRIPE_SIGNATURE_HEADER, header)
        .header(EVENT_ID_HEADER, &event.id)
        .body(body)
}

/// Records what the receiver said: a 2xx settles the delivery, anything else
/// is one more failed attempt on the ladder.
///
/// # Errors
///
/// [`JobError::Db`] for a Postgres failure.
async fn record_receiver_answer(
    repositories: &dyn Repositories,
    job: &JobRow,
    delivery: &DeliveryRow,
    event: &EventRow,
    endpoint: &Endpoint,
    sha: &str,
    response: reqwest::Response,
) -> Result<Outcome, JobError> {
    let status = response.status();
    let excerpt = ack_excerpt(response).await;
    if !status.is_success() {
        return record_failure(
            repositories,
            job,
            delivery,
            Some(sha),
            Some(i32::from(status.as_u16())),
            excerpt.as_deref(),
        )
        .await;
    }

    let recorded = repositories
        .record_success(
            delivery.id,
            i32::from(status.as_u16()),
            excerpt.as_deref(),
            sha,
        )
        .await?;
    if recorded {
        // After the CAS commits, and only when it actually changed the row: a
        // second pass over an already-settled delivery is not a second
        // success, and counting it would inflate the series past the number
        // of receivers that ever answered 2xx.
        metrics::counter!(
            WEBHOOK_DELIVERIES_TOTAL,
            "outcome" => webhook_outcome::SUCCEEDED
        )
        .increment(1);
        tracing::info!(
            job_id = %job.id,
            delivery_id = %delivery.id,
            event_id = %event.id,
            endpoint_id = %endpoint.id,
            status = status.as_u16(),
            "webhook delivered"
        );
    }
    Ok(Outcome::Done)
}

/// Records an attempt that got no answer at all: DNS, connect, TLS, or the
/// request deadline.
///
/// `Some(sha)` and not `None`: the body was rendered and signed before the
/// request, and this arm only means nothing came *back*. The digest records
/// what was signed, not what was received — [`record_unsigned`] is the arm
/// that signs nothing and therefore stores nothing.
///
/// # Errors
///
/// [`JobError::Db`] for a Postgres failure.
async fn record_no_response(
    repositories: &dyn Repositories,
    job: &JobRow,
    delivery: &DeliveryRow,
    sha: &str,
    error: &reqwest::Error,
) -> Result<Outcome, JobError> {
    record_failure(
        repositories,
        job,
        delivery,
        Some(sha),
        None,
        Some(&no_response_excerpt(error)),
    )
    .await
}

/// What `webhook_deliveries.response_excerpt` says when nothing came back.
///
/// [`vpay_core::error::display_with_chain`], exactly as `jobs.last_error`
/// renders a job failure (`crate::run_loop`, ADR-0011's amendment): a
/// `reqwest::Error` renders as "error sending request for url (…)" and keeps
/// "connection refused" or "dns error" in its `source()`, so the excerpt
/// without the chain repeats a URL the operator already has and omits the only
/// part that says what went wrong.
///
/// Safe to store: the error's `Display` names the host and the cause and never
/// the request body — `reqwest` does not put one into it. Bounded by
/// `vpay_db`'s own excerpt truncation against migration 0022's
/// `excerpt_length` CHECK, which no chain a transport failure produces comes
/// near.
///
/// Takes `&dyn Error` rather than `&reqwest::Error` so the rendering is
/// testable without a socket.
fn no_response_excerpt(error: &dyn std::error::Error) -> String {
    format!(
        "no response: {}",
        vpay_core::error::display_with_chain(error)
    )
}

/// Records one failed attempt and says when — or whether — to try again.
///
/// The ladder index is the delivery's **pre-increment** `attempt`, which
/// counts failures so far: after the first failure the wait is
/// `delivery_delay(0)` = 10s, `docs/flows/webhooks.md`'s first rung, and the
/// eighth failure exhausts the ladder.
///
/// `sha` is `None` for the one failure that never rendered or signed
/// anything, and `Some` for a transport failure — the bytes were signed and
/// only the answer is missing. `docs/reference/vpay-worker.md` §"Delivering
/// one webhook" says what the column would otherwise claim.
///
/// # Errors
///
/// [`JobError::Db`] for a Postgres failure.
async fn record_failure(
    repositories: &dyn Repositories,
    job: &JobRow,
    delivery: &DeliveryRow,
    sha: Option<&str>,
    status: Option<i32>,
    excerpt: Option<&str>,
) -> Result<Outcome, JobError> {
    let ladder_index = u32::try_from(delivery.attempt).unwrap_or(u32::MAX);
    let delay = crate::delivery_delay(ladder_index);
    let next_attempt_at = delay.and_then(|delay| {
        // `time::Duration::try_from` refuses a `std::time::Duration` too wide
        // for it; no rung of the ladder is, so the `None` arm is unreachable
        // and means "do not claim a next attempt we cannot express".
        time::Duration::try_from(delay)
            .ok()
            .map(|delay| OffsetDateTime::now_utc().saturating_add(delay))
    });

    repositories
        .record_attempt(
            delivery.id,
            status,
            excerpt,
            sha,
            next_attempt_at,
            delay.is_none(),
        )
        .await?;
    // After that write commits (a single `UPDATE`, autocommitted): the
    // ladder index already decided which of the two outcomes this attempt
    // is, so the label and the row's new `state` cannot disagree.
    metrics::counter!(
        WEBHOOK_DELIVERIES_TOTAL,
        "outcome" => if delay.is_some() {
            webhook_outcome::RETRY
        } else {
            webhook_outcome::EXHAUSTED
        }
    )
    .increment(1);

    match delay {
        Some(delay) => {
            tracing::warn!(
                job_id = %job.id,
                delivery_id = %delivery.id,
                event_id = %delivery.event_id,
                endpoint_id = %delivery.endpoint_id,
                attempt = delivery.attempt.saturating_add(1),
                status = status.unwrap_or_default(),
                retry_in_seconds = delay.as_secs(),
                "webhook delivery attempt failed; retrying"
            );
            Ok(Outcome::RescheduleAfter(delay))
        }
        None => {
            // `Severity::Error` with `alert = true`: a merchant will never be
            // told about this transition by vpay, so a human has to tell
            // them. Not `Severity::Page` — nothing is broken here, the
            // receiver is — and not a `JobError`, because the *job* did
            // exactly what it was asked to. The row is the durable record
            // (`state = 'exhausted'`); this is the line that gets someone to
            // look at it.
            tracing::error!(
                alert = true,
                job_id = %job.id,
                delivery_id = %delivery.id,
                event_id = %delivery.event_id,
                endpoint_id = %delivery.endpoint_id,
                url = %delivery.url,
                attempt = delivery.attempt.saturating_add(1),
                "webhook delivery exhausted the retry ladder; the merchant has not \
                 been told about this event"
            );
            Ok(Outcome::Done)
        }
    }
}

/// The receiver's answer, bounded and truncated for an operator to read.
///
/// `None` when there was nothing to read. The two error arms record *that*
/// the body could not be taken rather than any part of it, so a receiver
/// cannot push arbitrary content into vpay's database by way of an
/// unreadable response.
async fn ack_excerpt(response: reqwest::Response) -> Option<String> {
    match bounded_body(response, MAX_ACK_BODY_BYTES).await {
        Ok((_, body)) if body.is_empty() => None,
        Ok((_, body)) => Some(
            String::from_utf8_lossy(&body)
                .chars()
                .take(EXCERPT_CHARS)
                .collect(),
        ),
        Err(error) => Some(format!("the response body could not be recorded: {error}")),
    }
}

/// A job payload, decoded into the shape its `kind` promises.
fn decode<T: serde::de::DeserializeOwned>(job: &JobRow) -> Result<T, JobError> {
    serde_json::from_value(job.payload.clone()).map_err(|error| {
        poisoned(
            job,
            format!("payload is not the shape `{}` promises: {error}", job.kind),
        )
    })
}

/// A payload this crate is about to write into `jobs.payload`.
fn encode<T: serde::Serialize>(job: &JobRow, payload: &T) -> Result<serde_json::Value, JobError> {
    serde_json::to_value(payload)
        .map_err(|error| poisoned(job, format!("could not encode a job payload: {error}")))
}

/// [`JobError::Poisoned`] naming the row that could not be interpreted.
fn poisoned(job: &JobRow, reason: String) -> JobError {
    JobError::Poisoned {
        job_id: job.id,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use time::OffsetDateTime;

    use vpay_db::events::FanoutFailure;

    use super::{
        Endpoint, EndpointRegistry, FANOUT_MAX_ATTEMPTS, FanOutDisposition, event_bytes,
        fan_out_disposition, no_response_excerpt, payload_sha256,
    };

    /// An error with a source, shaped like the `reqwest::Error` a delivery
    /// that never reached its receiver produces: the outer message names the
    /// URL, the inner one says what actually failed.
    #[derive(Debug)]
    struct Layered {
        message: &'static str,
        source: Option<Box<Layered>>,
    }

    impl std::fmt::Display for Layered {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for Layered {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.source
                .as_deref()
                .map(|source| source as &(dyn std::error::Error + 'static))
        }
    }

    fn endpoint(id: &str, url: &str, secrets: &[&str]) -> Endpoint {
        Endpoint {
            id: id.to_owned(),
            url: url.to_owned(),
            secrets: secrets.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn event_row(data: serde_json::Value) -> vpay_db::EventRow {
        vpay_db::EventRow {
            id: "evt_1".to_owned(),
            seq: 1,
            merchant_id: "merchant_a".to_owned(),
            livemode: false,
            event_type: "payment_intent.succeeded".to_owned(),
            object_id: "pi_1".to_owned(),
            data,
            fanout_state: "pending".to_owned(),
            created_at: OffsetDateTime::from_unix_timestamp(1_753_401_600)
                .expect("a fixed, valid timestamp"),
        }
    }

    #[test]
    fn a_merchants_endpoints_are_found_and_another_merchants_are_not() {
        let registry = EndpointRegistry::from_pairs([
            (
                "merchant_a".to_owned(),
                vec![
                    endpoint("primary", "https://a.example/hook", &["whsec_a"]),
                    endpoint("backup", "https://a.example/backup", &["whsec_b"]),
                ],
            ),
            (
                "merchant_b".to_owned(),
                vec![endpoint("primary", "https://b.example/hook", &["whsec_c"])],
            ),
        ]);

        // Sorted by id, so a fan-out's insert order does not depend on the
        // order a YAML document happened to list them in.
        let ids: Vec<&str> = registry
            .for_merchant("merchant_a")
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, ["backup", "primary"]);

        // Ids are unique per merchant, not globally: both merchants have a
        // `primary`, and the lookup must not confuse them. This is the
        // property that makes fanning out to the wrong merchant's URL
        // impossible.
        assert_eq!(
            registry.find("merchant_a", "primary").map(|e| &e.url),
            Some(&"https://a.example/hook".to_owned())
        );
        assert_eq!(
            registry.find("merchant_b", "primary").map(|e| &e.url),
            Some(&"https://b.example/hook".to_owned())
        );

        assert_eq!(registry.find("merchant_a", "gone"), None);
        assert_eq!(registry.find("merchant_c", "primary"), None);
    }

    /// A merchant with no webhooks is not a missing entry — the fan-out still
    /// marks their events `done`, with zero deliveries.
    #[test]
    fn an_unconfigured_merchant_yields_an_empty_slice_not_a_lookup_failure() {
        let registry = EndpointRegistry::from_pairs([]);
        assert!(registry.for_merchant("merchant_a").is_empty());
        assert!(registry.find("merchant_a", "primary").is_none());
    }

    /// Two OAuth clients under one `merchant_id` are one set of endpoints.
    /// Overwriting instead of merging would silently drop a merchant's second
    /// endpoint, and nothing downstream could tell.
    #[test]
    fn two_pairs_naming_one_merchant_are_merged() {
        let registry = EndpointRegistry::from_pairs([
            (
                "merchant_a".to_owned(),
                vec![endpoint("primary", "https://a.example/one", &["s1"])],
            ),
            (
                "merchant_a".to_owned(),
                vec![endpoint("secondary", "https://a.example/two", &["s2"])],
            ),
        ]);
        assert_eq!(registry.for_merchant("merchant_a").len(), 2);
    }

    /// Defence in depth for a duplicate id boot validation should have
    /// refused: the first wins deterministically, rather than the delivery
    /// row's URL depending on iteration order.
    #[test]
    fn a_duplicate_endpoint_id_within_a_merchant_resolves_deterministically() {
        let registry = EndpointRegistry::from_pairs([(
            "merchant_a".to_owned(),
            vec![
                endpoint("primary", "https://a.example/second", &["s2"]),
                endpoint("primary", "https://a.example/first", &["s1"]),
            ],
        )]);
        assert_eq!(registry.for_merchant("merchant_a").len(), 1);
        assert_eq!(
            registry.find("merchant_a", "primary").map(|e| &e.url),
            Some(&"https://a.example/second".to_owned()),
            "the first pair's endpoint wins, and it wins every time"
        );
    }

    /// The registry outlives the process's whole run and lands in any `{:?}`
    /// of the loop's state. A secret in a log is a forged webhook: anyone
    /// holding it can sign a `payment_intent.succeeded` a merchant's handler
    /// will believe.
    #[test]
    fn debug_never_prints_a_secret() {
        let registry = EndpointRegistry::from_pairs([(
            "merchant_a".to_owned(),
            vec![endpoint(
                "primary",
                "https://a.example/hook",
                &["whsec_super_secret", "whsec_rotating"],
            )],
        )]);

        let rendered = format!("{registry:?}");
        assert!(!rendered.contains("whsec_super_secret"), "{rendered}");
        assert!(!rendered.contains("whsec_rotating"), "{rendered}");
        // Still useful: the endpoint, the URL, and whether a rotation is in
        // progress — the three things a runbook asks about.
        assert!(rendered.contains("merchant_a"), "{rendered}");
        assert!(rendered.contains("primary"), "{rendered}");
        assert!(rendered.contains("https://a.example/hook"), "{rendered}");
        assert!(rendered.contains("[2 redacted]"), "{rendered}");

        // And on the endpoint itself, which is what a `{:?}` of one field
        // would print.
        let one = format!("{:?}", endpoint("primary", "https://a.example", &["shh"]));
        assert!(!one.contains("shh"), "{one}");
    }

    /// The body is the `EventObject` envelope and nothing else — the same
    /// bytes `GET /v1/events` would serve for this row.
    #[test]
    fn an_event_renders_to_the_envelope_that_gets_signed() {
        let bytes = event_bytes(&event_row(json!({ "id": "pi_1" }))).expect("renders");
        assert_eq!(
            String::from_utf8(bytes).expect("ASCII JSON"),
            r#"{"id":"evt_1","object":"event","type":"payment_intent.succeeded","created":1753401600,"livemode":false,"data":{"object":{"id":"pi_1"}}}"#
        );
    }

    /// The digest is over the bytes, so re-rendering the same row twice must
    /// give the same one — that is exactly what `payload_sha256` on the
    /// delivery row asserts between attempt one and attempt two.
    #[test]
    fn the_digest_is_stable_across_renders_and_moves_with_the_bytes() {
        let row = event_row(json!({ "id": "pi_1" }));
        let first = payload_sha256(&event_bytes(&row).expect("renders"));
        let second = payload_sha256(&event_bytes(&row).expect("renders"));
        assert_eq!(first, second);

        let other =
            payload_sha256(&event_bytes(&event_row(json!({ "id": "pi_2" }))).expect("renders"));
        assert_ne!(first, other);

        // Lowercase hex SHA-256, as `webhook_deliveries.payload_sha256`
        // stores it.
        assert_eq!(first.len(), 64);
        assert!(
            first
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }

    /// One fan-out failure alerts **only** on the transition to `failed`.
    ///
    /// The property under test is the one that turns an unbounded alert
    /// storm back into a bounded count: four failures are `warn`, the fifth
    /// is the single `error!(alert = true, …)`, and an answer of `None` — the
    /// event was fanned out by another pass in between — wakes nobody at all.
    /// A mapping that returned `Abandoned` for every row would pass no row of
    /// this table but the last.
    #[test]
    fn only_the_transition_to_failed_alerts() {
        let failure = |attempts: i32, state: &str| FanoutFailure {
            attempts,
            state: state.to_owned(),
        };

        // The whole ladder, spelled out rather than generated: the count is
        // what an operator reads off the `warn` line, and a table that
        // computed it would agree with whatever the code did.
        let cases: [(Option<FanoutFailure>, FanOutDisposition); 7] = [
            (Some(failure(1, "pending")), FanOutDisposition::Retrying),
            (Some(failure(2, "pending")), FanOutDisposition::Retrying),
            (Some(failure(3, "pending")), FanOutDisposition::Retrying),
            (Some(failure(4, "pending")), FanOutDisposition::Retrying),
            (Some(failure(5, "failed")), FanOutDisposition::Abandoned),
            // Not reachable through `pending_page` (a `failed` event is not
            // `pending`), and mapped anyway: the state decides, not the
            // count, so a ceiling changed in one place cannot make a second
            // alert appear here.
            (Some(failure(6, "failed")), FanOutDisposition::Abandoned),
            (None, FanOutDisposition::Claimed),
        ];

        for (recorded, expected) in cases {
            assert_eq!(
                fan_out_disposition(recorded.as_ref()),
                expected,
                "{recorded:?} was dispositioned wrongly"
            );
        }

        // The count at which the database flips the state is the constant
        // this module hands it, and the table above is written against that
        // number.
        assert_eq!(FANOUT_MAX_ATTEMPTS, 5);
    }

    /// The one vector that pins the digest function itself rather than its
    /// self-consistency: NIST's SHA-256 of the empty string.
    #[test]
    fn the_digest_is_sha_256() {
        assert_eq!(
            payload_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
    /// The excerpt carries the whole chain, not just the outermost sentence.
    /// Without it `webhook_deliveries.response_excerpt` repeats the URL an
    /// operator already has and omits "connection refused" — the same loss
    /// `jobs.last_error` fixed in Phase A (ADR-0011's amendment).
    #[test]
    fn a_delivery_that_got_no_answer_records_the_whole_source_chain() {
        let error = Layered {
            message: "error sending request for url (https://merchant.example/hook)",
            source: Some(Box::new(Layered {
                message: "client error (Connect)",
                source: Some(Box::new(Layered {
                    message: "tcp connect error: Connection refused (os error 111)",
                    source: None,
                })),
            })),
        };

        assert_eq!(
            no_response_excerpt(&error),
            "no response: error sending request for url \
             (https://merchant.example/hook): client error (Connect): tcp connect \
             error: Connection refused (os error 111)"
        );
    }

    /// An error with nothing under it renders exactly as it always did, with
    /// no trailing separator.
    #[test]
    fn an_error_with_no_source_records_only_its_own_words() {
        let error = Layered {
            message: "operation timed out",
            source: None,
        };
        assert_eq!(
            no_response_excerpt(&error),
            "no response: operation timed out"
        );
    }
}
