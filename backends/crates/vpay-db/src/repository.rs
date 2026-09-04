//! The repository seam: what a consumer of this crate is allowed to name.
//!
//! Every table family exposes one `#[async_trait]` trait
//! ([`crate::charges::Charges`], [`crate::jobs::Jobs`], …) whose methods are
//! the queries that family owns. [`Repositories`] is the umbrella every
//! consumer holds — `&dyn Repositories` in `vpay-api`'s router state and in
//! every `vpay-worker` handler — and [`PgRepositories`] is its only
//! implementation, built by [`crate::connect`].
//!
//! [`UnitOfWork::transaction`] hands a closure a `&mut dyn TxRepositories` and
//! decides `COMMIT`/`ROLLBACK` from what it returns, so "forgot to commit" is
//! not expressible and no `sqlx` type leaves this crate. [`TxOutcome`] is
//! there because a *successful* closure has two endings.
//!
//! `docs/reference/vpay-db.md` §"The repository seam" carries the reasoning:
//! why `dyn` and not a generic parameter, why a closure and not a handle, and
//! why [`PendingTransaction`] owns its transaction rather than borrowing
//! one.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use crate::charges::Charges;
use crate::checkout_sessions::CheckoutSessions;
use crate::client_assertion::ClientAssertions;
use crate::config_reconcile::ConfigReconcile;
use crate::disabled_clients::DisabledClients;
use crate::error::DbError;
use crate::events::Events;
use crate::health::Health;
use crate::idempotency::Idempotency;
use crate::jobs::Jobs;
use crate::migrations::Migrations;
use crate::payment_intents::PaymentIntents;
use crate::provider_requests::ProviderRequests;
use crate::settlement::Settlement;
use crate::signing_keys::SigningKeys;
use crate::webhook_deliveries::WebhookDeliveries;

/// A future that borrows the transaction it was handed, boxed so the closure
/// [`UnitOfWork::transaction`] takes can be written inline.
pub type TxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// How a transaction closure ended when it did **not** fail.
///
/// The value travels out of the closure either way — [`TxOutcome::Abandon`]
/// is how a caller returns what it learned from a transaction it then rolled
/// back, which is what makes the confirm path's "re-read on the plain pool"
/// recovery expressible without holding a `sqlx` handle across the decision.
///
/// ```
/// use vpay_db::TxOutcome;
///
/// // Both endings carry a value out, and it is the same value either way:
/// // a lost race still knows how many rows it built before it lost.
/// assert_eq!(TxOutcome::Commit(2_usize).into_inner(), 2);
/// assert_eq!(TxOutcome::Abandon(2_usize).into_inner(), 2);
///
/// // They are not interchangeable, which is the point: only one of them
/// // commits, and neither is an error.
/// assert_ne!(TxOutcome::Commit(2_usize), TxOutcome::Abandon(2));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxOutcome<T> {
    /// Commit, then hand this value back.
    Commit(T),
    /// Roll back, then hand this value back. Not an error.
    Abandon(T),
}

impl<T> TxOutcome<T> {
    /// The value, whichever ending happened.
    ///
    /// For the callers that only ever return [`TxOutcome::Commit`] and would
    /// otherwise write a `match` with an unreachable arm — which ADR-0007
    /// forbids expressing as a `panic!`.
    ///
    /// ```
    /// use vpay_db::TxOutcome;
    ///
    /// let committed: TxOutcome<&str> = TxOutcome::Commit("pi_1");
    /// assert_eq!(committed.into_inner(), "pi_1");
    /// ```
    pub fn into_inner(self) -> T {
        match self {
            Self::Commit(value) | Self::Abandon(value) => value,
        }
    }
}

