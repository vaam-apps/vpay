//! The `idempotency_keys` repository (`backends/migrations/
//! 0015_create-idempotency-keys.sql`) — claiming a key, storing the
//! response to replay, releasing a key whose request must be re-executed,
//! and sweeping expired rows.
//!
//! # The claim is one statement, on purpose
//!
//! [`Idempotency::claim`] issues `INSERT ... ON CONFLICT DO UPDATE ... RETURNING` and
//! reads whether a row came back. It never asks "does this key exist?"
//! first: two simultaneous retries of the same POST would both be told "no"
//! and both charge the payer. This is the same argument, and the same shape,
//! as `oauth_client_assertion_jtis`' replay guard (migration 0011) — the
//! primary key is the mutual exclusion, and the `RETURNING` is how this
//! process learns whether it won. The `DO UPDATE` arm is guarded so that it
//! fires only for an *expired* row; see [`Idempotency::claim`]'s own docs.
//!
//! # A key is held for as long as its request runs, and no longer
//!
//! Three things end a claim, and between them they are the whole reason a
//! key cannot get stuck: [`Idempotency::store`] completes it (the response is now
//! replayable), [`Idempotency::release`] hands it back (the outcome is one a retry should
//! re-execute rather than replay), and [`Idempotency::sweep_expired`] — or the reclaim in
//! [`Idempotency::claim`] — collects it 24 hours later if the process that held it died
//! without doing either.
//!
//! The `SELECT` that follows a conflict is not a re-check of that decision:
//! by then the answer ("someone else claimed this") is already known, and
//! the read only decides *what to tell the caller* about the other request.
//!
//! # A claim is identified by `claim_id`, not by the key
//!
//! Because an expired row is reclaimable, the primary key does not identify
//! a *claim* — only a slot that claims pass through. Request R1 claims a
//! key, stalls past the 24-hour window, R2 reclaims the same row, and R1
//! then wakes up: addressed by `(merchant_id, idempotency_key)` and
//! `state = 'in_flight'` alone, R1's [`Idempotency::release`] would delete R2's live
//! claim and R1's [`Idempotency::store`] would overwrite R2's row with R1's response.
//! That is ABA, and what it hands a merchant is another request's payment
//! object under their key.
//!
//! So [`Idempotency::claim`] returns the `claim_id` the database minted for *that* claim
//! ([`IdempotencyClaim::Fresh`]), and [`Idempotency::store`] and [`Idempotency::release`] both carry
//! `AND claim_id = $n`. A caller holding a superseded id therefore matches
//! no row, and [`Idempotency::store`] says so with
//! [`IdempotencyStoreOutcome::StaleClaim`] rather than reporting a success
//! it did not perform.
//!
//! # Why the hash comparison is constant-time
//!
//! A merchant may present any `Idempotency-Key` with any body, so the
//! stored hash is attacker-adjacent data that the response reveals one bit
//! about ("same request" vs `idempotency_key_in_use`). A byte-by-byte
//! comparison with an early exit turns that bit into a timing oracle for
//! the stored digest — the classic authentication-tag mistake, applied to a
//! digest rather than a MAC. `subtle::ConstantTimeEq` compares all 32 bytes
//! regardless.
//!
//! This is cheap insurance rather than a demonstrated attack: exploiting it
//! would need a way to make vpay hash a chosen prefix, which is a strange
//! shape here. It costs one dependency and no branches, and the failure it
//! rules out is silent.
//!
//! # The one thing this store knows about another resource
//!
//! [`Idempotency::store`] takes a [`ResponseSubject`], and one of its two
//! variants names `/v1/customers`. That is a deliberate exception in a store
//! that is otherwise indifferent to what it is keeping, and it is here
//! because the alternative was worse rather than because it is tidy.
//!
//! A response is stored **after** the handler's transaction has committed
//! (`vpay_api::v1::payment_intents::PostRequest::finish`), so the two writes
//! are not one atom. A `POST /v1/customers/{id}` that commits, is overtaken
//! by a `DELETE` that erases the same payer — sweeping every stored copy that
//! exists at that moment, `idempotency_keys.response_body` included — and
//! only then stores its own response, puts the pre-erasure object back into a
//! table the erasure has already been through. Nothing moves it again until
//! `expires_at`, so a payer's name, email and phone were re-readable for up
//! to **24 hours** after they were erased, by replaying that request's own
//! key. Issue #111.
//!
//! Two shapes were available: leave the window and document it, or give the
//! customer routes an exception here. What decided it is that the exception
//! can be made **narrow and explicit** — one variant, carrying the `cus_…`
//! the caller already holds, applying `crate::customers`' existing redaction
//! and no rule of its own. The store never inspects a body, never guesses
//! which keys are sensitive, and does nothing at all for
//! [`ResponseSubject::Verbatim`], which is every other route. A generic store
//! that sniffed responses for things that look like personal data would be a
//! far worse thing to own than the window it closed.

