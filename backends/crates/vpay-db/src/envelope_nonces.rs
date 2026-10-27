//! The `envelope_nonces` repository
//! (`backends/migrations/0055_create-envelope-nonces.sql`) — the replay cache
//! of the signed `/v1` transport (VP-3).
//!
//! cratestack's `CoseEnvelope` records `(kid, cti)` after a request's signature
//! has verified and refuses a second request with the same pair until
//! `iat + 2·skew + 1`. This is the store it records into, shared by every
//! replica and by both Deployments of ADR-0022, because the two it ships with
//! (in memory, Redis) are per process or need a Redis vpay does not run. The
//! consumer is `vpay_api::envelope::PgNonceStore`; this module is the SQL.
//!
//! # One statement, and why it is not a check-then-insert
//!
//! [`EnvelopeNonces::record_envelope_nonce_if_unseen`] is a single `INSERT … ON
//! CONFLICT (id) DO UPDATE … WHERE expires_at < now() RETURNING id`. Postgres
//! takes the row lock on a conflicting row before it evaluates the `WHERE`, so
//! two replicas that open the same captured request at the same instant are
//! serialised by the database: one gets a row back, the other none. A
//! `SELECT` followed by an `INSERT` would let both observe "unseen".
//!
//! The `DO UPDATE … WHERE` arm exists for one case and is not decoration: a row
//! whose window has closed and which the sweep has not reached yet is *taken
//! over*, not treated as a replay. cratestack's `NonceStore` contract says
//! entries past `expires_at` must stop counting, and a counter-shaped `cti` (1
//! to 4 bytes, which the envelope allows a client to use) would otherwise be
//! refused as a replay for up to an hour after its previous use expired. A row
//! whose window is still open is untouched, which is the whole of replay
//! protection.
//!
//! The comparison reads the **database's** clock, as every other scheduling
//! decision in this crate does (ADR-0026): `expires_at` is a fact computed from
//! the request's `iat`, and the moment it elapses is the database's to say.
//!
//! # What is NOT here
//!
//! No read. Nothing asks "has this nonce been seen" except the act of
//! recording it, and a read-only accessor would invite the check-then-insert
//! this module refuses.

use std::time::Duration;

use async_trait::async_trait;
use time::OffsetDateTime;

use crate::error::{DbError, classify_write};

/// The replay cache of the signed `/v1` transport.
#[async_trait]
pub trait EnvelopeNonces: Send + Sync {
    /// Records `nonce` as spent until `expires_at`, and says whether this was
    /// its first use: `true` if so, `false` if it is a replay.
    ///
    /// `nonce` is cratestack's key, `cose:<hex kid>:<hex cti>`; the table
    /// refuses one longer than 64 characters. `expires_at` is when the pair
    /// stops being replayable (the request's `iat` plus twice the skew plus a
    /// second).
    ///
    /// # Errors
    ///
    /// [`DbError`] if the statement fails. A caller **must** refuse the request
    /// on `Err`, never admit it: a replay cache that fails open is a replay
    /// window an outage can open.
    async fn record_envelope_nonce_if_unseen(
        &self,
        nonce: &str,
        expires_at: OffsetDateTime,
    ) -> Result<bool, DbError>;

    /// Deletes every nonce whose window closed more than `retain_after_expiry`
    /// ago, **on the database's clock**, returning how many rows went.
    ///
    /// Takes a relative duration and no instant, so the caller's clock cannot
    /// reach the comparison (ADR-0026 D1). Correctness does not depend on this
    /// running — an expired row is taken over by the next statement that names
    /// it — it only bounds the table, so a failed sweep is retried by the job
    /// and is never a reason to refuse traffic.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the delete fails.
    async fn delete_expired_envelope_nonces(
        &self,
        retain_after_expiry: Duration,
    ) -> Result<u64, DbError>;
}

/// The claim: first use returns the row, a live duplicate returns nothing.
///
/// A `const &'static str` so [`crate::sql_audit`] has nothing to audit. The
/// shape is pinned by `the_claim_is_one_atomic_upsert_that_spares_a_live_row`.
pub(crate) const CLAIM_ENVELOPE_NONCE: &str = "INSERT INTO envelope_nonces (id, expires_at) \
             VALUES ($1, $2) \
             ON CONFLICT (id) DO UPDATE SET expires_at = EXCLUDED.expires_at \
             WHERE envelope_nonces.expires_at < now() \
             RETURNING id";

#[async_trait]
impl EnvelopeNonces for crate::repository::PgRepositories {
    async fn record_envelope_nonce_if_unseen(
        &self,
        nonce: &str,
        expires_at: OffsetDateTime,
    ) -> Result<bool, DbError> {
        let claimed = sqlx::query_scalar::<_, String>(CLAIM_ENVELOPE_NONCE)
            .bind(nonce)
            .bind(expires_at)
            .fetch_optional(&self.pool)
            .await
            .map_err(classify_write)?;
        Ok(claimed.is_some())
    }

    async fn delete_expired_envelope_nonces(
        &self,
        retain_after_expiry: Duration,
    ) -> Result<u64, DbError> {
        let result = sqlx::query(
            "DELETE FROM envelope_nonces \
             WHERE expires_at < now() - ($1::BIGINT * INTERVAL '1 microsecond')",
        )
        .bind(crate::jobs::as_micros(retain_after_expiry))
        .execute(&self.pool)
        .await
        .map_err(DbError::Query)?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `model EnvelopeNonce` carries **no `@@allow` arm**, and this pins the
    /// absence, in the direction that matters: an arm appearing would be a
    /// standing permission over rows that decide whether a captured request
    /// is accepted a second time, with no caller to have reviewed it.
    #[test]
    fn the_envelope_nonce_model_answers_no_rows_to_every_action() {
        use crate::schema::cratestack_schema::models::ENVELOPE_NONCE_MODEL as descriptor;

        for (action, policies) in [
            ("read", descriptor.read_allow_policies),
            ("create", descriptor.create_allow_policies),
            ("update", descriptor.update_allow_policies),
            ("delete", descriptor.delete_allow_policies),
        ] {
            assert!(
                policies.is_empty(),
                "model EnvelopeNonce grew an @@allow(\"{action}\", …) arm. Nothing queries this \
                 model through the generated layer — the claim is one hand-written statement \
                 whose conflict arm carries a predicate. If a query moved, move this assertion \
                 with it"
            );
        }
    }

    /// The claim keeps each property that makes it a replay guard. Each is a
    /// mutation someone could plausibly make while tidying, and a text test is
    /// the only thing that sees it (there is no type for "this is atomic").
    #[test]
    fn the_claim_is_one_atomic_upsert_that_spares_a_live_row() {
        for (fragment, why) in [
            (
                "INSERT INTO envelope_nonces",
                "the insert is the guard; a SELECT first is a race",
            ),
            (
                "ON CONFLICT (id) DO UPDATE",
                "the conflict arm is what takes the row lock two replicas serialise on",
            ),
            (
                "WHERE envelope_nonces.expires_at < now()",
                "without this predicate a live nonce is overwritten and a replay is admitted",
            ),
            (
                "RETURNING id",
                "the returned row is the only signal that this was the first use",
            ),
        ] {
            assert!(
                CLAIM_ENVELOPE_NONCE.contains(fragment),
                "the claim lost `{fragment}`: {why}"
            );
        }
        assert_eq!(
            CLAIM_ENVELOPE_NONCE.matches(';').count(),
            0,
            "one statement, nothing batched"
        );
    }
}
