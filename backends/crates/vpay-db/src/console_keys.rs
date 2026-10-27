//! The `console_keys` repository (`backends/migrations/0056_console-keys.sql`) —
//! a person's registered console signing key (VP-6a,
//! [ADR-0036](../../../../docs/adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)).
//!
//! # What this module is, and the three things it is not
//!
//! It stores a P-256 public key, finds the live ones for a signature's `kid`,
//! and revokes one. It **never parses** a key (`vpay_api::console_keys` does,
//! with `cratestack-cose`, and hands it the base64url text and the thumbprint
//! it computed), it **never decides who may pair** (the route and the command
//! line do), and it **never reads a clock of its own**: every instant is an
//! argument, so the 30-day life is a property a test can walk across by
//! pairing in the past.
//!
//! # Life of a key
//!
//! ```text
//!   pair ──▶ live ──(720 h after paired_at)──▶ expired   never extended, never deleted
//!             │
//!             └──(revoke)──▶ revoked                      revoked_at keeps its first value
//! ```
//!
//! Expiry is data, not a job: a key is live iff `revoked_at IS NULL` **and**
//! `expires_at > now`. [`ConsoleKeys::live_console_keys_for_kid`] is the one
//! read the signed transport makes, on **every** request, with no cache — so a
//! revocation takes effect on the next request and an expired key stops
//! authenticating at the instant it expires. A cache here would be the
//! window the owner's "revocation takes effect on the next request" rules out.
//!
//! # Why the row is never deleted
//!
//! A revoked or expired thumbprint must stay taken: `console_keys_thumbprint_key`
//! then refuses the same key a second time, so a stolen browser profile's key
//! cannot be re-registered after the person revoked it. There is no `delete`
//! method and no `@@allow("delete")` arm.

use std::fmt;

use async_trait::async_trait;
use time::{Duration, OffsetDateTime};

use crate::error::DbError;
use crate::persistence::{classify_cratestack, system_context};
use crate::schema::cratestack_schema::{self, console_key};
use crate::staff::{from_chrono, to_chrono};

/// The `.cstack` model these calls name.
const MODEL: &str = "ConsoleKey";

/// The stored spelling of COSE `ESP256`, the only algorithm
/// (`console_keys_alg_enum_check`).
pub const ALG_ESP256: &str = "esp256";

/// How long a key lives: exactly 720 hours from `paired_at` (owner ruling R3,
/// "a console key expires 30 days after pairing").
///
/// Hours, not days, and the same figure the database enforces
/// (`console_keys_lives_exactly_thirty_days`): a calendar-day interval differs
/// from thirty 24-hour days across a DST change in a non-UTC session, and the
/// two sides must compute one number.
///
/// ```
/// use vpay_db::CONSOLE_KEY_LIFETIME;
///
/// assert_eq!(CONSOLE_KEY_LIFETIME.whole_seconds(), 30 * 24 * 60 * 60);
/// ```
pub const CONSOLE_KEY_LIFETIME: Duration = Duration::hours(720);

/// One `console_keys` row, exactly as stored.
#[derive(Clone, PartialEq, Eq)]
pub struct ConsoleKeyRow {
    /// `ck_…`.
    pub id: String,
    /// The member this key signs as.
    pub staff_member_id: String,
    /// [`ALG_ESP256`].
    pub alg: String,
    /// The SEC1 uncompressed P-256 point as unpadded base64url (87 characters).
    pub public_key: String,
    /// The RFC 9679 thumbprint, lower-case hex (64 characters).
    pub thumbprint: String,
    /// The operator's name for the key, if it has one.
    pub label: Option<String>,
    /// When it was paired.
    pub paired_at: OffsetDateTime,
    /// `paired_at` plus [`CONSOLE_KEY_LIFETIME`].
    pub expires_at: OffsetDateTime,
    /// When it was revoked, if it was.
    pub revoked_at: Option<OffsetDateTime>,
}

impl ConsoleKeyRow {
    /// Whether this key authenticates at `now`: not revoked, and not past
    /// `expires_at`. The instant itself is expired (`expires_at <= now`).
    ///
    /// ```
    /// use time::{Duration, OffsetDateTime};
    /// use vpay_db::{ConsoleKeyRow, CONSOLE_KEY_LIFETIME};
    ///
    /// let paired = OffsetDateTime::UNIX_EPOCH;
    /// let mut row = ConsoleKeyRow {
    ///     id: "ck_1".into(),
    ///     staff_member_id: "stf_1".into(),
    ///     alg: "esp256".into(),
    ///     public_key: String::new(),
    ///     thumbprint: String::new(),
    ///     label: None,
    ///     paired_at: paired,
    ///     expires_at: paired + CONSOLE_KEY_LIFETIME,
    ///     revoked_at: None,
    /// };
    /// assert!(row.is_live(paired));
    /// assert!(row.is_live(row.expires_at - Duration::seconds(1)));
    /// assert!(!row.is_live(row.expires_at), "the instant it expires it is expired");
    /// row.revoked_at = Some(paired);
    /// assert!(!row.is_live(paired), "a revoked key is never live");
    /// ```
    #[must_use]
    pub fn is_live(&self, now: OffsetDateTime) -> bool {
        self.revoked_at.is_none() && self.expires_at > now
    }
}