use serde_json::Value;
use subtle::ConstantTimeEq as _;
use uuid::Uuid;

use crate::error::DbError;

/// The stored half of an idempotent request: what it hashed to, how far it
/// got, and the response to replay if it finished.
///
/// Deliberately omits `request_method`/`request_path` (stored for operators
/// — the hash already covers both) and the timestamps: nothing that reads
/// this repository needs them to decide what to do.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct IdempotencyRecord {
    /// SHA-256 over method + path + raw body, as raw bytes. Always 32 bytes
    /// (`request_hash_is_sha256`).
    pub request_hash: Vec<u8>,
    /// `in_flight` or `complete`.
    pub state: String,
    /// The status to replay. `Some` iff `state` is `complete`, enforced by
    /// the `complete_has_a_response` CHECK.
    pub response_status: Option<i16>,
    /// The exact JSON body to replay — the bytes that were sent, not a
    /// re-render from today's code.
    pub response_body: Option<Value>,
    /// The `stripe-should-retry` value the stored response carried, verbatim
    /// (`"true"` or `"false"` — migration `0025`'s
    /// `response_retry_is_an_advisory` CHECK), or `None` if it carried none.
    ///
    /// `None` is a real answer and not a missing one: only
    /// `vpay_api::error::ApiError::into_response` emits the header, so a
    /// stored `2xx` never had one and its replay must not invent one. Kept
    /// as text rather than a `bool` so that a replay re-emits the bytes the
    /// original response carried instead of rendering the advisory a second
    /// time — see the migration for why that matters under ADR-0011.
    pub response_retry: Option<String>,
}

/// What [`Idempotency::claim`] found, and therefore what the caller must do next.
///
/// Four outcomes rather than a `bool` because each one is a different HTTP
/// answer, and collapsing any two of them would either re-execute a payment
/// or hide a genuine key collision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyClaim {
    /// This process owns the key: do the work, then call [`Idempotency::store`] or
    /// [`Idempotency::release`] with the `claim_id` handed back here.
    Fresh {
        /// Which claim this is, so a later [`Idempotency::store`] or [`Idempotency::release`] cannot
        /// act on a row that has since been reclaimed by someone else. See
        /// the module comment's ABA section for what carrying it prevents.
        claim_id: Uuid,
    },
    /// The same request already completed. Replay the stored response
    /// verbatim — do **not** re-execute it.
    Replay(IdempotencyRecord),
    /// The same request is still running somewhere (possibly in this
    /// process's own past, if it crashed before storing a response). The
    /// caller must not start a second one; the honest answer to the
    /// merchant is "in progress, retry".
    InFlight,
    /// The key was used before with a *different* request. This is the
    /// `400 idempotency_key_in_use` case: replaying the other request's
    /// answer would be wrong, and executing this one under a key that
    /// already means something else would destroy the guarantee.
    Mismatch,
}

