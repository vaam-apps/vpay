//! The `staff_sessions` repository
//! (`backends/migrations/0035_create-staff-auth.sql`) — "this browser is that
//! person, until 12 hours from now"
//! ([ADR-0017](../../../../docs/adr/0017-staff-authentication.md) decision 2).
//!
//! # The two bounds are not one bound
//!
//! * **Absolute**, 12 hours from creation, never extended. An attacker who
//!   steals a session cannot outrun this by using it, which is the whole
//!   reason it is separate from the idle bound.
//! * **Idle**, 30 minutes since the last accepted request. Moved forward by
//!   every accepted request, so an unattended browser stops being a
//!   credential long before the absolute bound arrives.
//!
//! Both are enforced on **read**, by [`StaffSessions::load`], and neither is
//! enforced by a delete: an expired row is refused and left in place, because
//! deleting it would make "this session expired" and "this session never
//! existed" the same answer to an operator reading the table. Nothing sweeps
//! the table yet — `docs/status.md` says so, and
//! `staff_sessions_expiry_idx` is what a sweep would need rather than
//! evidence that one exists.
//!
//! # The id in this table is never the credential
//!
//! The caller holds an opaque token; the primary key is its SHA-256, as hex.
//! Hashing is the caller's ([`vpay_api::staff_auth`]), not this crate's, for
//! [`crate::staff`]'s reason — but the *contract* is here, because it is what
//! makes a dump of this table yield no usable session.

use std::fmt;

use async_trait::async_trait;
use time::{Duration, OffsetDateTime};

use crate::error::DbError;
use crate::persistence::{classify_cratestack, system_context};
use crate::schema::cratestack_schema::{self, staff_session};
use crate::staff::{from_chrono, to_chrono};

/// The `.cstack` model these calls name.
const MODEL: &str = "StaffSession";

/// The wire value of [`SessionState::PendingTotp`].
const STATE_PENDING_TOTP: &str = "pending_totp";
/// The wire value of [`SessionState::Authenticated`].
const STATE_AUTHENTICATED: &str = "authenticated";

/// How long a session lives from creation, whatever it does in between.
///
/// Twelve hours: ADR-0017 decision 2, and it is a *product* rule — one
/// working day — rather than a security floor, which is why it is a constant
/// here and not a column default. A shorter absolute bound would sign people
/// out mid-shift; a longer one would let a session survive the day it was
/// stolen on.
pub const ABSOLUTE_LIFETIME: Duration = Duration::hours(12);

/// How long a session survives without being used.
///
/// Thirty minutes: ADR-0017 decision 2. This is the bound that matters for an
/// unattended browser, and it is deliberately far shorter than the absolute
/// one.
pub const IDLE_TIMEOUT: Duration = Duration::minutes(30);

/// How far a session has got through sign-in.
///
/// Two states and no third. The password-change step deliberately has none:
/// it is read off `staff_members.password_change_required`, so a session cannot be
/// promoted past it by anything that writes this column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// The password was accepted. This session may present a TOTP code and
    /// do nothing else — in particular, `/dash/v1/oauth/authorize` refuses
    /// it.
    PendingTotp,
    /// Both factors were accepted.
    Authenticated,
}

impl SessionState {
    /// The stored spelling.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            SessionState::PendingTotp => STATE_PENDING_TOTP,
            SessionState::Authenticated => STATE_AUTHENTICATED,
        }
    }

    /// The stored spelling, back. `None` for anything else — and no default,
    /// for [`crate::StaffStatus::parse`]'s reason: the first variant is the
    /// *less* privileged one here, but relying on that ordering to make a
    /// default safe is exactly the kind of coincidence that stops being true.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            STATE_PENDING_TOTP => Some(SessionState::PendingTotp),
            STATE_AUTHENTICATED => Some(SessionState::Authenticated),
            _ => None,
        }
    }
}

