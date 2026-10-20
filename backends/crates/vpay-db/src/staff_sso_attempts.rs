//! The `staff_sso_attempts` repository
//! (`backends/migrations/0050_create-staff-sso-attempts.sql`) — an SSO
//! sign-in in flight
//! ([ADR-0030](../../../../docs/adr/0030-staff-sign-in-federates-to-an-oidc-provider.md)).
//!
//! # Single use is a delete, not a flag
//!
//! [`StaffSsoAttempts::consume`] reads the row and then deletes it, and the
//! **delete's row count** is the answer to "did I win": two callbacks racing
//! on one `state` both read the row and exactly one deletes it. A `consumed_at`
//! column would need a second statement to clean up and would leave a spent
//! `state` in a table for the sweep to find; a deleted row is also the end of
//! the row's secrets (the sealed verifier) at the earliest possible moment.
//!
//! An **expired** row is consumed too — deleted, and reported as absent — so
//! "expired" and "never existed" are one answer to the caller, the uniformity
//! [`crate::staff_sessions`] keeps for sessions, but without that module's
//! reason to keep the row for an operator: nothing here is evidence worth
//! keeping, and it carries a sealed secret.
//!
//! # What this module does not know
//!
//! It never seals, never hashes and never compares. The caller hands it
//! digests and an already-sealed verifier, exactly as [`crate::credentials`]
//! is handed an argon2id string it cannot read.

use std::fmt;

use async_trait::async_trait;
use time::{Duration, OffsetDateTime};

use crate::error::DbError;
use crate::persistence::{classify_cratestack, system_context};
use crate::schema::cratestack_schema::{self, staff_sso_attempt};
use crate::staff::{from_chrono, to_chrono};

/// The `.cstack` model these calls name.
const MODEL: &str = "StaffSsoAttempt";

/// How long a pending SSO sign-in lives.
///
/// Ten minutes: ADR-0030. Long enough for a person to finish a login form at
/// the provider including an authenticator-app code, short enough that a
/// leaked `state` is a stale one almost at once. A product rule rather than a
/// security floor, so a constant and not configuration.
pub const SSO_ATTEMPT_LIFETIME: Duration = Duration::minutes(10);

/// One `staff_sso_attempts` row, as stored.
#[derive(Clone, PartialEq, Eq)]
pub struct SsoAttemptRow {
    /// The SHA-256 of the pending-flow token, hex.
    pub id: String,
    /// The SHA-256 of the `state` sent to the provider, hex.
    pub state_hash: String,
    /// The SHA-256 of the `nonce` sent to the provider, hex.
    pub nonce_hash: String,
    /// The PKCE verifier, **sealed**. Opaque here.
    pub code_verifier: String,
    /// Whether this flow is itself the single `prompt=login` retry.
    pub is_retry: bool,
    /// When it was created.
    pub created_at: OffsetDateTime,
    /// When it stops being redeemable.
    pub expires_at: OffsetDateTime,
}

/// Redacts the sealed verifier; the digests and instants are what an operator
/// debugging a stuck sign-in reads.
impl fmt::Debug for SsoAttemptRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SsoAttemptRow")
            .field("id", &self.id)
            .field("state_hash", &"[redacted]")
            .field("nonce_hash", &"[redacted]")
            .field("code_verifier", &"[redacted]")
            .field("is_retry", &self.is_retry)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Everything the `start` handler supplies for one pending sign-in.
#[derive(Clone, PartialEq, Eq)]
pub struct NewSsoAttempt {
    /// The SHA-256 of the pending-flow token, hex.
    pub id: String,
    /// The SHA-256 of the `state`, hex.
    pub state_hash: String,
    /// The SHA-256 of the `nonce`, hex.
    pub nonce_hash: String,
    /// The sealed PKCE verifier.
    pub code_verifier: String,
    /// Whether this flow is the `prompt=login` retry.
    pub is_retry: bool,
    /// When it is created. `expires_at` is derived from this and
    /// [`SSO_ATTEMPT_LIFETIME`] here, so the two cannot drift.
    pub now: OffsetDateTime,
}

/// Redacts for [`SsoAttemptRow`]'s reason.
impl fmt::Debug for NewSsoAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewSsoAttempt")
            .field("id", &self.id)
            .field("is_retry", &self.is_retry)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// Reads and writes of `staff_sso_attempts`. Every method through CrateStack.
#[async_trait]
pub trait StaffSsoAttempts {
    /// Records one pending sign-in.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] — [`crate::PersistenceError::Unique`] on a
    /// digest collision, [`crate::PersistenceError::Check`] on a bound,
    /// [`crate::PersistenceError::Denied`] if `model StaffSsoAttempt` loses
    /// its `@@allow("create", …)`.
    async fn create(&self, new: NewSsoAttempt) -> Result<(), DbError>;

    /// Takes the pending sign-in for a token digest **exactly once**.
    ///
    /// `None` for no such row, a row already taken, and an expired one — the
    /// caller refuses all three the same way. The row is gone afterwards in
    /// every case.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`]. A missing `@@allow("delete", …)` is
    /// **silent** (the policy is compiled into the `WHERE`): every callback
    /// would answer `None`, which fails closed.
    async fn consume(
        &self,
        id: &str,
        now: OffsetDateTime,
    ) -> Result<Option<SsoAttemptRow>, DbError>;