/// An open transaction: the only [`TxRepositories`], and opaque on purpose.
///
/// It carries no public method beyond the trait, so a caller outside this
/// crate can obtain one from [`TransactionSource::begin_transaction`] and do
/// nothing with it but hand it back — which is what makes `PgRepositories`
/// the only usable implementation of [`TransactionSource`], and therefore of
/// [`Repositories`], without a sealed-trait dance. Dropping it rolls back.
///
/// It **owns** its `sqlx` transaction rather than borrowing one, and that is
/// load-bearing rather than incidental: `PendingTransaction: 'static` is what
/// lets [`UnitOfWork::transaction`]'s closure be spelled
/// `for<'t> FnOnce(&'t mut (dyn TxRepositories + 'a)) -> TxFuture<'t, _>`.
/// The `'a` on the trait object is well-formed only when `'a: 't`, and that
/// implied bound is the whole reason a closure may borrow the caller's locals
/// (`&NewCharge`, a `&str` merchant id) across the `.await`. With a borrowing
/// `PgTransaction<'t>` the same signature forces every capture to be
/// `'static`, which no call site in this workspace can satisfy.
pub struct PendingTransaction(Transaction<'static, Postgres>);

impl std::fmt::Debug for PendingTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PendingTransaction")
    }
}

impl PendingTransaction {
    fn conn(&mut self) -> &mut PgConnection {
        &mut self.0
    }

    pub(crate) async fn commit(self) -> Result<(), DbError> {
        self.0.commit().await.map_err(DbError::Query)
    }

    pub(crate) async fn rollback(self) -> Result<(), DbError> {
        self.0.rollback().await.map_err(DbError::Query)
    }
}

/// The queries that must run inside a caller's transaction, and only there.
///
/// One flat trait rather than one per table, because a transaction is
/// exactly the place the table boundaries stop mattering: every method here
/// exists because it has to commit beside one of the others. The names are
/// the free functions' own, unchanged, so a `git log -S` on a query still
/// finds its callers.
#[async_trait]
pub trait TxRepositories: Send {
    /// `charges`: opens a charge. See [`crate::charges`] for why the unique
    /// index, and not a preceding `SELECT`, is what enforces one charge per
    /// intent.
    ///
    /// # Errors
    ///
    /// [`DbError::UniqueViolation`] naming `one_charge_per_intent` when the
    /// intent already has one; [`DbError::Query`] otherwise.
    async fn insert_for_intent(
        &mut self,
        new: &crate::NewCharge,
    ) -> Result<crate::ChargeRow, DbError>;

    /// `charges`: records what the rail accepted — a compare-and-swap out of
    /// `submitting`.
    ///
    /// # Errors
    ///
    /// [`DbError::WriteMatchedNoRow`] if the charge had already left
    /// `submitting`; [`DbError::Query`] otherwise.
    async fn mark_submitted(
        &mut self,
        id: &str,
        state: &str,
        provider_ref_extra: Option<&serde_json::Value>,
        redirect_url: Option<&str>,
    ) -> Result<crate::ChargeRow, DbError>;

    /// `charges`: records a rail's decline — a compare-and-swap out of
    /// `submitting`.
    ///
    /// # Errors
    ///
    /// As [`TxRepositories::mark_submitted`].
    async fn mark_failed(
        &mut self,
        id: &str,
        failure_code: &str,
        failure_raw: &str,
    ) -> Result<crate::ChargeRow, DbError>;

    /// `events`: appends the outbox row that a merchant is eventually told
    /// about, in the same transaction as the state change it describes.
    ///
    /// # Errors
    ///
    /// [`DbError::UniqueViolation`] on a replayed `event_id`;
    /// [`DbError::Query`] otherwise.
    async fn insert_in_tx(&mut self, new: &crate::NewEvent) -> Result<crate::EventRow, DbError>;

    /// `jobs`: enqueues work, `ON CONFLICT (dedupe_key) DO NOTHING`.
    ///
    /// `false` means the key was already taken and nothing was written,
    /// which is a normal outcome and never an error.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or a CHECK violation for an unknown `kind`.
    async fn enqueue_in_tx(
        &mut self,
        kind: &str,
        dedupe_key: &str,
        payload: &serde_json::Value,
        run_at: time::OffsetDateTime,
    ) -> Result<bool, DbError>;

