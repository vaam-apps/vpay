//! The `staff_members` repository (`backends/migrations/0035_create-staff-auth.sql`,
//! `0044_create-credentials.sql`) — who may sign in to `/dash/v1`, and which
//! merchant they may read as
//! ([ADR-0017](../../../../docs/adr/0017-staff-authentication.md),
//! [ADR-0019](../../../../docs/adr/0019-credential-model.md)).
//!
//! # The credentials are not here any more
//!
//! `password_hash`, `password_change_required`, `totp_secret`,
//! `totp_enrolled_at` and `last_totp_step` moved to [`crate::credentials`] in
//! migration 0044, and with them `enrol_totp`, `record_totp_step` and
//! `set_password`. What is left is a trait about **people**: who they are,
//! which merchant they belong to, whether they may sign in at all, and when
//! they last did.
//!
//! `last_sign_in_at` stayed, deliberately: it is when this *person* last
//! completed a full sign-in, across every credential they hold and across the
//! two factors one sign-in presents. `merchant_id` and `is_admin` stayed for
//! the reason ADR-0019 decision 7 gives — they are who the person may read
//! as, not how they proved who they are.
//!
//! # Every method here runs through CrateStack, and that is the point
//!
//! [`crate::disabled_clients`] moved three methods and [`crate::customers`]
//! two of seven; this module moves all four, because migration 0035 was shaped
//! so it could — no `jsonb`, no `bytea`, no native enum, no `DEFAULT` on any
//! column a writer names. `docs/reference/vpay-db.md` § CrateStack carries the
//! general account; what is specific here is that the *security* properties
//! are now carried by generated statements, so the four `@@allow` arms on
//! `model StaffMember` are load-bearing in a way no previous model's were. Two of
//! the four fail **silently**, which is why
//! [`tests::every_action_this_module_calls_has_an_allow_arm`] exists and runs
//! without a container. The *dangerous* one of those two moved with the
//! replay guard: see `crate::credentials`' equivalent test.
//!
//! # What this module does not know
//!
//! It never hashes, never verifies and never decrypts — and since migration
//! 0044 it never *sees* a credential at all. That property, and the replay
//! guard that was its one exception, moved whole to [`crate::credentials`].

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
/// `Debug` is **hand-written** below: two of these fields are personal data,
/// and `{:?}` on this struct reaches `tracing` fields, `anyhow` chains and
/// every failing assertion's output. It said "three of these fields are
/// credentials or personal data" until migration 0044 took the credentials
/// to [`crate::credentials::CredentialRow`], which carries the same rule and
/// its own test.
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
    /// Whether this account may sign in.
    pub status: StaffStatus,
    /// A cross-tenant read grant
    /// ([ADR-0018](../../../../docs/adr/0018-cross-tenant-admin-reads.md)):
    /// may this person's `/dash/v1` session read a merchant other than the
    /// one it is bound to, by naming one in `?merchant_id=`? `false` for
    /// every row `staff add` writes unless `--admin` is passed, and for
    /// every row this table held before migration `0043` (backfilled to the
    /// safe answer, not inferred). Not sensitive — it is not a credential —
    /// so [`fmt::Debug`] below shows it plainly.
    pub is_admin: bool,
    /// When the row was created.
    pub created_at: OffsetDateTime,
    /// When it last changed.
    pub updated_at: OffsetDateTime,
    /// When this person last completed a full sign-in. Not "last seen" —
    /// that is `staff_sessions.last_seen_at`.
    pub last_sign_in_at: Option<OffsetDateTime>,
}

/// Redacts the email address and the display name.
///
/// The address is personal data and is also the sign-in identifier, so a log
/// line carrying it hands a reader half of a credential pair. The id, the
/// merchant and the status are what an operator debugging this table actually
/// needs, and none of them is any of those things.
///
/// There is no credential left on this struct to redact — migration 0044 took
/// them — but the *rule* did not move: `crate::credentials::CredentialRow`
/// has its own hand-written `Debug` and its own test.
impl fmt::Debug for StaffRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaffRow")
            .field("id", &self.id)
            .field("merchant_id", &self.merchant_id)
            .field("email", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("status", &self.status)
            .field("is_admin", &self.is_admin)
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
    /// [`StaffRow::is_admin`]'s doc. `false` unless the operator passed
    /// `--admin` — a plain `clap` default, not a database one: migration
    /// `0043` gives this column no `DEFAULT`, on the same rule every other
    /// column here follows, so a caller that forgot this field is a compile
    /// error at this literal rather than a row the database half-filled.
    pub is_admin: bool,
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
            .field("is_admin", &self.is_admin)
            .field("now", &self.now)
            .finish()
    }
}

