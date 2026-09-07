//! The `staff_members` repository (`backends/migrations/0035_create-staff-auth.sql`)
//! — who may sign in to `/dash/v1`, and the two credentials that prove it
//! ([ADR-0017](../../../../docs/adr/0017-staff-authentication.md)).
//!
//! # Every method here runs through CrateStack, and that is the point
//!
//! [`crate::disabled_clients`] moved three methods and [`crate::customers`]
//! two of seven; this module moves all six, because migration 0035 was shaped
//! so it could — no `jsonb`, no `bytea`, no native enum, no `DEFAULT` on any
//! column a writer names. `docs/reference/vpay-db.md` § CrateStack carries the
//! general account; what is specific here is that the *security* properties
//! are now carried by generated statements, so the four `@@allow` arms on
//! `model StaffMember` are load-bearing in a way no previous model's were. Two of
//! the four fail **silently** (see [`Staff::record_totp_step`]), which is why
//! [`tests::every_action_this_module_calls_has_an_allow_arm`] exists and runs
//! without a container.
//!
//! # What this module does not know
//!
//! It never hashes, never verifies and never decrypts. `password_hash` and
//! `totp_secret` are opaque strings on the way in and on the way out;
//! `vpay_api::staff_auth` owns argon2id, RFC 6238 and the AEAD, because those
//! need deployment secrets this crate has no business holding. The one
//! credential rule that *is* here is the replay guard, and it is here because
//! it is a compare-and-swap on a row — see [`Staff::record_totp_step`].

use std::fmt;

use async_trait::async_trait;
use time::OffsetDateTime;

use crate::error::DbError;
use crate::persistence::{classify_cratestack, system_context};
use crate::schema::cratestack_schema::{self, staff_member};

/// The `.cstack` model these calls name, for
/// [`crate::persistence::classify_cratestack`]'s `model` slot.
const MODEL: &str = "StaffMember";

/// The wire value of [`StaffStatus::Active`], and one of the two
/// `staff_members_status_is_known` admits.
const STATUS_ACTIVE: &str = "active";
/// The wire value of [`StaffStatus::Disabled`].
const STATUS_DISABLED: &str = "disabled";

/// Whether this account may sign in at all.
///
/// A Rust enum over a `TEXT` column with a hand-named CHECK, which is the
/// shape `events.type` and `jobs.kind` already use and the shape migration
/// 0035's header argues for: CrateStack decodes every enum column with
/// `try_get::<String>()`, so a native Postgres enum fails to decode on every
/// read (upstream #228, the defect migration 0032 had to convert
/// `providers.flow` out of).
///
/// The vocabulary is closed twice: by `staff_members_status_is_known` at the
/// database, and by [`StaffStatus::parse`] refusing anything else here. A
/// value in one and not the other is a row this crate declines to decode
/// rather than a row it guesses about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaffStatus {
    /// May sign in, and may hold a session.
    Active,
    /// May not. Refused at sign-in **and** on every session read, so
    /// disabling an account takes effect on that person's next request
    /// rather than at their next login.
    Disabled,
}

impl StaffStatus {
    /// The stored spelling.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            StaffStatus::Active => STATUS_ACTIVE,
            StaffStatus::Disabled => STATUS_DISABLED,
        }
    }

    /// The stored spelling, back.
    ///
    /// `None` for anything else, and the caller turns that into
    /// [`DbError::StaffStatusUnknown`] rather than a default. A default here
    /// would be `Active` (the first variant), i.e. a row the database refused
    /// to constrain would be read as *permitted to sign in* — the
    /// `unwrap_or_default()` hazard `docs/reference/vpay-db.md` records for
    /// `ProviderFlow`, on a column where the consequence is authentication
    /// rather than a mis-labelled rail.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            STATUS_ACTIVE => Some(StaffStatus::Active),
            STATUS_DISABLED => Some(StaffStatus::Disabled),
            _ => None,
        }
    }
}