/// The response [`Idempotency::store`] is being asked to make replayable.
///
/// A struct rather than four more parameters because the four belong
/// together — they are one HTTP response, described — and because
/// `store(merchant, key, claim, 200, &body, None, subject)` is a call in
/// which every argument after the third is a positional puzzle. Each field
/// is named at the call site instead.
#[derive(Debug, Clone, Copy)]
pub struct StoredResponse<'a> {
    /// The status to replay. `u16` on the wire and `SMALLINT` in the table;
    /// anything above `i16::MAX` is not an HTTP status and is refused rather
    /// than wrapped into a negative number.
    pub status: u16,
    /// The exact JSON body that was sent, to be replayed byte for byte
    /// rather than re-rendered by a later version of the code.
    pub body: &'a Value,
    /// The `stripe-should-retry` value this response actually carried, read
    /// off its own `HeaderMap` by the caller and never re-derived here from
    /// `status` — see [`Idempotency::store`]'s `retry` section.
    pub retry: Option<&'a str>,
    /// What the body is about. [`ResponseSubject::Verbatim`] for every route
    /// but `/v1/customers`.
    pub subject: ResponseSubject<'a>,
}

/// What the response being stored is *about*, and therefore whether an
/// erasure racing this write has to be able to beat it.
///
/// Two variants and not a `bool`, and not an `Option<&str>`: the customer
/// case is an exception to this store's whole premise (see the module
/// comment), so it is spelled at every call site rather than being the
/// absence of something. A route that adds itself to `/v1` has to say which
/// of the two it is, which is the one property that keeps the exception from
/// silently failing to apply to a fourth customer route somebody writes next
/// year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseSubject<'a> {
    /// Store the body exactly as given. Every route but `/v1/customers` —
    /// an intent, a session, an invoice and an invoice item carry a
    /// `cus_…` at most, never a payer's identifiers, and the erasure has
    /// nothing to redact in them.
    Verbatim,
    /// `/v1/customers`' shaped exception: this body renders the customer
    /// `id`, so the write must lose to an erasure of that customer rather
    /// than re-introducing them.
    ///
    /// The id comes from the route — the path segment, or the one a create
    /// minted — and never from reading the body, so a response that is not a
    /// customer object (a validation `400`, a `DELETE`'s `{deleted: true}`)
    /// is carried here too and is simply left alone: the redaction matches on
    /// the body's own `object` and `id`.
    Customer {
        /// The `cus_…` this response is about.
        id: &'a str,
    },
}

/// What [`Idempotency::store`] did with the response it was given.
///
/// Two outcomes rather than `()` because "the row you claimed is no longer
/// yours" is a real, reachable state — see the module comment's ABA section
/// — and it is neither a success (nothing was recorded) nor this process's
/// bug (the 24-hour window did exactly what it promises). Collapsing it
/// into either would be wrong in a different direction: reported as a
/// success, the caller would believe a replay is available that is not;
/// reported as an error, a merchant whose request actually completed would
/// be answered `500` and would retry a payment that already happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdempotencyStoreOutcome {
    /// The response is recorded. A retry under this key replays it.
    Stored,
    /// The `claim_id` given no longer owns this key: the claim expired and
    /// was taken over by a later request, or its row was swept. Nothing was
    /// written, and nothing should be — the row belongs to someone else.
    /// The caller may still answer its own request; it simply cannot make
    /// that answer replayable.
    StaleClaim,
}