/// Public key material and a thumbprint are public, and an operator needs both
/// to find a refused request's key; nothing here is a secret.
impl fmt::Debug for ConsoleKeyRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleKeyRow")
            .field("id", &self.id)
            .field("staff_member_id", &self.staff_member_id)
            .field("alg", &self.alg)
            .field("thumbprint", &self.thumbprint)
            .field("label", &self.label)
            .field("paired_at", &self.paired_at)
            .field("expires_at", &self.expires_at)
            .field("revoked_at", &self.revoked_at)
            .finish_non_exhaustive()
    }
}

/// Everything a writer supplies to pair a key. `expires_at` is not here: it is
/// `now + CONSOLE_KEY_LIFETIME`, computed once, in [`ConsoleKeys::pair_console_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewConsoleKey {
    /// `ck_…`, from `vpay_core::ids::console_key_id`.
    pub id: String,
    /// The member the key signs as.
    pub staff_member_id: String,
    /// The SEC1 uncompressed point, unpadded base64url.
    pub public_key: String,
    /// The RFC 9679 thumbprint, lower-case hex — computed by the caller from
    /// the key with `cratestack-cose`, never taken from a request.
    pub thumbprint: String,
    /// A name for the key, 1 to 64 characters.
    pub label: Option<String>,
    /// The pairing instant; also `paired_at`.
    pub now: OffsetDateTime,
}

/// The repository for `console_keys`.
#[async_trait]
pub trait ConsoleKeys {
    /// Inserts one key, live from `new.now` for [`CONSOLE_KEY_LIFETIME`].
    ///
    /// `now` is truncated to the microsecond Postgres stores, so
    /// `console_keys_lives_exactly_thirty_days` compares two values with one
    /// fractional part.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] — [`crate::PersistenceError::Unique`] when the
    /// thumbprint (or id) is taken, [`crate::PersistenceError::ForeignKey`]
    /// when the member does not exist, [`crate::PersistenceError::Check`] when
    /// a shape rule is broken, [`crate::PersistenceError::Denied`] if `model
    /// ConsoleKey` loses its `create` arm.
    async fn pair_console_key(&self, new: NewConsoleKey) -> Result<(), DbError>;

    /// The key with this thumbprint, live or not, or `None`.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`]; a missing `read` arm is **silent** (the
    /// policy is compiled into the `WHERE`), so every thumbprint answers `None`.
    async fn find_console_key(&self, thumbprint: &str) -> Result<Option<ConsoleKeyRow>, DbError>;

