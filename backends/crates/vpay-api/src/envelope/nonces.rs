//! The envelope layer's replay cache, in Postgres (VP-3).
//!
//! cratestack records `(kid, cti)` after a request's signature has verified and
//! refuses a second request with the same pair until `iat + 2·skew + 1`. Its
//! shipped stores are per process (memory) or need Redis, and vpay has several
//! replicas, two Deployments (ADR-0022) and no Redis, so the record is a table
//! — `envelope_nonces`, migration 0055 — claimed by one atomic statement
//! (`vpay_db::EnvelopeNonces`, whose module header has the argument for why it
//! is one statement and not a read followed by a write).
//!
//! **A failed claim is a refusal.** [`NonceStore::record_if_unseen`] returns
//! `Err` for a database that cannot answer, and the envelope turns that into a
//! `500`: the request is not opened, the handler does not run. A replay cache
//! that failed open would make a Postgres outage a replay window.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cratestack_core::{CratestackError, NonceStore};
use time::OffsetDateTime;
use vpay_db::Repositories;

/// [`NonceStore`] over [`Repositories`].
#[derive(Clone)]
pub struct PgNonceStore {
    repositories: Arc<dyn Repositories>,
}

impl std::fmt::Debug for PgNonceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgNonceStore").finish_non_exhaustive()
    }
}

impl PgNonceStore {
    /// A store over the process's repositories.
    #[must_use]
    pub fn new(repositories: Arc<dyn Repositories>) -> Self {
        Self { repositories }
    }
}

/// `chrono` (cratestack's type) to `time` (this workspace's), exact: both
/// count from the Unix epoch, and cratestack's expiry is whole seconds.
fn to_offset_date_time(instant: DateTime<Utc>) -> Result<OffsetDateTime, CratestackError> {
    OffsetDateTime::from_unix_timestamp(instant.timestamp())
        .map_err(|_| CratestackError::Internal("a nonce expiry outside the time range".to_owned()))?
        .replace_nanosecond(instant.timestamp_subsec_nanos().min(999_999_999))
        .map_err(|_| CratestackError::Internal("a nonce expiry with a bad nanosecond".to_owned()))
}

#[async_trait]
impl NonceStore for PgNonceStore {
    async fn record_if_unseen(
        &self,
        nonce: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<bool, CratestackError> {
        let expires_at = to_offset_date_time(expires_at)?;
        self.repositories
            .record_envelope_nonce_if_unseen(nonce, expires_at)
            .await
            .map_err(|error| {
                // The detail stays in the log: a `CratestackError::Internal`
                // is answered `500` with a canned body.
                tracing::error!(%error, "the envelope replay cache failed; refusing the request");
                CratestackError::Internal("the replay cache failed".to_owned())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_second_expiry_converts_exactly() {
        let expiry = DateTime::from_timestamp(1_800_000_001, 0).expect("a valid instant");
        let converted = to_offset_date_time(expiry).expect("converts");
        assert_eq!(converted.unix_timestamp(), 1_800_000_001);
        assert_eq!(converted.nanosecond(), 0);
    }

    #[test]
    fn a_leap_second_nanosecond_is_clamped_not_refused() {
        let leap = DateTime::from_timestamp(1_800_000_059, 1_500_000_000).expect("chrono's :60");
        let converted = to_offset_date_time(leap).expect("clamped");
        assert_eq!(converted.nanosecond(), 999_999_999);
    }
}