    /// `jobs`: brings an already-queued job's `run_at` forward to now, so a
    /// rail's callback is answered by a status query now rather than at the
    /// poll ladder's next rung.
    ///
    /// `false` means nothing moved — the job was due within `floor` (which
    /// includes "already claimable"), a worker holds its lease, or it is
    /// parked. All three are normal, and [`crate::jobs::pull_forward_in_tx`]
    /// says why each is refused and why the floor is the caller's number
    /// rather than this crate's.
    ///
    /// Transactional-only for [`TxRepositories::enqueue_in_tx`]'s reason:
    /// its one caller enqueues *and* pulls forward, and the two must reach
    /// the same commit or a callback can leave a job it created at a
    /// `run_at` it never moved.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    async fn pull_forward_in_tx(
        &mut self,
        dedupe_key: &str,
        floor: std::time::Duration,
    ) -> Result<bool, DbError>;

    /// `payment_intents`: the merchant-scoped compare-and-swap on `status`.
    ///
    /// `Ok(None)` means the intent was not in `expected` — a lost race, not
    /// a failure.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    async fn transition_in_tx(
        &mut self,
        merchant_id: &str,
        id: &str,
        expected: &str,
        new: &str,
    ) -> Result<Option<crate::PaymentIntentRow>, DbError>;

    /// `payment_intents`: stamps `last_payment_error` without moving the
    /// status the intent never left.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    async fn record_payment_error(
        &mut self,
        merchant_id: &str,
        id: &str,
        expected: &str,
        code: &str,
        message: &str,
    ) -> Result<Option<crate::PaymentIntentRow>, DbError>;

    /// `webhook_deliveries`: opens one delivery of one event to one
    /// endpoint. `Ok(None)` means another drain already created it.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    async fn create_in_tx(
        &mut self,
        event_id: &str,
        endpoint_id: &str,
        url: &str,
    ) -> Result<Option<uuid::Uuid>, DbError>;

    /// `events`: the compare-and-swap that closes a fan-out. `false` means
    /// another drain claimed the event first and this transaction must be
    /// abandoned — see [`TxOutcome::Abandon`].
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    async fn mark_fanned_out_in_tx(&mut self, event_id: &str) -> Result<bool, DbError>;
}

#[async_trait]
impl TxRepositories for PendingTransaction {
    async fn insert_for_intent(
        &mut self,
        new: &crate::NewCharge,
    ) -> Result<crate::ChargeRow, DbError> {
        crate::charges::insert_for_intent(self.conn(), new).await
    }

    async fn mark_submitted(
        &mut self,
        id: &str,
        state: &str,
        provider_ref_extra: Option<&serde_json::Value>,
        redirect_url: Option<&str>,
    ) -> Result<crate::ChargeRow, DbError> {
        crate::charges::mark_submitted(self.conn(), id, state, provider_ref_extra, redirect_url)
            .await
    }

    async fn mark_failed(
        &mut self,
        id: &str,
        failure_code: &str,
        failure_raw: &str,
    ) -> Result<crate::ChargeRow, DbError> {
        crate::charges::mark_failed(self.conn(), id, failure_code, failure_raw).await
    }

    async fn insert_in_tx(&mut self, new: &crate::NewEvent) -> Result<crate::EventRow, DbError> {
        crate::events::insert_in_tx(self.conn(), new).await
    }

    async fn enqueue_in_tx(
        &mut self,
        kind: &str,
        dedupe_key: &str,
        payload: &serde_json::Value,
        run_at: time::OffsetDateTime,
    ) -> Result<bool, DbError> {
        crate::jobs::enqueue_in_tx(self.conn(), kind, dedupe_key, payload, run_at).await
    }

    async fn pull_forward_in_tx(
        &mut self,
        dedupe_key: &str,
        floor: std::time::Duration,
    ) -> Result<bool, DbError> {
        crate::jobs::pull_forward_in_tx(self.conn(), dedupe_key, floor).await
    }

    async fn transition_in_tx(
        &mut self,
        merchant_id: &str,
        id: &str,
        expected: &str,
        new: &str,
    ) -> Result<Option<crate::PaymentIntentRow>, DbError> {
        crate::payment_intents::transition_in_tx(self.conn(), merchant_id, id, expected, new).await
    }

    async fn record_payment_error(
        &mut self,
        merchant_id: &str,
        id: &str,
        expected: &str,
        code: &str,
        message: &str,
    ) -> Result<Option<crate::PaymentIntentRow>, DbError> {
        crate::payment_intents::record_payment_error(
            self.conn(),
            merchant_id,
            id,
            expected,
            code,
            message,
        )
        .await
    }

    async fn create_in_tx(
        &mut self,
        event_id: &str,
        endpoint_id: &str,
        url: &str,
    ) -> Result<Option<uuid::Uuid>, DbError> {
        crate::webhook_deliveries::create_in_tx(self.conn(), event_id, endpoint_id, url).await
    }