/// One `staff_sessions` row, as stored.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// The SHA-256 of the session token, hex. Never the token.
    pub id: String,
    /// Whose session it is.
    pub staff_id: String,
    /// How far through sign-in it is.
    pub state: SessionState,
    /// When it was created.
    pub created_at: OffsetDateTime,
    /// The absolute bound. Never moved.
    pub expires_at: OffsetDateTime,
    /// The idle bound's input. Moved by every accepted request.
    pub last_seen_at: OffsetDateTime,
    /// The `/dash/v1` access token this session's code exchange minted, or
    /// `None` before it. A live bearer credential — see the `Debug` impl.
    pub access_token: Option<String>,
    /// When [`Self::access_token`] stops being accepted, or `None` when there
    /// is none.
    ///
    /// `None` exactly when the token is — migration 0040's
    /// `staff_sessions_token_expiry_is_paired`. It is what lets the dashboard
    /// replace the token **before** a read fails on it (issue #88 item 1)
    /// rather than after; see [`StaffSessions::record_access_token`].
    pub access_token_expires_at: Option<OffsetDateTime>,
}

/// Redacts the access token, which is a live bearer credential for the length
/// of its TTL. The id is already a digest and the staff id is opaque, so both
/// are printed: they are what an operator debugging a session needs.
impl fmt::Debug for SessionRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionRow")
            .field("id", &self.id)
            .field("staff_id", &self.staff_id)
            .field("state", &self.state)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("last_seen_at", &self.last_seen_at)
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "[redacted]"),
            )
            // NOT redacted, unlike the token beside it: an expiry is a
            // timestamp and it is the one field somebody debugging a re-mint
            // loop actually needs to read.
            .field("access_token_expires_at", &self.access_token_expires_at)
            .finish()
    }
}

impl SessionRow {
    /// Whether this session is still inside **both** bounds at `now`.
    ///
    /// A free function on the row rather than a filter in the statement,
    /// deliberately: [`StaffSessions::load`] has to be able to answer
    /// "expired" *without* deleting anything, and a `WHERE` clause that
    /// excluded expired rows would make an expired session and a forged one
    /// the same `None`. It is the same distinction
    /// `vpay_api::staff_auth` then collapses on the wire — a caller is told
    /// `401` either way — kept apart here so an operator's read of the table
    /// is not.
    #[must_use]
    pub fn is_live_at(&self, now: OffsetDateTime) -> bool {
        now < self.expires_at && now < self.last_seen_at.saturating_add(IDLE_TIMEOUT)
    }
}

/// Everything the login handler supplies for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    /// The SHA-256 of the token this session was minted with, hex.
    pub id: String,
    /// Whose session it is.
    pub staff_id: String,
    /// Always [`SessionState::PendingTotp`] today: the only caller is the
    /// password step. It is a field rather than a hard-coded value so that a
    /// second factor becoming optional would be a visible change at the call
    /// site rather than an invisible one here.
    pub state: SessionState,
    /// When it was created. `expires_at` is derived from this and
    /// [`ABSOLUTE_LIFETIME`], here rather than by the caller, so the two
    /// cannot drift.
    pub now: OffsetDateTime,
}