#[async_trait::async_trait]
pub trait Idempotency: Send + Sync {
    /// Claims `key` for this request, atomically.
    ///
    /// See the module comment for why this is a single `INSERT ... ON CONFLICT
    /// DO UPDATE ... RETURNING` and never a check-then-insert.
    ///
    /// # An expired row is claimable, and that is what bounds a stuck key
    ///
    /// The `ON CONFLICT` arm updates — rather than doing nothing — but only
    /// `WHERE idempotency_keys.expires_at < now()`, so it fires for exactly the
    /// rows [`Idempotency::sweep_expired`] would have deleted and for no others. Without it,
    /// a key whose owning request died before it could [`Idempotency::store`] or
    /// [`Idempotency::release`] anything would be answered [`IdempotencyClaim::InFlight`]
    /// **forever**: nothing else in the system ever moves such a row, so the
    /// merchant's key would be unusable for the life of the deployment rather
    /// than for the 24 hours the table's `expires_at` promises. The reclaim
    /// resets the whole row (method, path, hash, state, all three response
    /// columns — status, body and the `response_retry` advisory — and the
    /// timestamps),
    /// because what it is doing is starting a new request under a key that has
    /// expired — keeping the old hash would make a genuinely different retry
    /// look like a [`IdempotencyClaim::Mismatch`]. The reset includes a fresh
    /// `claim_id`, which is what stops the previous owner — who may be about to
    /// wake up — from writing to the row it no longer holds.
    ///
    /// A conflicting row that has *vanished* between the insert and the read —
    /// possible if [`Idempotency::sweep_expired`] deleted a 24-hour-old row, or if the
    /// request that owned it called [`Idempotency::release`], in that window — is reported as
    /// [`IdempotencyClaim::InFlight`]. It is the one
    /// outcome that is safe when the state is genuinely unknown: it never lets
    /// two requests run under one key, and a retry moments later claims the key
    /// cleanly.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if either statement fails.
    async fn claim(
        &self,
        merchant_id: &str,
        key: &str,
        method: &str,
        path: &str,
        request_hash: &[u8; 32],
    ) -> Result<IdempotencyClaim, DbError>;

    /// Records the response to replay, completing a key this process claimed
    /// under `claim_id`.
    ///
    /// The `WHERE ... AND claim_id = $3 AND state = 'in_flight'` is a
    /// compare-and-swap, not a belt-and-braces filter: it is what stops a late
    /// or duplicated writer from overwriting a response a merchant has already
    /// been given, and what stops a *superseded* claim from overwriting the
    /// claim that replaced it. Zero rows matched is therefore never reported as
    /// a success — returning `Ok(())` after writing nothing would leave the
    /// caller free to answer `200` while the next replay of that key
    /// re-executed the payment.
    ///
    /// # Telling a stale claim from a broken invariant
    ///
    /// Zero rows matched has two causes, and they need different answers, so
    /// this reads the row back on that path only:
    ///
    /// * the row is gone, or its `claim_id` is not the one given → the claim
    ///   was superseded or swept, which is [`IdempotencyStoreOutcome::StaleClaim`];
    /// * the row is still this claim's and is already `complete` → the caller
    ///   is completing one key twice, which is its own invariant breaking and
    ///   is [`DbError::WriteMatchedNoRow`] (`Category::Internal`, so it pages).
    ///
    /// "Storing for a key that was never claimed" is not among the causes,
    /// because a `claim_id` can only be obtained from
    /// [`IdempotencyClaim::Fresh`] — the type is what rules that case out, so
    /// the classification does not have to.
    ///
    /// `response.status` is `u16` on the wire and `SMALLINT` in the table; a
    /// status above `i16::MAX` is not a real HTTP status and is rejected as
    /// [`DbError::WriteMatchedNoRow`] rather than silently wrapping into a
    /// negative number.
    ///
    /// # `retry`
    ///
    /// The `stripe-should-retry` value the response being stored actually
    /// carried, read off its own `HeaderMap` by the caller — never re-derived
    /// here from `status`. That is the whole point of the column (migration
    /// `0025`): the advisory comes from `Classify::retry` in exactly one place,
    /// and a replay re-emits what was sent rather than making a second decision
    /// about it (ADR-0011). `None` for a response that carried no advisory, and
    /// the database's `response_retry_is_an_advisory` CHECK refuses anything
    /// that is neither `"true"` nor `"false"` — so a caller inventing a third
    /// value fails loudly rather than storing a header nothing can read.
    ///
    /// # `response.subject`, and the one resource this store knows the name of
    ///
    /// [`ResponseSubject::Verbatim`] stores the body as given, in **one**
    /// statement — the shape every route but `/v1/customers` gets, and the
    /// reason the compare-and-swap above reads a row back only when it
    /// matched none.
    ///
    /// [`ResponseSubject::Customer`] adds a transaction around that statement
    /// and two things inside it: the payer's row is read under a **share
    /// lock**, and if that payer has since been erased the body this write
    /// just stored is put through `crate::customers`' own redaction before
    /// the transaction commits. The lock is the half that matters — without
    /// it the read could be true and the write could land on the far side of
    /// an erasure committing in between, which is the same race one statement
    /// smaller. Every erasure takes `FOR UPDATE` on that row first, so the
    /// two serialise: either the erasure goes first and this write sees it,
    /// or this write goes first and the erasure's own sweep finds the row.
    /// See the module comment for why the exception is here at all.
    ///
    /// # Errors
    ///
    /// [`DbError::WriteMatchedNoRow`] if this claim's row is already complete,
    /// and [`DbError::Query`] if any statement fails.
    async fn store(
        &self,
        merchant_id: &str,
        key: &str,
        claim_id: Uuid,
        response: StoredResponse<'_>,
    ) -> Result<IdempotencyStoreOutcome, DbError>;