    async fn mark_fanned_out_in_tx(&mut self, event_id: &str) -> Result<bool, DbError> {
        crate::webhook_deliveries::mark_fanned_out_in_tx(self.conn(), event_id).await
    }
}

/// Where a transaction comes from.
///
/// Object-safe, so [`Repositories`] can be a trait object; the useful method
/// is [`UnitOfWork::transaction`], which is generic over the closure's return
/// type and therefore cannot live here.
#[async_trait]
pub trait TransactionSource: Send + Sync {
    /// Opens a transaction. Of no use to a caller outside this crate — see
    /// [`PendingTransaction`].
    #[doc(hidden)]
    async fn begin_transaction(&self) -> Result<PendingTransaction, DbError>;
}

/// Run a closure inside one transaction, and let its return value decide
/// whether that transaction commits.
///
/// ```text
/// let outcome = repositories
///     .transaction(|tx| Box::pin(async move {
///         let charge = tx.insert_for_intent(&new).await?;
///         Ok(TxOutcome::Commit(charge))
///     }))
///     .await?;
/// ```
///
/// An `Err` from the closure rolls back — by dropping the transaction, as
/// the hand-written `pool.begin()` sites this replaced did, so the original
/// error is what the caller sees rather than a rollback failure on top of
/// it. [`TxOutcome::Abandon`] rolls back explicitly and for the same reason
/// swallows a failure to do so: see the arm.
///
/// # Why the error type is a parameter and not [`DbError`]
///
/// Three of the transactions this replaced raise their layer's own error
/// from inside the unit of work — `vpay_api::ApiError` for the confirm
/// path's "the rail accepted a charge whose intent moved" invariant, and
/// `vpay_worker::JobError` for a payload that will not encode. Pinning the
/// closure to `DbError` would force each of them either to smuggle its error
/// out through the success channel or to mislabel it as storage, and
/// ADR-0011 exists to stop exactly that. `E: From<DbError>` keeps the
/// simple case (`E = DbError`) spelled the same way.
#[async_trait]
pub trait UnitOfWork {
    /// See the trait.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the transaction cannot be opened or committed,
    /// and whatever the closure returns.
    async fn transaction<'a, T, E, F>(&self, f: F) -> Result<TxOutcome<T>, E>
    where
        T: Send,
        E: From<DbError> + Send,
        F: for<'t> FnOnce(
                &'t mut (dyn TxRepositories + 'a),
            ) -> TxFuture<'t, Result<TxOutcome<T>, E>>
            + Send;
}

#[async_trait]
impl<S: TransactionSource + ?Sized> UnitOfWork for S {
    async fn transaction<'a, T, E, F>(&self, f: F) -> Result<TxOutcome<T>, E>
    where
        T: Send,
        E: From<DbError> + Send,
        F: for<'t> FnOnce(
                &'t mut (dyn TxRepositories + 'a),
            ) -> TxFuture<'t, Result<TxOutcome<T>, E>>
            + Send,
    {
        let mut pending = self.begin_transaction().await?;
        let outcome = f(&mut pending).await;
        match outcome {
            // Dropped rather than rolled back explicitly: that is what the
            // `?`-on-a-held-`Transaction` sites this replaced did, and an
            // awaited rollback here could fail and mask the real error.
            Err(error) => Err(error),
            Ok(TxOutcome::Commit(value)) => {
                pending.commit().await?;
                Ok(TxOutcome::Commit(value))
            }
            // A rollback that fails is logged and swallowed, and the value
            // still comes back. `ROLLBACK` is best-effort by construction:
            // the transaction is aborted whether the statement lands or the
            // connection dies before it does, so a failure here changes
            // nothing about the database — only about what the caller is
            // allowed to say. Returning it would replace the caller's own
            // answer with a storage error, and both abandoning call sites
            // have an answer that must survive: the confirm path's
            // duplicate-charge recovery owes the merchant its `409`, and
            // `persist_submitted` owes an operator the `Internal` alert
            // saying a rail may hold a live payment. Turning either into a
            // `503` loses the only report of it.
            Ok(TxOutcome::Abandon(value)) => {
                if let Err(error) = pending.rollback().await {
                    tracing::warn!(
                        %error,
                        "rolling back an abandoned transaction failed; it is aborted either \
                         way and the caller's own answer stands"
                    );
                }
                Ok(TxOutcome::Abandon(value))
            }
        }
    }
}