/// Reads and writes of `staff_sessions`. Every method through CrateStack.
#[async_trait]
pub trait StaffSessions {
    /// Creates a session in [`SessionState::PendingTotp`].
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] — [`crate::PersistenceError::Unique`] on the
    /// (astronomically unlikely) digest collision,
    /// [`crate::PersistenceError::ForeignKey`] if the staff row went away
    /// between the read and this write, [`crate::PersistenceError::Backend`]
    /// otherwise.
    async fn create(&self, new: NewSession) -> Result<(), DbError>;

    /// The session for a token digest, **if it is still inside both bounds**.
    ///
    /// `None` covers "no such session", "expired" and "idle" without telling
    /// the caller which — the same uniformity `/dash/v1`'s 404 keeps for a
    /// foreign object id, and for the same reason: a caller who could tell
    /// them apart could use this as an oracle for which session ids exist.
    ///
    /// It does **not** touch `last_seen_at`: moving the idle bound is a
    /// separate, explicit call ([`StaffSessions::touch`]), so a read that
    /// happens to run during a request cannot silently keep a session alive.
    ///
    /// # Errors
    ///
    /// [`DbError::SessionStateUnknown`] if the stored `state` is outside the
    /// vocabulary, [`DbError::Persistence`] otherwise.
    async fn load(&self, id: &str, now: OffsetDateTime) -> Result<Option<SessionRow>, DbError>;

    /// Moves the idle bound forward. `false` means no row moved.
    ///
    /// The filter is `last_seen_at < now`, which does two things: it makes a
    /// repeated touch within the same instant a no-op rather than a write,
    /// and it means a stamp that would move the clock **backwards** matches
    /// nothing. Two vpay processes do not share a clock, and a session whose
    /// idle bound could be dragged back is one a slow replica could expire
    /// early.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn touch(&self, id: &str, now: OffsetDateTime) -> Result<bool, DbError>;

    /// Promotes a session to [`SessionState::Authenticated`], once the TOTP
    /// code has been accepted. `false` means no row moved.
    ///
    /// The guard is `state = 'pending_totp'`, in the statement: a session
    /// that is already authenticated must not be re-promoted, because the
    /// only caller that would try is one replaying the TOTP step.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn mark_authenticated(&self, id: &str, now: OffsetDateTime) -> Result<bool, DbError>;

    /// Records the access token the code exchange minted for this session,
    /// **and when it expires**. `false` means no such session.
    ///
    /// This is what makes [`StaffSessions::delete`] a **revocation** rather
    /// than a sign-out: the token cannot be presented by anyone who cannot
    /// read it back out of this row, and the row is gone. It is the
    /// server-side deny-list ADR-0009's Consequences section left undecided.
    ///
    /// `expires_at` is not derived here, unlike [`NewSession`]'s absolute
    /// bound: the TTL is the *caller's* — `staff_auth.access_token_ttl_seconds`,
    /// read off the OP configuration the token was actually signed under — and
    /// a second copy of it in this crate would be a number that could disagree
    /// with the one inside the JWT. What this crate guarantees is that the two
    /// columns move together, which is migration 0040's CHECK.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn record_access_token(
        &self,
        id: &str,
        access_token: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<bool, DbError>;

    /// Deletes a session. `false` means there was none — sign-out is
    /// idempotent, and a caller signing out twice is not an error.
    ///
    /// The cascade on `oauth_authorization_codes.session_id` means this also
    /// kills any code this session issued and has not yet exchanged, which is
    /// the property that makes signing out during a login race safe.
    ///
    /// `delete_many` and not `delete(pk)`, for
    /// [`crate::DisabledClients::enable_client`]'s measured reason:
    /// `delete_exec.rs` turns "matched no row" into
    /// `CratestackError::Forbidden`, so the single-row builder cannot tell
    /// "there was no session" from "the policy refused you" — and here the
    /// first is the ordinary case.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn delete(&self, id: &str) -> Result<bool, DbError>;

    /// Deletes every session of `staff_id` **except** `keep_id`, and answers
    /// how many went.
    ///
    /// The write that makes a password change mean something (issue #79
    /// item 3). Replacing a password while somebody else's browser holds a
    /// live session of the same account changes nothing about that browser:
    /// the session was authenticated before the change and its `access_token`
    /// column is still readable. Whoever the password was changed *because
    /// of* keeps reading `/dash/v1` until the absolute bound twelve hours
    /// later.
    ///
    /// `keep_id` and not "all of them", because the caller is one of them.
    /// Signing out the browser that just chose a new password would make the
    /// success case look like a failure, and a person who has just proved
    /// two factors and their current password is the one caller here whose
    /// session is known good.
    ///
    /// The cascade on `oauth_authorization_codes.session_id` applies to each
    /// deleted row, exactly as it does for [`StaffSessions::delete`], so a
    /// code another browser had in flight dies with its session.
    ///
    /// `delete_many` for [`StaffSessions::delete`]'s measured reason —
    /// `delete_exec.rs` turns "matched no row" into
    /// `CratestackError::Forbidden`, and "there were no other sessions" is
    /// the ordinary case here, not an error.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn delete_others(&self, staff_id: &str, keep_id: &str) -> Result<usize, DbError>;
}

#[async_trait]
impl StaffSessions for crate::repository::PgRepositories {
    async fn create(&self, new: NewSession) -> Result<(), DbError> {
        self.cs
            .staff_session()
            .create(cratestack_schema::CreateStaffSessionInput {
                id: new.id,
                staff_id: new.staff_id,
                state: new.state.as_wire_str().to_owned(),
                created_at: to_chrono(new.now),
                // Derived here, never by the caller: the absolute bound and
                // the creation instant are one decision.
                expires_at: to_chrono(new.now.saturating_add(ABSOLUTE_LIFETIME)),
                last_seen_at: to_chrono(new.now),
                access_token: None,
                // Paired with the token, and there is none yet: the code
                // exchange is what writes both.
                access_token_expires_at: None,
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "create", error)))?;