    /// Hands back a key this process claimed but will not answer for, so the
    /// merchant's retry re-executes instead of being told "in progress".
    ///
    /// The counterpart to [`Idempotency::store`], for every outcome that must **not** become
    /// a replayable answer. Two exist:
    ///
    /// * a `5xx` — including the `501` an unimplemented rail produces. Freezing
    ///   that for 24 hours would answer every retry with a failure the
    ///   deployment may since have been fixed for;
    /// * a request that was refused before it did anything at all (a validation
    ///   `400` on a create), where the refusal depends on deployment
    ///   configuration that can legitimately change under the merchant.
    ///
    /// Deleting rather than marking is deliberate: the row's only purpose is to
    /// say "someone owns this key", so a released key must look exactly like one
    /// that was never claimed — and the next [`Idempotency::claim`] then takes it through the
    /// same `INSERT` path a first request would, with no second code path to get
    /// wrong.
    ///
    /// `AND state = 'in_flight'` is the safety: a caller that released a key
    /// another request had already *completed* would delete a stored response a
    /// merchant is entitled to replay. `AND claim_id = $3` is the other half of
    /// it: without that, a request whose expired claim was taken over would
    /// delete the *live* claim that replaced it, and two requests would then be
    /// free to run under one key. Zero rows deleted is therefore not an error —
    /// the key was already completed, already released, already swept, or is
    /// held by a later claim, and in each case there is nothing of this
    /// caller's to hand back.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the delete fails.
    async fn release(&self, merchant_id: &str, key: &str, claim_id: Uuid) -> Result<u64, DbError>;

    /// Deletes every key past its 24-hour window and reports how many went.
    ///
    /// Rows still `in_flight` are deleted too once expired: a request that
    /// claimed a key a day ago and never stored a response is a crashed one,
    /// and keeping its claim forever would lock that key out permanently.
    ///
    /// **This is not scheduled.** `vpay-server`'s boot calls it once per process
    /// start, next to the `oauth_client_assertion_jtis` sweep and for the same
    /// reason — the worker's job loop does not exist (`docs/status.md`), so
    /// nothing runs it on a timer. A long-lived process therefore lets this
    /// table grow between restarts. That growth is the *only* thing at stake:
    /// correctness does not depend on the sweep, because [`Idempotency::claim`] reclaims an
    /// expired row itself, so a key is never held past its window whether or not
    /// anything ever sweeps.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the delete fails.
    async fn sweep_expired(&self) -> Result<u64, DbError>;
}

/// The compare-and-swap that completes a claim, and the whole of what
/// [`Idempotency::store`] writes.
///
/// One function over an executor rather than one statement per arm of
/// [`ResponseSubject`]: the `AND claim_id = $3 AND state = 'in_flight'` is
/// the guard that stops a late writer overwriting a response a merchant has
/// already been given, and a second copy of it is a second chance to write it
/// differently.
///
/// # Errors
///
/// [`DbError::Query`] if the statement fails.
async fn complete_claim<'e, E>(
    executor: E,
    merchant_id: &str,
    key: &str,
    claim_id: Uuid,
    status: i16,
    body: &Value,
    retry: Option<&str>,
) -> Result<u64, DbError>
where
    E: sqlx::PgExecutor<'e>,
{
    let affected = sqlx::query(
        "UPDATE idempotency_keys \
         SET state = 'complete', response_status = $4, response_body = $5, \
             response_retry = $6, completed_at = now() \
         WHERE merchant_id = $1 AND idempotency_key = $2 AND claim_id = $3 \
           AND state = 'in_flight'",
    )
    .bind(merchant_id)
    .bind(key)
    .bind(claim_id)
    .bind(status)
    .bind(body)
    .bind(retry)
    .execute(executor)
    .await
    .map_err(DbError::Query)?
    .rows_affected();

    Ok(affected)
}