/// One `staff_members` row, exactly as stored.
///
/// `Debug` is **hand-written** below: three of these fields are credentials
/// or personal data, and `{:?}` on this struct reaches `tracing` fields,
/// `anyhow` chains and every failing assertion's output.
#[derive(Clone, PartialEq, Eq)]
pub struct StaffRow {
    /// `stf_…`.
    pub id: String,
    /// The one merchant whose rows this person may see.
    pub merchant_id: String,
    /// Lower-cased, unique across the deployment.
    pub email: String,
    /// What the dashboard greets them by.
    pub display_name: String,
    /// An argon2id PHC string. Opaque here — `vpay_api::staff_auth` verifies
    /// it, and the pepper it needs is a deployment secret this crate never
    /// sees.
    pub password_hash: String,
    /// Set by `staff add` and cleared when the person picks their own
    /// password. Every authenticated route refuses a session whose staff row
    /// still has it, so the printed one-time password cannot become a
    /// long-lived credential by being ignored.
    pub password_change_required: bool,
    /// The sealed RFC 6238 secret, or `None` before enrolment. Opaque here.
    pub totp_secret: Option<String>,
    /// When enrolment completed. `None` exactly when [`Self::totp_secret`]
    /// is — `staff_members_totp_is_paired` makes that an invariant.
    pub totp_enrolled_at: Option<OffsetDateTime>,
    /// The time step of the last accepted code; `0` before the first.
    pub last_totp_step: i64,
    /// Whether this account may sign in.
    pub status: StaffStatus,
    /// When the row was created.
    pub created_at: OffsetDateTime,
    /// When it last changed.
    pub updated_at: OffsetDateTime,
    /// When this person last completed a full sign-in. Not "last seen" —
    /// that is `staff_sessions.last_seen_at`.
    pub last_sign_in_at: Option<OffsetDateTime>,
}

/// Redacts the password hash, the TOTP secret and the email address.
///
/// The hash is an offline cracking target; the sealed secret is a second
/// factor; the address is personal data and is also the sign-in identifier,
/// so a log line carrying it hands a reader half of a credential pair. The
/// id, the merchant and the status are what an operator debugging this table
/// actually needs, and none of them is any of those things.
impl fmt::Debug for StaffRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaffRow")
            .field("id", &self.id)
            .field("merchant_id", &self.merchant_id)
            .field("email", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("password_hash", &"[redacted]")
            .field("password_change_required", &self.password_change_required)
            .field(
                "totp_secret",
                &self.totp_secret.as_ref().map(|_| "[redacted]"),
            )
            .field("totp_enrolled_at", &self.totp_enrolled_at)
            .field("last_totp_step", &self.last_totp_step)
            .field("status", &self.status)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("last_sign_in_at", &self.last_sign_in_at)
            .finish()
    }
}

impl StaffRow {
    /// Whether this account may sign in and may hold a session.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.status == StaffStatus::Active
    }

    /// Whether TOTP enrolment has happened. Enrolment is mandatory at first
    /// sign-in, so `false` here means the session may do exactly one thing.
    #[must_use]
    pub fn is_totp_enrolled(&self) -> bool {
        self.totp_secret.is_some()
    }
}

/// Everything `vpay-server staff add` supplies for one row.
///
/// Every column, with no defaults anywhere — migration 0035 declares none on
/// purpose (see its header), so this struct is the *whole* insert and a
/// column added to the table without being added here is a compile error at
/// the `CreateStaffMemberInput` literal rather than a silently defaulted value.
#[derive(Clone, PartialEq, Eq)]
pub struct NewStaff {
    /// `stf_…`, from `vpay_core::ids::staff_id`.
    pub id: String,
    /// Checked against `merchant_clients[].merchant_id` by the CLI before it
    /// gets here; there is no merchants table to make it a foreign key.
    pub merchant_id: String,
    /// **Must already be lower-cased.** `staff_members_email_is_lower_case` refuses
    /// the insert otherwise, deliberately loudly: the read is a plain
    /// `email = $1`, so a row written with a capital letter is an account
    /// that can never sign in and whose failure has no diagnosis.
    pub email: String,
    /// What the dashboard greets them by.
    pub display_name: String,
    /// The argon2id PHC string for the one-time password the CLI printed.
    pub password_hash: String,
    /// When the row is created; also its `updated_at`.
    pub now: OffsetDateTime,
}

/// Redacts for [`StaffRow`]'s reason. Same fields, same argument.
impl fmt::Debug for NewStaff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewStaff")
            .field("id", &self.id)
            .field("merchant_id", &self.merchant_id)
            .field("email", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("password_hash", &"[redacted]")
            .field("now", &self.now)
            .finish()
    }
}