/// Reads and writes of the `staff_members` table.
///
/// Three methods, every one of them through CrateStack. There is deliberately
/// **no** `list`, no `disable` and no `delete`, and since VP-6a no
/// `record_sign_in` either: the only writer of this table is the operator CLI
/// (`staff add`) and the just-in-time provisioning transaction a first pairing
/// through vaam runs ([`provision_federated_in_tx`]), and a repository method
/// with no caller is a claim `docs/status.md` would have to carry.
///
/// It was six until migration 0044 moved the credentials out, and five until
/// migration 0056 retired the session that `record_sign_in` stamped and the
/// compensating `delete` that `staff add`'s second insert (the password) needed.
#[async_trait]
pub trait Staff {
    /// Inserts one staff member. The operator CLI is the only caller, and
    /// there is no HTTP endpoint that reaches this.
    ///
    /// `create`, not `upsert`: a caller that supplies an id already in the table
    /// must raise, not overwrite. A second `staff add` for an address that
    /// already exists fails on `staff_members_email_key` whichever builder is
    /// used; the builder choice is what refuses the other case (an id already
    /// present), asserted by
    /// `a_second_create_for_one_staff_id_is_refused_rather_than_overwriting` in
    /// `vpay-db/tests/repositories.rs`.
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
    /// # Errors
    ///
    /// [`DbError::StaffStatusUnknown`] if the stored `status` is outside the
    /// vocabulary (only reachable if `staff_members_status_is_known` were dropped),
    /// [`DbError::Persistence`] otherwise.
    async fn find_by_email(&self, email: &str) -> Result<Option<StaffRow>, DbError>;

    /// The account for an id, or `None`. The signed transport's read: every
    /// `/dash/v1` request re-reads the staff row rather than trusting what a
    /// key recorded, which is what makes disabling an account take effect on
    /// the next request.
    ///
    /// # Errors
    ///
    /// [`Staff::find_by_email`]'s.
    async fn find(&self, id: &str) -> Result<Option<StaffRow>, DbError>;
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
                status: StaffStatus::Active.as_wire_str().to_owned(),
                is_admin: new.is_admin,
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
}

/// Creates a staff member **and** their federated link in the caller's
/// transaction — the one write a first pairing through vaam makes (VP-6a,
/// [ADR-0036](../../../../docs/adr/0036-the-console-is-a-signed-message-under-a-paired-key.md);
/// the pattern is ADR-0030's just-in-time provisioning).
///
/// # Why this is a transaction
///
/// CrateStack's `run_in_tx` runs the *same* generated statements on a
/// transaction the caller owns (`customers::erase_in_tx` and
/// `webhook_deliveries::create_in_tx` already do), so a person provisioned by
/// their first pairing either has a member **and** the link that lets the next
/// pairing find them, or has neither: never a member holding an address
/// nobody else can now be given and no link to reach it by.
///
/// A unique violation on `staff_members_email_key` (the address belongs to
/// somebody else) or on `credentials_federated_identity_key` (a concurrent
/// first pairing of the same identity won) rolls the whole thing back and
/// reaches the caller as [`crate::PersistenceError::Unique`].
///
/// # Errors
///
/// [`DbError::Persistence`].
pub(crate) async fn provision_federated_in_tx(
    cs: &cratestack_schema::Cratestack,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    staff: &NewStaff,
    credential: crate::credentials::NewCredential,
) -> Result<(), DbError> {
    cs.staff_member()
        .create(cratestack_schema::CreateStaffMemberInput {
            id: staff.id.clone(),
            merchant_id: staff.merchant_id.clone(),
            email: staff.email.clone(),
            display_name: staff.display_name.clone(),
            status: StaffStatus::Active.as_wire_str().to_owned(),
            is_admin: staff.is_admin,
            created_at: to_chrono(staff.now),
            updated_at: to_chrono(staff.now),
            last_sign_in_at: None,
        })
        .run_in_tx(&mut *tx, &system_context())
        .await
        .map_err(|error| DbError::from(classify_cratestack(MODEL, "create", error)))?;

    cs.credential()
        .create(crate::credentials::create_input(credential))
        .run_in_tx(&mut *tx, &system_context())
        .await
        .map_err(|error| DbError::from(classify_cratestack("Credential", "create", error)))?;

    Ok(())
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
        status,
        is_admin: model.is_admin,
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
            "model StaffMember lost @@allow(\"update\", …)"
        );
        assert!(
            descriptor.delete_allow_policies.is_empty(),
            "model StaffMember gained @@allow(\"delete\", …): nothing deletes a staff member, \
             and a delete cascades their console keys and their federated link"
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

    /// The sign-in identifier does not reach a `{:?}`.
    ///
    /// There is no credential left on this struct to leak — migration 0044
    /// took them — and the half of this test that used to assert that lives in
    /// `crate::credentials::tests::debug_redacts_material_and_shows_the_public_identity`
    /// now. The address is still here and is still half of a credential pair.
    #[test]
    fn debug_shows_no_credential_and_no_address() {
        let row = StaffRow {
            id: "stf_1".to_owned(),
            merchant_id: "acct_1".to_owned(),
            email: "ada@example.test".to_owned(),
            display_name: "Ada Lovelace".to_owned(),
            status: StaffStatus::Active,
            is_admin: false,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            last_sign_in_at: None,
        };

        let rendered = format!("{row:?}");
        for secret in ["ada@example.test", "Ada Lovelace"] {
            assert!(
                !rendered.contains(secret),
                "StaffRow's Debug leaked {secret}: {rendered}"
            );
        }
        assert!(rendered.contains("stf_1"), "the id is what a log needs");
        assert!(rendered.contains("acct_1"));
    }
}