/// Tells a stale claim from a broken invariant, when [`complete_claim`]
/// matched no row.
///
/// Only on that path: the happy case must stay one statement. See
/// [`Idempotency::store`]'s own doc for why the two causes cannot be
/// collapsed.
///
/// # Errors
///
/// [`DbError::WriteMatchedNoRow`] if the row is still this claim's — which
/// means it is already `complete`, so the caller is completing one key twice
/// — and [`DbError::Query`] if the read fails.
async fn diagnose_empty_store<'e, E>(
    executor: E,
    merchant_id: &str,
    key: &str,
    claim_id: Uuid,
) -> Result<IdempotencyStoreOutcome, DbError>
where
    E: sqlx::PgExecutor<'e>,
{
    let current = sqlx::query_scalar::<_, Uuid>(
        "SELECT claim_id FROM idempotency_keys \
         WHERE merchant_id = $1 AND idempotency_key = $2",
    )
    .bind(merchant_id)
    .bind(key)
    .fetch_optional(executor)
    .await
    .map_err(DbError::Query)?;

    match current {
        // Still this claim's row, and the update still matched nothing: the
        // only remaining guard is `state`, so it is already complete.
        Some(current) if current == claim_id => Err(DbError::WriteMatchedNoRow {
            table: "idempotency_keys",
            key: format!("{merchant_id}/{key}"),
        }),
        _ => Ok(IdempotencyStoreOutcome::StaleClaim),
    }
}

#[async_trait::async_trait]
impl Idempotency for crate::repository::PgRepositories {
    async fn claim(
        &self,
        merchant_id: &str,
        key: &str,
        method: &str,
        path: &str,
        request_hash: &[u8; 32],
    ) -> Result<IdempotencyClaim, DbError> {
        let claimed = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO idempotency_keys \
             (merchant_id, idempotency_key, request_method, request_path, request_hash, state) \
         VALUES ($1, $2, $3, $4, $5, 'in_flight') \
         ON CONFLICT (merchant_id, idempotency_key) DO UPDATE \
             SET request_method = EXCLUDED.request_method, \
                 request_path = EXCLUDED.request_path, \
                 request_hash = EXCLUDED.request_hash, \
                 claim_id = gen_random_uuid(), \
                 state = 'in_flight', \
                 response_status = NULL, \
                 response_body = NULL, \
                 response_retry = NULL, \
                 completed_at = NULL, \
                 created_at = now(), \
                 expires_at = now() + INTERVAL '24 hours' \
             WHERE idempotency_keys.expires_at < now() \
         RETURNING claim_id",
        )
        .bind(merchant_id)
        .bind(key)
        .bind(method)
        .bind(path)
        .bind(request_hash.as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)?;

        // A row came back from either arm: this request inserted the key, or it
        // reclaimed an expired one. Both mean the same thing to the caller, and
        // both hand back the `claim_id` that arm minted.
        if let Some(claim_id) = claimed {
            return Ok(IdempotencyClaim::Fresh { claim_id });
        }

        let existing = sqlx::query_as::<_, IdempotencyRecord>(
            "SELECT request_hash, state, response_status, response_body, response_retry \
         FROM idempotency_keys \
         WHERE merchant_id = $1 AND idempotency_key = $2",
        )
        .bind(merchant_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)?;

        let Some(existing) = existing else {
            // Swept or released between the two statements; see the doc
            // comment above.
            return Ok(IdempotencyClaim::InFlight);
        };