/// Reads and writes of the `staff_members` table.
///
/// Six methods, every one of them through CrateStack. There is deliberately
/// **no** `list`, no `disable` and no `delete`: the only writer of this table
/// is the operator CLI, ADR-0017 gives it exactly one subcommand, and a
/// repository method with no caller is a claim `docs/status.md` would have to
/// carry.
#[async_trait]
pub trait Staff {
    /// Inserts one staff member. The operator CLI is the only caller, and
    /// there is no HTTP endpoint that reaches this (ADR-0017 decision 1).
    ///
    /// `create`, not `upsert`. A second `staff add` for an address that
    /// already exists must **fail**, not quietly rewrite that person's
    /// password hash to one an operator just printed on a terminal.
    ///
    /// **What actually refuses it is measured rather than assumed**, and the
    /// two halves are different. `staff add` mints a fresh `stf_…` every
    /// time, so the second row's *primary key* is new and the conflict is on
    /// the **email** — which `staff_members_email_key` refuses whichever
    /// builder is used. Swapping this `create` for an `upsert` therefore
    /// changes nothing observable for that case, and a mutation proved it:
    /// `a_staff_address_is_unique_and_looked_up_exactly` stays green.
    ///
    /// The builder choice is what refuses the *other* case — a caller that
    /// supplies an id already in the table — where an upsert would overwrite
    /// silently and a create raises. No caller does that today (`staff add`
    /// generates the id), so it is a guard against a second writer rather
    /// than a live one, and it is stated that way instead of being claimed as
    /// the reason the duplicate-address case fails.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] — [`crate::PersistenceError::Unique`] when
    /// the id or the address is taken, [`crate::PersistenceError::Check`]
    /// when the address is not lower-cased or a bound is exceeded,
    /// [`crate::PersistenceError::Denied`] if `model StaffMember` loses its
    /// `@@allow("create", …)` (loud: the create path evaluates its policies
    /// in Rust before any SQL), [`crate::PersistenceError::Backend`]
    /// otherwise.
    async fn create(&self, new: NewStaff) -> Result<(), DbError>;

    /// The account for an address, or `None`.
    ///
    /// **The address must already be lower-cased by the caller.** This is a
    /// plain equality on the column, matching `staff_members_email_is_lower_case`;
    /// there is no `lower(email)` in the statement, because a functional
    /// predicate here would let a writer store a form the database and the
    /// reader disagreed about.
    ///
    /// `None` is the answer for "no such address" and is also what a caller
    /// must produce for a wrong password: see `vpay_api::staff_auth`'s login
    /// handler, which runs the same argon2 verification against a dummy hash
    /// so the two take the same path.
    ///
    /// # Errors
    ///
    /// [`DbError::StaffStatusUnknown`] if the stored `status` is outside the
    /// vocabulary (only reachable if `staff_members_status_is_known` were dropped),
    /// [`DbError::Persistence`] otherwise.
    async fn find_by_email(&self, email: &str) -> Result<Option<StaffRow>, DbError>;

    /// The account for an id, or `None`. The session path's read: every
    /// request re-reads the staff row rather than trusting what the session
    /// recorded, which is what makes disabling an account take effect on the
    /// next request.
    ///
    /// # Errors
    ///
    /// [`Staff::find_by_email`]'s.
    async fn find(&self, id: &str) -> Result<Option<StaffRow>, DbError>;