    /// The keys that might have produced a signature carrying this `kid` (the
    /// first 8 bytes of a thumbprint, as 16 lower-case hex characters) and
    /// that are live at `now`.
    ///
    /// The one read the signed transport makes on every `/dash/v1` request. A
    /// `kid` is eight bytes of a SHA-256, so more than one key can match;
    /// the caller verifies each candidate's signature.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn live_console_keys_for_kid(
        &self,
        kid_hex: &str,
        now: OffsetDateTime,
    ) -> Result<Vec<ConsoleKeyRow>, DbError>;

    /// Stamps `revoked_at = now` on the key with this thumbprint **if it is
    /// not revoked yet**. `true` when this call revoked it; `false` when it
    /// was already revoked (the first `revoked_at` is kept) or does not exist
    /// — the caller tells the two apart with [`Self::find_console_key`].
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`]; a missing `update` arm is **silent** and
    /// makes every revocation answer `false`.
    async fn revoke_console_key(
        &self,
        thumbprint: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError>;
}

/// `now` cut to the microsecond, the resolution `timestamptz` stores.
fn micros(at: OffsetDateTime) -> OffsetDateTime {
    at.replace_nanosecond(at.nanosecond() / 1_000 * 1_000)
        .unwrap_or(at)
}

/// The generated create input for a key paired at `new.now`: live for
/// [`CONSOLE_KEY_LIFETIME`], never revoked. The one place the 30-day life is
/// computed, shared by the pool path and the in-transaction one.
fn create_input(new: NewConsoleKey) -> cratestack_schema::CreateConsoleKeyInput {
    let paired_at = micros(new.now);
    cratestack_schema::CreateConsoleKeyInput {
        id: new.id,
        staff_member_id: new.staff_member_id,
        alg: ALG_ESP256.to_owned(),
        public_key: new.public_key,
        thumbprint: new.thumbprint,
        label: new.label,
        paired_at: to_chrono(paired_at),
        expires_at: to_chrono(paired_at + CONSOLE_KEY_LIFETIME),
        revoked_at: None,
    }
}

/// [`ConsoleKeys::pair_console_key`] inside a caller's transaction: the insert
/// that has to commit or roll back with the member it belongs to (a first
/// pairing provisions the member, their link and their key together).
///
/// # Errors
///
/// [`ConsoleKeys::pair_console_key`]'s.
pub(crate) async fn pair_in_tx(
    cs: &cratestack_schema::Cratestack,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    new: NewConsoleKey,
) -> Result<(), DbError> {
    cs.console_key()
        .create(create_input(new))
        .run_in_tx(&mut *tx, &system_context())
        .await
        .map_err(|error| DbError::from(classify_cratestack(MODEL, "create", error)))?;
    Ok(())
}

#[async_trait]
impl ConsoleKeys for crate::repository::PgRepositories {
    async fn pair_console_key(&self, new: NewConsoleKey) -> Result<(), DbError> {
        self.cs
            .console_key()
            .create(create_input(new))
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "create", error)))?;
        Ok(())
    }

    async fn find_console_key(&self, thumbprint: &str) -> Result<Option<ConsoleKeyRow>, DbError> {
        let rows = self
            .cs
            .console_key()
            .find_many()
            .where_(console_key::thumbprint().eq(thumbprint.to_owned()))
            .limit(1)
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))?;
        Ok(rows.into_iter().next().map(row_from_model))
    }

    async fn live_console_keys_for_kid(
        &self,
        kid_hex: &str,
        now: OffsetDateTime,
    ) -> Result<Vec<ConsoleKeyRow>, DbError> {
        let rows = self
            .cs
            .console_key()
            .find_many()
            .where_(console_key::thumbprint().starts_with(kid_hex.to_owned()))
            .where_(console_key::revoked_at().is_null())
            .where_(console_key::expires_at().gt(to_chrono(now)))
            // A `kid` is 8 bytes of a SHA-256: a second candidate is a
            // 2^-64-per-pair event, and a bound keeps a hostile table from
            // making this read unbounded.
            .limit(16)
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))?;
        Ok(rows.into_iter().map(row_from_model).collect())
    }

    async fn revoke_console_key(
        &self,
        thumbprint: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError> {
        let summary = self
            .cs
            .console_key()
            .update_many()
            .where_(console_key::thumbprint().eq(thumbprint.to_owned()))
            .where_(console_key::revoked_at().is_null())
            .set(cratestack_schema::UpdateConsoleKeyInput {
                revoked_at: Some(Some(to_chrono(micros(now)))),
                ..cratestack_schema::UpdateConsoleKeyInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;
        Ok(summary.ok == 1)
    }
}

fn row_from_model(model: cratestack_schema::models::ConsoleKey) -> ConsoleKeyRow {
    ConsoleKeyRow {
        id: model.id,
        staff_member_id: model.staff_member_id,
        alg: model.alg,
        public_key: model.public_key,
        thumbprint: model.thumbprint,
        label: model.label,
        paired_at: from_chrono(model.paired_at),
        expires_at: from_chrono(model.expires_at),
        revoked_at: model.revoked_at.map(from_chrono),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `@@allow` slot this module's calls need is occupied — the
    /// staff module's device, without a database. `read` and `update` are
    /// **silent** when missing (the policy compiles into the `WHERE`), and
    /// both are security-relevant here: a missing `read` arm would refuse every
    /// console, and a missing `update` arm would make a revocation a no-op that
    /// answers `false`.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use cratestack_schema::models::CONSOLE_KEY_MODEL as descriptor;

        assert!(
            !descriptor.read_allow_policies.is_empty(),
            "model ConsoleKey lost @@allow(\"read\", …): every signature would resolve to no key"
        );
        assert!(
            !descriptor.create_allow_policies.is_empty(),
            "model ConsoleKey lost @@allow(\"create\", …): no key could be paired"
        );
        assert!(
            !descriptor.update_allow_policies.is_empty(),
            "model ConsoleKey lost @@allow(\"update\", …): revocation would silently do nothing"
        );
        assert!(
            descriptor.delete_allow_policies.is_empty(),
            "model ConsoleKey gained @@allow(\"delete\", …): a deleted row frees its thumbprint, \
             and a revoked key could be registered again"
        );
        assert!(
            descriptor.read_deny_policies.is_empty()
                && descriptor.create_deny_policies.is_empty()
                && descriptor.update_deny_policies.is_empty()
                && descriptor.delete_deny_policies.is_empty(),
            "a @@deny arm appeared on model ConsoleKey; every deny is evaluated ahead of every \
             allow, so one added here refuses the system principal this crate runs as"
        );
    }

    #[test]
    fn micros_cuts_below_the_resolution_postgres_stores() {
        let at = OffsetDateTime::UNIX_EPOCH + Duration::nanoseconds(1_234_567_891);
        assert_eq!(micros(at).nanosecond() % 1_000, 0);
        assert_eq!(micros(at).nanosecond(), 234_567_000);
    }
}