        // All 32 bytes, every time — no early exit on the first differing byte.
        // `ConstantTimeEq` for slices also answers "not equal" for a length
        // mismatch, which the `request_hash_is_sha256` CHECK already prevents.
        if !bool::from(existing.request_hash.ct_eq(request_hash.as_slice())) {
            return Ok(IdempotencyClaim::Mismatch);
        }

        // Same request. Either it finished (replay its answer) or it is still
        // running (say so). The `complete_has_a_response` CHECK is what makes
        // "complete" imply the record can actually answer.
        if existing.state == "complete" {
            Ok(IdempotencyClaim::Replay(existing))
        } else {
            Ok(IdempotencyClaim::InFlight)
        }
    }

    async fn store(
        &self,
        merchant_id: &str,
        key: &str,
        claim_id: Uuid,
        response: StoredResponse<'_>,
    ) -> Result<IdempotencyStoreOutcome, DbError> {
        let StoredResponse {
            status,
            body,
            retry,
            subject,
        } = response;
        let Ok(status) = i16::try_from(status) else {
            return Err(DbError::WriteMatchedNoRow {
                table: "idempotency_keys",
                key: format!("{merchant_id}/{key} (status {status} is not a valid HTTP status)"),
            });
        };

        let ResponseSubject::Customer { id } = subject else {
            // One statement, on the pool, for every route that is not
            // `/v1/customers`.
            let affected =
                complete_claim(&self.pool, merchant_id, key, claim_id, status, body, retry).await?;
            if affected == 0 {
                return diagnose_empty_store(&self.pool, merchant_id, key, claim_id).await;
            }
            return Ok(IdempotencyStoreOutcome::Stored);
        };

        // The shaped exception. A transaction, because the question "has this
        // payer been erased?" is being asked in order to write, so the answer
        // has to still be true when the write lands — and the share lock is
        // what makes it so against the `FOR UPDATE` every erasure takes.
        let mut tx = self.pool.begin().await.map_err(DbError::Query)?;
        let erased = crate::customers::erased_under_share_lock(&mut tx, merchant_id, id).await?;
        let affected =
            complete_claim(&mut *tx, merchant_id, key, claim_id, status, body, retry).await?;

        if affected == 0 {
            // Nothing was written, so there is nothing to redact and no
            // reason to hold the payer's row: hand it back before the
            // diagnosis, which is a read this claim no longer owns anything
            // for.
            tx.rollback().await.map_err(DbError::Query)?;
            return diagnose_empty_store(&self.pool, merchant_id, key, claim_id).await;
        }

        if erased {
            // The row this transaction just wrote is a stored response for a
            // payer who is gone. `crate::customers`' own statement, so there
            // is one definition of what a redacted stored body is rather than
            // this module's opinion of it — scoped to `key`, because the row
            // this write created is the only one it can have put the payer
            // back into. See that function's `only_key` section.
            crate::customers::redact_stored_responses_in_tx(&mut tx, id, merchant_id, Some(key))
                .await?;
        }

        tx.commit().await.map_err(DbError::Query)?;
        Ok(IdempotencyStoreOutcome::Stored)
    }

    async fn release(&self, merchant_id: &str, key: &str, claim_id: Uuid) -> Result<u64, DbError> {
        let affected = sqlx::query(
            "DELETE FROM idempotency_keys \
         WHERE merchant_id = $1 AND idempotency_key = $2 AND claim_id = $3 \
           AND state = 'in_flight'",
        )
        .bind(merchant_id)
        .bind(key)
        .bind(claim_id)
        .execute(&self.pool)
        .await
        .map_err(DbError::Query)?
        .rows_affected();

        Ok(affected)
    }

    async fn sweep_expired(&self) -> Result<u64, DbError> {
        let affected = sqlx::query("DELETE FROM idempotency_keys WHERE expires_at < now()")
            .execute(&self.pool)
            .await
            .map_err(DbError::Query)?
            .rows_affected();

        Ok(affected)
    }
}