    /// Deletes every expired row and answers how many went.
    ///
    /// Called on every `start`: that route is unauthenticated and writes a
    /// row, so a table nothing swept would be one a caller fills. The cost is
    /// one indexed delete (`staff_sso_attempts_expires_idx`).
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn sweep_expired_sso_attempts(&self, now: OffsetDateTime) -> Result<usize, DbError>;
}

#[async_trait]
impl StaffSsoAttempts for crate::repository::PgRepositories {
    async fn create(&self, new: NewSsoAttempt) -> Result<(), DbError> {
        self.cs
            .staff_sso_attempt()
            .create(cratestack_schema::CreateStaffSsoAttemptInput {
                id: new.id,
                state_hash: new.state_hash,
                nonce_hash: new.nonce_hash,
                code_verifier: new.code_verifier,
                is_retry: new.is_retry,
                created_at: to_chrono(new.now),
                expires_at: to_chrono(new.now.saturating_add(SSO_ATTEMPT_LIFETIME)),
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "create", error)))?;

        Ok(())
    }

    async fn consume(
        &self,
        id: &str,
        now: OffsetDateTime,
    ) -> Result<Option<SsoAttemptRow>, DbError> {
        let Some(model) = self
            .cs
            .staff_sso_attempt()
            .find_unique(id.to_owned())
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))?
        else {
            return Ok(None);
        };

        // THE COMPARE-AND-SWAP. Two callbacks that both read the row race
        // here, and `ok == 1` is true for exactly one of them.
        let summary = self
            .cs
            .staff_sso_attempt()
            .delete_many()
            .where_(staff_sso_attempt::id().eq(id.to_owned()))
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "delete", error)))?;
        if summary.ok != 1 {
            return Ok(None);
        }

        let row = row_from_model(model);
        Ok((now < row.expires_at).then_some(row))
    }

    async fn sweep_expired_sso_attempts(&self, now: OffsetDateTime) -> Result<usize, DbError> {
        let summary = self
            .cs
            .staff_sso_attempt()
            .delete_many()
            .where_(staff_sso_attempt::expires_at().lt(to_chrono(now)))
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "delete", error)))?;

        Ok(summary.ok)
    }
}

/// The generated model row, in vpay's own types.
fn row_from_model(model: cratestack_schema::models::StaffSsoAttempt) -> SsoAttemptRow {
    SsoAttemptRow {
        id: model.id,
        state_hash: model.state_hash,
        nonce_hash: model.nonce_hash,
        code_verifier: model.code_verifier,
        is_retry: model.is_retry,
        created_at: from_chrono(model.created_at),
        expires_at: from_chrono(model.expires_at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `@@allow` slot this module's calls need is occupied — asked of
    /// the descriptor, so it needs no container. `read` and `delete` fail
    /// silently when empty (`consume` would answer `None` for every flow, so
    /// nobody could sign in through SSO); `create` fails loudly.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use cratestack_schema::models::STAFF_SSO_ATTEMPT_MODEL as descriptor;

        assert!(
            !descriptor.read_allow_policies.is_empty(),
            "model StaffSsoAttempt lost @@allow(\"read\", …): consume would find nothing"
        );
        assert!(
            !descriptor.create_allow_policies.is_empty(),
            "model StaffSsoAttempt lost @@allow(\"create\", …): start would fail with a Forbidden"
        );
        assert!(
            !descriptor.delete_allow_policies.is_empty(),
            "model StaffSsoAttempt lost @@allow(\"delete\", …): its policy compiles into the \
             statement's WHERE, so consume would match zero rows and answer None for every flow \
             — and nothing would ever be swept"
        );
        assert!(
            descriptor.read_deny_policies.is_empty()
                && descriptor.create_deny_policies.is_empty()
                && descriptor.update_deny_policies.is_empty()
                && descriptor.delete_deny_policies.is_empty(),
            "a @@deny arm appeared on model StaffSsoAttempt; every deny is evaluated ahead of \
             every allow, so one added here refuses the system principal this crate runs as"
        );
    }

    #[test]
    fn the_sealed_verifier_never_reaches_debug() {
        let row = SsoAttemptRow {
            id: "a".repeat(64),
            state_hash: "b".repeat(64),
            nonce_hash: "c".repeat(64),
            code_verifier: "sealed-verifier-bytes".to_owned(),
            is_retry: false,
            created_at: OffsetDateTime::UNIX_EPOCH,
            expires_at: OffsetDateTime::UNIX_EPOCH,
        };
        let printed = format!("{row:?}");
        assert!(!printed.contains("sealed-verifier-bytes"), "{printed}");
        assert!(!printed.contains(&"b".repeat(64)), "{printed}");
    }

    #[test]
    fn a_pending_sign_in_lives_ten_minutes() {
        assert_eq!(SSO_ATTEMPT_LIFETIME, Duration::minutes(10));
    }
}