        Ok(())
    }

    async fn load(&self, id: &str, now: OffsetDateTime) -> Result<Option<SessionRow>, DbError> {
        let Some(model) = self
            .cs
            .staff_session()
            .find_unique(id.to_owned())
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))?
        else {
            return Ok(None);
        };

        let row = row_from_model(model)?;
        // The bounds are checked here and not in the statement: see
        // `SessionRow::is_live_at` for why an expired row is refused rather
        // than filtered out.
        Ok(row.is_live_at(now).then_some(row))
    }

    async fn touch(&self, id: &str, now: OffsetDateTime) -> Result<bool, DbError> {
        let summary = self
            .cs
            .staff_session()
            .update_many()
            .where_(staff_session::id().eq(id.to_owned()))
            .where_(staff_session::last_seen_at().lt(to_chrono(now)))
            .set(cratestack_schema::UpdateStaffSessionInput {
                last_seen_at: Some(to_chrono(now)),
                ..cratestack_schema::UpdateStaffSessionInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        Ok(summary.ok == 1)
    }

    async fn mark_authenticated(&self, id: &str, now: OffsetDateTime) -> Result<bool, DbError> {
        let summary = self
            .cs
            .staff_session()
            .update_many()
            .where_(staff_session::id().eq(id.to_owned()))
            .where_(staff_session::state().eq(SessionState::PendingTotp.as_wire_str().to_owned()))
            .set(cratestack_schema::UpdateStaffSessionInput {
                state: Some(SessionState::Authenticated.as_wire_str().to_owned()),
                last_seen_at: Some(to_chrono(now)),
                ..cratestack_schema::UpdateStaffSessionInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        Ok(summary.ok == 1)
    }

    async fn record_access_token(
        &self,
        id: &str,
        access_token: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<bool, DbError> {
        let summary = self
            .cs
            .staff_session()
            .update_many()
            .where_(staff_session::id().eq(id.to_owned()))
            .set(cratestack_schema::UpdateStaffSessionInput {
                access_token: Some(Some(access_token.to_owned())),
                // In the SAME statement as the token, which is what makes
                // migration 0040's paired CHECK a property rather than a
                // convention: there is no instant at which one is written
                // and the other is not.
                access_token_expires_at: Some(Some(to_chrono(expires_at))),
                last_seen_at: Some(to_chrono(now)),
                ..cratestack_schema::UpdateStaffSessionInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        Ok(summary.ok == 1)
    }

    async fn delete(&self, id: &str) -> Result<bool, DbError> {
        let summary = self
            .cs
            .staff_session()
            .delete_many()
            .where_(staff_session::id().eq(id.to_owned()))
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "delete", error)))?;

        Ok(summary.ok == 1)
    }

    async fn delete_others(&self, staff_id: &str, keep_id: &str) -> Result<usize, DbError> {
        let summary = self
            .cs
            .staff_session()
            .delete_many()
            .where_(staff_session::staff_id().eq(staff_id.to_owned()))
            // `ne` and not "delete then re-create": the caller's own session
            // must survive the statement, not survive a gap in it. A delete
            // of everything followed by an insert would leave the person who
            // changed their password signed out for the width of two
            // statements, and signed out for good if the process died between
            // them.
            .where_(staff_session::id().ne(keep_id.to_owned()))
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "delete", error)))?;

        Ok(summary.ok)
    }
}