    /// Records a completed TOTP enrolment: the sealed secret and the instant.
    ///
    /// `false` means no row moved, which here means **the account was
    /// already enrolled** — the guard is `totp_enrolled_at IS NULL`. That
    /// guard is not decoration: without
    /// it, a caller who reached the enrolment screen twice could replace a
    /// working second factor with one they had just been shown, which is a
    /// second-factor reset with no authentication in front of it.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] — [`crate::PersistenceError::Check`] if the
    /// pair invariant would break, [`crate::PersistenceError::Backend`]
    /// otherwise. A missing `@@allow("update", …)` is **silent** here: see
    /// [`Staff::record_totp_step`].
    async fn enrol_totp(
        &self,
        id: &str,
        sealed_secret: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError>;

    /// **The TOTP replay guard.** Records `step` as the last accepted one,
    /// but only if it is strictly greater than what is stored.
    ///
    /// `false` means the code was already spent — the caller refuses the
    /// sign-in. This is the whole of the replay defence and it is a
    /// compare-and-swap on the row rather than a check in Rust, because two
    /// concurrent presentations of the same six digits are exactly the race
    /// a read-then-write would lose.
    ///
    /// # Why a missing `@@allow("update", …)` is the most dangerous edit in
    /// this crate
    ///
    /// `update_many`'s policy is compiled into the statement's own `WHERE`
    /// (`update_many_exec.rs`), so an empty allow list renders `FALSE`, the
    /// statement matches zero rows and this returns `Ok(false)` — **with no
    /// error anywhere**. `Ok(false)` is refuse-the-code, so the immediate
    /// effect is fail-closed and nobody can sign in; the danger is the fix
    /// somebody reaches for when every sign-in starts failing. It is pinned
    /// by [`tests::every_action_this_module_calls_has_an_allow_arm`], which
    /// names the slot rather than the symptom.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn record_totp_step(
        &self,
        id: &str,
        step: i64,
        now: OffsetDateTime,
    ) -> Result<bool, DbError>;

    /// Replaces the password hash and clears `password_change_required`.
    ///
    /// `false` means no such staff member. There is no "old password"
    /// parameter: the caller has already authenticated the session that is
    /// making the change, and re-checking a password this method would then
    /// overwrite would be a second copy of that check in the wrong layer.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn set_password(
        &self,
        id: &str,
        password_hash: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError>;

    /// Stamps `last_sign_in_at`, once both factors have been accepted.
    ///
    /// Deliberately **not** merged into [`Staff::record_totp_step`], although
    /// both run at the same moment: that one is a compare-and-swap whose
    /// `false` refuses the sign-in, and this one is bookkeeping whose failure
    /// must not. Merging them would make a stamp that could not be written
    /// look like a replayed code.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn record_sign_in(&self, id: &str, now: OffsetDateTime) -> Result<bool, DbError>;
}

#[async_trait]
impl Staff for crate::repository::PgRepositories {
    async fn create(&self, new: NewStaff) -> Result<(), DbError> {
        // THROUGH CRATESTACK. Every column of the table is named here, which
        // is what migration 0035's "no DEFAULT on any column a writer names"
        // buys: `cratestack-macros` drops every `@default(...)` field from
        // `CreateStaffMemberInput`, so a defaulted column would be one this literal
        // could not set and the row would carry whatever the DDL invented.
        self.cs
            .staff_member()
            .create(cratestack_schema::CreateStaffMemberInput {
                id: new.id,
                merchant_id: new.merchant_id,
                email: new.email,
                display_name: new.display_name,
                password_hash: new.password_hash,
                // The one-time password the CLI printed is a credential the
                // operator has seen; it stops being usable the moment its
                // owner picks their own.
                password_change_required: true,
                totp_secret: None,
                totp_enrolled_at: None,
                // The seed migration 0035 requires: `NULL < step` is NULL, so
                // a nullable column would refuse this person's first code
                // forever.
                last_totp_step: 0,
                status: StaffStatus::Active.as_wire_str().to_owned(),
                created_at: to_chrono(new.now),
                updated_at: to_chrono(new.now),
                last_sign_in_at: None,
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "create", error)))?;