/// Everything a consumer of this crate can ask Postgres to do.
///
/// Held as `Arc<dyn Repositories>` by both binaries, by `vpay-api`'s router
/// state and by every `vpay-worker` handler. One umbrella rather than a
/// dozen parameters: a handler that needs `jobs` and `events` names one
/// value, and the *method* it calls says which table it touched.
pub trait Repositories:
    Charges
    + CheckoutSessions
    + ClientAssertions
    + ConfigReconcile
    + DisabledClients
    + Events
    + Health
    + Idempotency
    + Jobs
    + Migrations
    + PaymentIntents
    + ProviderRequests
    + Settlement
    + SigningKeys
    + TransactionSource
    + WebhookDeliveries
    + std::fmt::Debug
    + Send
    + Sync
    + 'static
{
    /// The one place a raw `sqlx` pool still leaves this crate.
    ///
    /// `authkestra_op::sqlx_store::SqlxOpStore<sqlx::Postgres>` (ADR-0010)
    /// is a *foreign* trait implementation over a pool: vpay does not own
    /// its queries and cannot express them as repository methods. Step 7's
    /// decision (9) exempts it from the repository split rather than
    /// abstracting a subsystem this step was never asked to touch — see
    /// `docs/status.md`.
    ///
    /// Not a general escape hatch: a new `sqlx::query!` behind this call is
    /// a repository method that was not written.
    fn op_store_pool(&self) -> PgPool;
}

/// The only [`Repositories`].
///
/// Private: [`crate::connect`] is the only way to obtain one, so there is
/// exactly one implementation in the workspace and no test double can be
/// substituted for it (ADR-0006 — the suites that exercise this crate run
/// against a real Postgres container).
pub(crate) struct PgRepositories {
    pub(crate) pool: PgPool,
}

impl std::fmt::Debug for PgRepositories {
    /// Names the type and nothing else. `PgPool`'s own `Debug` prints no
    /// connection string, but `RouterDeps` derives `Debug` over this and a
    /// pool's internal state is noise in a startup log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PgRepositories")
    }
}

impl PgRepositories {
    /// The one constructor, called only by [`crate::connect`].
    ///
    /// Returns the trait object rather than `Self` so the concrete type has
    /// no way out of this crate: `connect` is the whole public surface.
    pub(crate) fn boxed(pool: PgPool) -> Arc<dyn Repositories> {
        Arc::new(Self { pool })
    }
}

#[async_trait]
impl TransactionSource for PgRepositories {
    async fn begin_transaction(&self) -> Result<PendingTransaction, DbError> {
        self.pool
            .begin()
            .await
            .map(PendingTransaction)
            .map_err(DbError::Query)
    }
}

impl Repositories for PgRepositories {
    fn op_store_pool(&self) -> PgPool {
        self.pool.clone()
    }
}

#[cfg(test)]
mod closure_shape {
    //! A compile-only guard on [`UnitOfWork::transaction`]'s signature.
    //!
    //! Nothing here runs; it exists because the one thing that signature can
    //! silently lose is the ability for a closure to *borrow the caller's
    //! locals* across the `.await` — the difference between
    //! `dyn TxRepositories + 'a` and `dyn TxRepositories + 't`, which no
    //! runtime test can tell apart because the second one does not compile at
    //! any call site. `new: &NewCharge` below is that borrow.

    use super::*;

    #[expect(dead_code, reason = "compiled, never called: see the module doc")]
    async fn borrows_a_caller_local(
        repositories: &dyn Repositories,
        new: &crate::NewCharge,
    ) -> Result<(), DbError> {
        let outcome = repositories
            .transaction(|tx| {
                Box::pin(async move {
                    let charge = tx.insert_for_intent(new).await?;
                    let enqueued = tx
                        .enqueue_in_tx(
                            "poll_charge",
                            "poll:ch_1",
                            &serde_json::json!({}),
                            time::OffsetDateTime::now_utc(),
                        )
                        .await?;
                    if enqueued {
                        Ok(TxOutcome::Commit(charge))
                    } else {
                        Ok(TxOutcome::Abandon(charge))
                    }
                })
            })
            .await?;
        let _ = outcome.into_inner();
        Ok(())
    }
}