/// The generated model row, in vpay's own types.
fn row_from_model(model: cratestack_schema::models::StaffSession) -> Result<SessionRow, DbError> {
    let state = SessionState::parse(&model.state).ok_or_else(|| DbError::SessionStateUnknown {
        id: model.id.clone(),
        state: model.state.clone(),
    })?;

    Ok(SessionRow {
        id: model.id,
        staff_id: model.staff_id,
        state,
        created_at: from_chrono(model.created_at),
        expires_at: from_chrono(model.expires_at),
        last_seen_at: from_chrono(model.last_seen_at),
        access_token: model.access_token,
        access_token_expires_at: model.access_token_expires_at.map(from_chrono),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// See [`crate::staff`]'s equivalent for why this exists and what it
    /// does not replace. The `update` slot is the dangerous one again: a
    /// missing arm would make `mark_authenticated` answer `Ok(false)`
    /// forever with no error, which fails closed — and `touch` answer
    /// `Ok(false)` forever, which does **not** matter for safety (the idle
    /// bound would simply never move, so every session would expire after 30
    /// minutes) but would look like a bug nobody could locate.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use cratestack_schema::models::STAFF_SESSION_MODEL as descriptor;

        assert!(
            !descriptor.read_allow_policies.is_empty(),
            "model StaffSession lost @@allow(\"read\", …): every session would read as \
             absent and nobody could stay signed in"
        );
        assert!(
            !descriptor.create_allow_policies.is_empty(),
            "model StaffSession lost @@allow(\"create\", …): sign-in would fail loudly"
        );
        assert!(
            !descriptor.update_allow_policies.is_empty(),
            "model StaffSession lost @@allow(\"update\", …): mark_authenticated would answer \
             Ok(false) forever, with no error anywhere"
        );
        assert!(
            !descriptor.delete_allow_policies.is_empty(),
            "model StaffSession lost @@allow(\"delete\", …): sign-out would report success \
             while leaving the session — and its access token — live"
        );
    }

    /// The two bounds are separate, and each refuses on its own.
    #[test]
    fn a_session_is_live_only_inside_both_bounds() {
        let created = OffsetDateTime::UNIX_EPOCH;
        let session = |last_seen: OffsetDateTime| SessionRow {
            id: "a".repeat(64),
            staff_id: "stf_1".to_owned(),
            state: SessionState::Authenticated,
            created_at: created,
            expires_at: created.saturating_add(ABSOLUTE_LIFETIME),
            last_seen_at: last_seen,
            access_token: None,
            access_token_expires_at: None,
        };

        assert!(session(created).is_live_at(created));
        assert!(
            session(created).is_live_at(created.saturating_add(Duration::minutes(29))),
            "inside both bounds"
        );
        assert!(
            !session(created).is_live_at(created.saturating_add(Duration::minutes(31))),
            "the IDLE bound refuses on its own, hours before the absolute one"
        );

        // Kept alive by use right up to the absolute bound, and refused the
        // moment it passes — which is the property that makes the two bounds
        // two bounds rather than one.
        let almost = created.saturating_add(ABSOLUTE_LIFETIME) - Duration::minutes(1);
        assert!(session(almost).is_live_at(almost));
        let past = created.saturating_add(ABSOLUTE_LIFETIME);
        assert!(
            !session(past).is_live_at(past),
            "the ABSOLUTE bound refuses however recently the session was used"
        );
    }

    /// The vocabulary round-trips and admits nothing else.
    #[test]
    fn the_state_vocabulary_round_trips_and_admits_nothing_else() {
        for state in [SessionState::PendingTotp, SessionState::Authenticated] {
            assert_eq!(SessionState::parse(state.as_wire_str()), Some(state));
        }
        assert_eq!(SessionState::parse("authorised"), None);
        assert_eq!(SessionState::parse(""), None);
    }

    /// The access token never reaches a `{:?}`.
    #[test]
    fn debug_does_not_leak_the_access_token() {
        let row = SessionRow {
            id: "b".repeat(64),
            staff_id: "stf_1".to_owned(),
            state: SessionState::Authenticated,
            created_at: OffsetDateTime::UNIX_EPOCH,
            expires_at: OffsetDateTime::UNIX_EPOCH,
            last_seen_at: OffsetDateTime::UNIX_EPOCH,
            access_token: Some("eyJhbGciOiJSUzI1NiJ9.payload.sig".to_owned()),
            access_token_expires_at: Some(OffsetDateTime::UNIX_EPOCH + Duration::minutes(15)),
        };

        let rendered = format!("{row:?}");
        assert!(!rendered.contains("eyJhbGciOiJSUzI1NiJ9"), "{rendered}");
        assert!(rendered.contains("[redacted]"), "{rendered}");
    }
}