        Ok(())
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<StaffRow>, DbError> {
        // `find_many().limit(1)` and not `find_unique`, because `email` is a
        // unique *index*, not the primary key, and `find_unique` takes the
        // key. The `LIMIT 1` is belt to the index's braces: with the unique
        // index in place there can only be one, and without it this must
        // still not decode an arbitrary number of accounts into memory
        // because somebody dropped a constraint.
        let rows = self
            .cs
            .staff_member()
            .find_many()
            .where_(staff_member::email().eq(email.to_owned()))
            .limit(1)
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))?;

        rows.into_iter().next().map(row_from_model).transpose()
    }

    async fn find(&self, id: &str) -> Result<Option<StaffRow>, DbError> {
        self.cs
            .staff_member()
            .find_unique(id.to_owned())
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))?
            .map(row_from_model)
            .transpose()
    }

    async fn enrol_totp(
        &self,
        id: &str,
        sealed_secret: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError> {
        // The guard is in the statement, not in Rust: two enrolment posts
        // racing must not both win, and the loser must not be the one whose
        // secret the person just scanned.
        let summary = self
            .cs
            .staff_member()
            .update_many()
            .where_(staff_member::id().eq(id.to_owned()))
            .where_(staff_member::totp_enrolled_at().is_null())
            .set(cratestack_schema::UpdateStaffMemberInput {
                totp_secret: Some(Some(sealed_secret.to_owned())),
                totp_enrolled_at: Some(Some(to_chrono(now))),
                updated_at: Some(to_chrono(now)),
                ..cratestack_schema::UpdateStaffMemberInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        Ok(summary.ok == 1)
    }

    async fn record_totp_step(
        &self,
        id: &str,
        step: i64,
        now: OffsetDateTime,
    ) -> Result<bool, DbError> {
        // `lt(step)` on the *stored* value, i.e. "the stored step is below
        // this one". That is the compare half of the compare-and-swap and it
        // is the reason `last_totp_step` is NOT NULL: `NULL < step` is NULL,
        // so a nullable column would refuse every first code.
        let summary = self
            .cs
            .staff_member()
            .update_many()
            .where_(staff_member::id().eq(id.to_owned()))
            .where_(staff_member::last_totp_step().lt(step))
            .set(cratestack_schema::UpdateStaffMemberInput {
                last_totp_step: Some(step),
                updated_at: Some(to_chrono(now)),
                ..cratestack_schema::UpdateStaffMemberInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        Ok(summary.ok == 1)
    }

    async fn set_password(
        &self,
        id: &str,
        password_hash: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError> {
        let summary = self
            .cs
            .staff_member()
            .update_many()
            .where_(staff_member::id().eq(id.to_owned()))
            .set(cratestack_schema::UpdateStaffMemberInput {
                password_hash: Some(password_hash.to_owned()),
                password_change_required: Some(false),
                updated_at: Some(to_chrono(now)),
                ..cratestack_schema::UpdateStaffMemberInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        Ok(summary.ok == 1)
    }

    async fn record_sign_in(&self, id: &str, now: OffsetDateTime) -> Result<bool, DbError> {
        let summary = self
            .cs
            .staff_member()
            .update_many()
            .where_(staff_member::id().eq(id.to_owned()))
            .set(cratestack_schema::UpdateStaffMemberInput {
                last_sign_in_at: Some(Some(to_chrono(now))),
                updated_at: Some(to_chrono(now)),
                ..cratestack_schema::UpdateStaffMemberInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        Ok(summary.ok == 1)
    }
}

/// The generated model row, in vpay's own types.
///
/// The one fallible step is [`StaffStatus::parse`]: a stored value outside
/// the vocabulary is [`DbError::StaffStatusUnknown`] rather than a default,
/// for the reason that function's doc gives.
fn row_from_model(model: cratestack_schema::models::StaffMember) -> Result<StaffRow, DbError> {
    let status = StaffStatus::parse(&model.status).ok_or_else(|| DbError::StaffStatusUnknown {
        id: model.id.clone(),
        status: model.status.clone(),
    })?;

    Ok(StaffRow {
        id: model.id,
        merchant_id: model.merchant_id,
        email: model.email,
        display_name: model.display_name,
        password_hash: model.password_hash,
        password_change_required: model.password_change_required,
        totp_secret: model.totp_secret,
        totp_enrolled_at: model.totp_enrolled_at.map(from_chrono),
        last_totp_step: model.last_totp_step,
        status,
        created_at: from_chrono(model.created_at),
        updated_at: from_chrono(model.updated_at),
        last_sign_in_at: model.last_sign_in_at.map(from_chrono),
    })
}

/// vpay's `time` instant, in the `chrono` type every generated input takes.
///
/// The same one-way boundary [`crate::client_assertion`] and
/// [`crate::customers`] already keep: chrono is authkestra's and CrateStack's
/// convention, `time` is vpay's, and the conversion happens at the edge and
/// nowhere else.
pub(crate) fn to_chrono(at: OffsetDateTime) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(at.unix_timestamp(), at.nanosecond())
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MAX_UTC)
}

/// The other direction, for a column read back out of a generated model.
///
/// `MAX` on an out-of-range value rather than a panic: this crate denies
/// `unwrap`, and a timestamp Postgres accepted but `time` cannot represent is
/// a row that must still be readable — clamping makes an expired session read
/// as *further* expired, which is the safe direction for every consumer here.
pub(crate) fn from_chrono(at: chrono::DateTime<chrono::Utc>) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(at.timestamp_nanos_opt().unwrap_or(0)))
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `@@allow` slot this module's calls need is occupied — asked of
    /// the descriptor rather than of a database, so it costs milliseconds.
    ///
    /// It is not a substitute for the container tests: a non-empty slot does
    /// not say the policy admits *this* caller, which is what
    /// `auth().isSystem()` has to get right. What it removes is the wait to
    /// learn that a slot is **empty**, and an empty slot is silent on two of
    /// the four actions here — including the TOTP replay guard. See
    /// `disabled_clients`' equivalent, which this copies deliberately.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use cratestack_schema::models::STAFF_MEMBER_MODEL as descriptor;

        assert!(
            !descriptor.read_allow_policies.is_empty(),
            "model StaffMember lost @@allow(\"read\", …): find_by_email would answer `no such \
             account` for every address, because a read policy is compiled into the WHERE \
             clause and an empty allow list renders FALSE"
        );
        assert!(
            !descriptor.create_allow_policies.is_empty(),
            "model StaffMember lost @@allow(\"create\", …): `vpay-server staff add` would fail with \
             a Forbidden naming the model"
        );
        assert!(
            !descriptor.update_allow_policies.is_empty(),
            "model StaffMember lost @@allow(\"update\", …): record_totp_step is an update_many, its \
             policy is part of the statement's own WHERE, and an empty allow list renders \
             FALSE — the TOTP replay guard would match zero rows and answer Ok(false) with no \
             error anywhere"
        );
        assert!(
            !descriptor.delete_allow_policies.is_empty(),
            "model StaffMember lost @@allow(\"delete\", …)"
        );
        assert!(
            descriptor.read_deny_policies.is_empty()
                && descriptor.create_deny_policies.is_empty()
                && descriptor.update_deny_policies.is_empty()
                && descriptor.delete_deny_policies.is_empty(),
            "a @@deny arm appeared on model StaffMember; every deny is evaluated ahead of every \
             allow, so one added here refuses the system principal this crate runs as"
        );
    }

    /// The vocabulary round-trips, and nothing outside it decodes.
    ///
    /// The `None` half is the one that matters: `StaffStatus::parse` must not
    /// have a default, because the first variant is `Active` and a default
    /// would read a row the database stopped constraining as *permitted to
    /// sign in*.
    #[test]
    fn the_status_vocabulary_round_trips_and_admits_nothing_else() {
        for status in [StaffStatus::Active, StaffStatus::Disabled] {
            assert_eq!(StaffStatus::parse(status.as_wire_str()), Some(status));
        }
        assert_eq!(StaffStatus::parse("Active"), None, "case matters");
        assert_eq!(StaffStatus::parse(""), None);
        assert_eq!(StaffStatus::parse("deleted"), None);
    }

    /// Neither credential nor the sign-in identifier reaches a `{:?}`.
    #[test]
    fn debug_shows_no_credential_and_no_address() {
        let row = StaffRow {
            id: "stf_1".to_owned(),
            merchant_id: "acct_1".to_owned(),
            email: "ada@example.test".to_owned(),
            display_name: "Ada Lovelace".to_owned(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".to_owned(),
            password_change_required: false,
            totp_secret: Some("c2VhbGVk".to_owned()),
            totp_enrolled_at: Some(OffsetDateTime::UNIX_EPOCH),
            last_totp_step: 42,
            status: StaffStatus::Active,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            last_sign_in_at: None,
        };

        let rendered = format!("{row:?}");
        for secret in [
            "ada@example.test",
            "Ada Lovelace",
            "$argon2id",
            "aGFzaA",
            "c2VhbGVk",
        ] {
            assert!(
                !rendered.contains(secret),
                "StaffRow's Debug leaked {secret}: {rendered}"
            );
        }
        assert!(rendered.contains("stf_1"), "the id is what a log needs");
        assert!(rendered.contains("acct_1"));
    }
}
