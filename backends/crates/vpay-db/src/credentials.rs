//! The `credentials` repository (`backends/migrations/0044_create-credentials.sql`)
//! — how a subject proved who they are, as its own object
//! ([ADR-0019](../../../../docs/adr/0019-credential-model.md)).
//!
//! # What this module does not know, and that is the whole design
//!
//! It **never hashes, never verifies and never decrypts**. [`CredentialRow::material`]
//! is an opaque string on the way in and on the way out, whatever the kind:
//! an argon2id PHC string for `password`, an AES-256-GCM sealed RFC 6238
//! secret for `totp`, a base64url COSE public key for `webauthn` when that
//! exists. `vpay_api::staff_auth` owns argon2id, RFC 6238 and the AEAD,
//! because those need deployment secrets this crate has no business holding.
//!
//! That property is [`crate::staff`]'s, carried over verbatim in spirit — it
//! stated it for two named columns, and the point of the split is that it now
//! holds for a column whose contents this module could not interpret even if
//! it wanted to. It extends one step further than it used to: this module
//! also **never fetches a JWKS**. A federated credential's material is the
//! issuer's public keys, which live behind `vpay_api`'s `jwks_cache`, and
//! this table holds only the `(issuer, subject)` pair that names the identity.
//!
//! The one credential rule that **is** here is the replay guard, and it is
//! here for the reason it always was: it is a compare-and-swap on a row, and
//! two concurrent presentations of the same six digits are exactly the race a
//! read-then-write would lose. See [`Credentials::advance_counter`].
//!
//! # Declaring a kind is not implementing it
//!
//! [`CredentialKind`] has **eight** variants and this deployment can reach
//! **two**: [`CredentialKind::Password`] and [`CredentialKind::Totp`].
//! Nothing mints any of the other six and `vpay_api::staff_auth` has a
//! verifier for two. They are shapes the schema can hold. A reader who finds
//! [`CredentialKind::Webauthn`] here must not conclude vpay supports WebAuthn;
//! `docs/status.md` carries the gap.
//!
//! # A subject may hold no credential at all
//!
//! There is no invariant that a staff member has a credential and none that
//! they have a password — no `NOT NULL`, no CHECK, and nothing in this module
//! that assumes one. Just-in-time SSO provisioning creates a member who has
//! never had a password and never will, so every read here answers `None`
//! rather than erroring, and the caller decides what a missing credential
//! means on its own path.

use std::fmt;

use async_trait::async_trait;
use time::OffsetDateTime;

use crate::error::DbError;
use crate::persistence::{classify_cratestack, system_context};
use crate::schema::cratestack_schema::{self, credential};
use crate::staff::{from_chrono, to_chrono};

/// The `.cstack` model these calls name, for
/// [`crate::persistence::classify_cratestack`]'s `model` slot.
const MODEL: &str = "Credential";

/// What a credential is, and therefore how something above this crate would
/// verify it.
///
/// A Rust enum over a `TEXT` column with a hand-named CHECK — `staff_members.status`'
/// shape and for its reason: CrateStack decodes every enum column with
/// `try_get::<String>()`, so a native Postgres enum fails to decode on every
/// read (upstream #228, the defect migration 0032 had to convert
/// `providers.flow` out of).
///
/// The vocabulary is closed twice: by `credentials_kind_is_known` at the
/// database and by [`CredentialKind::parse`] refusing anything else here. A
/// value in one and not the other is a row this crate declines to decode
/// rather than a row it guesses about — and unlike
/// [`crate::staff::StaffStatus`], there is no "safe default" that would even
/// be tempting here, because every default is a claim about how to verify
/// material this module cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CredentialKind {
    /// An argon2id PHC string, peppered from configuration. **Implemented.**
    Password,
    /// The AES-256-GCM sealed RFC 6238 shared secret. **Implemented.**
    Totp,
    /// RFC 4226's counter-based cousin. Declared, not implemented.
    Hotp,
    /// A single-use link mailed to the subject. Declared, not implemented;
    /// `credentials_transient_kinds_expire` refuses one with no `expires_at`.
    MagicLink,
    /// A one-time code mailed to the subject. Declared, not implemented.
    EmailOtp,
    /// A one-time code sent to the subject's phone. Declared, not
    /// implemented.
    PhoneOtp,
    /// A WebAuthn credential's public key, base64url. Declared, not
    /// implemented. **Many per subject are correct**, which is why no
    /// uniqueness index names this kind.
    Webauthn,
    /// A link to an identity another issuer asserts: `(iss, sub)`, and no
    /// secret at all. Declared, not implemented.
    ///
    /// This is the kind the whole model is shaped around, and it is the one
    /// that makes [`CredentialRow::material`] an `Option`. A federated
    /// identity **is not a secret**: `(iss, sub)` is public, and verification
    /// means "a token signed by that issuer validates against its JWKS and
    /// its `sub` equals this row's". A model that assumed every credential
    /// had secret material to check would not fit it, and what happens next
    /// is that somebody crams an issuer into a `password_hash` column.
    Oidc,
}

/// The stored spelling of [`CredentialKind::Password`], and one of the eight
/// `credentials_kind_is_known` admits.
const KIND_PASSWORD: &str = "password";
/// The stored spelling of [`CredentialKind::Totp`].
const KIND_TOTP: &str = "totp";
/// The stored spelling of [`CredentialKind::Hotp`].
const KIND_HOTP: &str = "hotp";
/// The stored spelling of [`CredentialKind::MagicLink`].
const KIND_MAGIC_LINK: &str = "magic_link";
/// The stored spelling of [`CredentialKind::EmailOtp`].
const KIND_EMAIL_OTP: &str = "email_otp";
/// The stored spelling of [`CredentialKind::PhoneOtp`].
const KIND_PHONE_OTP: &str = "phone_otp";
/// The stored spelling of [`CredentialKind::Webauthn`].
const KIND_WEBAUTHN: &str = "webauthn";
/// The stored spelling of [`CredentialKind::Oidc`].
const KIND_OIDC: &str = "oidc";

impl CredentialKind {
    /// Every variant, in the order `credentials_kind_is_known` lists them.
    ///
    /// Public because the vocabulary is a **contract with a CHECK
    /// constraint**, and a test that walks it is how the two are kept in
    /// step; `a_kind_spelled_here_is_a_kind_the_database_admits` in
    /// `vpay-db/tests/repositories.rs` inserts one row per entry.
    pub const ALL: [Self; 8] = [
        Self::Password,
        Self::Totp,
        Self::Hotp,
        Self::MagicLink,
        Self::EmailOtp,
        Self::PhoneOtp,
        Self::Webauthn,
        Self::Oidc,
    ];

    /// The stored spelling.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Password => KIND_PASSWORD,
            Self::Totp => KIND_TOTP,
            Self::Hotp => KIND_HOTP,
            Self::MagicLink => KIND_MAGIC_LINK,
            Self::EmailOtp => KIND_EMAIL_OTP,
            Self::PhoneOtp => KIND_PHONE_OTP,
            Self::Webauthn => KIND_WEBAUTHN,
            Self::Oidc => KIND_OIDC,
        }
    }

    /// The stored spelling, back.
    ///
    /// `None` for anything else, and the caller turns that into
    /// [`DbError::CredentialKindUnknown`] rather than a default —
    /// [`crate::staff::StaffStatus::parse`]'s rule, and here the consequence
    /// of a default would be worse: it would mean handing material of one
    /// kind to the verifier for another.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            KIND_PASSWORD => Some(Self::Password),
            KIND_TOTP => Some(Self::Totp),
            KIND_HOTP => Some(Self::Hotp),
            KIND_MAGIC_LINK => Some(Self::MagicLink),
            KIND_EMAIL_OTP => Some(Self::EmailOtp),
            KIND_PHONE_OTP => Some(Self::PhoneOtp),
            KIND_WEBAUTHN => Some(Self::Webauthn),
            KIND_OIDC => Some(Self::Oidc),
            _ => None,
        }
    }

    /// Whether at most one credential of this kind may exist per subject.
    ///
    /// Mirrors `credentials_one_singleton_kind_per_staff_member`'s predicate
    /// exactly, and is **not** what enforces it — the partial unique index
    /// is, and a container test proves that rather than this function. What
    /// this is for is the caller that wants to know whether a second
    /// `create` can succeed without asking Postgres.
    ///
    /// `false` for [`CredentialKind::Webauthn`] and the three transient kinds
    /// because **many of each is correct**: a person may enrol several
    /// security keys and may have several links in flight. `false` for
    /// [`CredentialKind::Oidc`] because account linking is many-to-one — one
    /// staff member may hold a Google link and an Entra link at once — and
    /// its uniqueness is a *different* rule, one link per issuer, carried by
    /// a different index.
    #[must_use]
    pub fn is_singleton_per_subject(self) -> bool {
        matches!(self, Self::Password | Self::Totp | Self::Hotp)
    }

    /// Whether this kind's material is a secret this deployment stores.
    ///
    /// `false` only for [`CredentialKind::Oidc`], and that single exception
    /// is the whole federation requirement expressed as a predicate.
    /// `credentials_federated_carries_identity_and_no_material` is what makes
    /// it true of the data rather than merely of this function.
    #[must_use]
    pub fn carries_material(self) -> bool {
        !matches!(self, Self::Oidc)
    }
}

/// One `credentials` row, exactly as stored.
///
/// `Debug` is **hand-written** below and is **per field, not per row**: see
/// its own doc for why a blanket rule would be wrong in both directions.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialRow {
    /// `cred_…`. Never rendered on any surface; an internal handle the login
    /// path carries between two statements.
    pub id: String,
    /// The staff member this credential authenticates, or `None` once a
    /// second subject type exists and this row names that one instead.
    /// `credentials_has_exactly_one_subject` is what keeps "exactly one" true.
    pub staff_member_id: Option<String>,
    /// What kind of proof this is.
    pub kind: CredentialKind,
    /// The opaque secret, or `None` for a kind that has none. Opaque here —
    /// this crate never interprets it.
    pub material: Option<String>,
    /// The Issuer Identifier, for [`CredentialKind::Oidc`] and nothing else.
    /// **Not a secret**, and not an allow-list: which issuers a deployment
    /// trusts is configuration, and a row whose issuer is not in it verifies
    /// nothing.
    pub issuer: Option<String>,
    /// The IdP's `sub`, for [`CredentialKind::Oidc`] and nothing else.
    /// **Not a secret.** Matched on; an `email` claim never is.
    pub subject: Option<String>,
    /// The RFC 6238 time step of the last accepted code, the RFC 4226
    /// counter, or `0`. Never `NULL`: see [`Credentials::advance_counter`].
    pub counter: i64,
    /// Whether this material was issued by an operator and must be replaced
    /// by material the subject chose. Only ever `true` on a password —
    /// `credentials_only_a_password_may_require_change`.
    pub must_change: bool,
    /// When a transient credential stops being presentable. `None` for every
    /// row this deployment can write, because nothing writes a transient kind.
    pub expires_at: Option<OffsetDateTime>,
    /// When the row was created. For a `totp` row this **is** the enrolment
    /// date: under the split, enrolment is the existence of the row.
    pub created_at: OffsetDateTime,
    /// When it last changed.
    pub updated_at: OffsetDateTime,
}

/// Redacts the material and **nothing else**.
///
/// The rule is **per field, not per kind**, and both halves of that are
/// deliberate.
///
/// `material` is redacted for every kind, including the ones whose material
/// is not very secret — a WebAuthn public key is public — because a per-kind
/// exception is a *branch*, and a branch that decides whether to print a
/// secret is one mis-edit away from printing an argon2id digest or a TOTP
/// seed. The column decides, not the row.
///
/// `issuer` and `subject` are shown, because they are **not secrets** and
/// they are exactly what an operator debugging a broken SSO link needs. A
/// blanket "redact everything on the credentials table" would hide them, and
/// that debugging session ends with somebody printing the whole row by hand —
/// which is strictly worse than printing two public identifiers.
impl fmt::Debug for CredentialRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialRow")
            .field("id", &self.id)
            .field("staff_member_id", &self.staff_member_id)
            .field("kind", &self.kind)
            .field("material", &self.material.as_ref().map(|_| "[redacted]"))
            .field("issuer", &self.issuer)
            .field("subject", &self.subject)
            .field("counter", &self.counter)
            .field("must_change", &self.must_change)
            .field("expires_at", &self.expires_at)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Everything a writer supplies for one credential row.
///
/// Every column, with no defaults anywhere — migration 0044 declares none, on
/// migration 0035's rule — so this struct is the *whole* insert and a column
/// added to the table without being added here is a compile error at the
/// `CreateCredentialInput` literal rather than a silently defaulted value.
#[derive(Clone, PartialEq, Eq)]
pub struct NewCredential {
    /// `cred_…`, from `vpay_core::ids::credential_id`.
    pub id: String,
    /// The subject. `Option` because the column is, for the reason
    /// [`CredentialRow::staff_member_id`] gives.
    pub staff_member_id: Option<String>,
    /// What kind of proof this is.
    pub kind: CredentialKind,
    /// The opaque secret, or `None` for [`CredentialKind::Oidc`].
    /// `credentials_federated_carries_identity_and_no_material` refuses the
    /// two wrong combinations at the insert.
    pub material: Option<String>,
    /// The Issuer Identifier, for [`CredentialKind::Oidc`] only.
    pub issuer: Option<String>,
    /// The IdP's `sub`, for [`CredentialKind::Oidc`] only.
    pub subject: Option<String>,
    /// Whether this material must be replaced by material the subject chose.
    pub must_change: bool,
    /// When a transient credential stops being presentable. Required for
    /// `magic_link`, `email_otp` and `phone_otp` — the database refuses the
    /// insert without it.
    pub expires_at: Option<OffsetDateTime>,
    /// When the row is created; also its `updated_at`.
    pub now: OffsetDateTime,
}

/// Redacts for [`CredentialRow`]'s reason, with the same per-field split.
impl fmt::Debug for NewCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewCredential")
            .field("id", &self.id)
            .field("staff_member_id", &self.staff_member_id)
            .field("kind", &self.kind)
            .field("material", &self.material.as_ref().map(|_| "[redacted]"))
            .field("issuer", &self.issuer)
            .field("subject", &self.subject)
            .field("must_change", &self.must_change)
            .field("expires_at", &self.expires_at)
            .field("now", &self.now)
            .finish()
    }
}

/// Reads and writes of the `credentials` table.
///
/// Four methods, every one of them through CrateStack, and every one of them
/// **credential-agnostic**: none takes or returns anything but an opaque
/// string, a kind and a counter.
#[async_trait]
pub trait Credentials {
    /// Inserts one credential.
    ///
    /// `create`, not `upsert`, and the choice is load-bearing on this table
    /// in a way it was only defensive on `staff_members`. An upsert would
    /// make a second enrolment **silently replace a working second factor**
    /// with one the caller had just been shown, which is a second-factor
    /// reset with no authentication in front of it. A create raises instead,
    /// and the partial unique index
    /// `credentials_one_singleton_kind_per_staff_member` is what it collides
    /// with.
    ///
    /// That index **replaces** `Staff::enrol_totp`'s old
    /// `totp_enrolled_at IS NULL` guard. The guard was a `WHERE` clause a
    /// future edit could drop with nothing to say so; the index is the
    /// database refusing.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`] — [`crate::PersistenceError::Unique`] when
    /// the id is taken or a per-kind uniqueness rule fires,
    /// [`crate::PersistenceError::Check`] when the kind is outside the
    /// vocabulary, when a bound is exceeded, or when the material/identity
    /// combination does not match the kind,
    /// [`crate::PersistenceError::Denied`] if `model Credential` loses its
    /// `@@allow("create", …)` (loud: the create path evaluates its policies
    /// in Rust before any SQL), [`crate::PersistenceError::Backend`]
    /// otherwise.
    async fn create(&self, new: NewCredential) -> Result<(), DbError>;

    /// The credential of one kind held by one staff member, or `None`.
    ///
    /// **`None` is an ordinary answer and not an error**, because a subject
    /// may hold no credential of a given kind and, after just-in-time SSO
    /// provisioning, may hold no secret-bearing credential at all. A caller
    /// that treats `None` as "impossible" is a caller that will panic on the
    /// first federated staff member.
    ///
    /// Only meaningful for a kind where
    /// [`CredentialKind::is_singleton_per_subject`] is `true`: for the
    /// others, several rows are correct and this answers with whichever the
    /// index returns first. It is deliberately not `find_all_for` — nothing
    /// needs that yet, and a repository method with no caller is a claim
    /// `docs/status.md` would have to carry.
    ///
    /// # Errors
    ///
    /// [`DbError::CredentialKindUnknown`] if the stored `kind` is outside the
    /// vocabulary (only reachable if `credentials_kind_is_known` were
    /// dropped), [`DbError::Persistence`] otherwise.
    async fn find_for_staff_member(
        &self,
        staff_member_id: &str,
        kind: CredentialKind,
    ) -> Result<Option<CredentialRow>, DbError>;

    /// **The replay guard.** Records `counter` as the last accepted one, but
    /// only if it is strictly greater than what is stored.
    ///
    /// `false` means the code was already spent — the caller refuses the
    /// sign-in. This is the whole of the replay defence and it is a
    /// compare-and-swap on a row rather than a check in Rust, because two
    /// concurrent presentations of the same six digits are exactly the race a
    /// read-then-write would lose.
    ///
    /// It is `crate::staff::Staff::record_totp_step` moved, with its
    /// reasoning intact and its name generalised: the same statement serves
    /// RFC 6238's time step and RFC 4226's counter, because both are
    /// "strictly greater than the last accepted".
    ///
    /// # Why a missing `@@allow("update", …)` is the most dangerous edit in
    /// this crate
    ///
    /// `update_many`'s policy is compiled into the statement's own `WHERE`,
    /// so an empty allow list renders `FALSE`, the statement matches zero
    /// rows and this returns `Ok(false)` — **with no error anywhere**.
    /// `Ok(false)` is refuse-the-code, so the immediate effect is fail-closed
    /// and nobody can sign in; the danger is the fix somebody reaches for
    /// when every sign-in starts failing. It is pinned by
    /// [`tests::every_action_this_module_calls_has_an_allow_arm`], which
    /// names the slot rather than the symptom.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn advance_counter(
        &self,
        id: &str,
        counter: i64,
        now: OffsetDateTime,
    ) -> Result<bool, DbError>;

    /// Replaces the opaque material and clears [`CredentialRow::must_change`].
    ///
    /// `false` means no such credential. `crate::staff::Staff::set_password`
    /// moved and generalised: it takes no "old material" parameter for that
    /// method's reason — the caller has already authenticated the session
    /// making the change, and re-checking material this method would then
    /// overwrite would be a second copy of that check in the wrong layer.
    ///
    /// Note what `false` now means that it could not before. It used to mean
    /// "no such staff member", which the caller had already ruled out. It now
    /// **also** means "this staff member holds no credential of that kind",
    /// which is reachable the moment a member is provisioned through SSO and
    /// has never had a password. A caller must render that as a refusal, not
    /// as an impossibility.
    ///
    /// # Errors
    ///
    /// [`DbError::Persistence`].
    async fn replace_material(
        &self,
        id: &str,
        material: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError>;
}

#[async_trait]
impl Credentials for crate::repository::PgRepositories {
    async fn create(&self, new: NewCredential) -> Result<(), DbError> {
        // THROUGH CRATESTACK. Every column of the table is named here, which
        // is what migration 0044's "no DEFAULT on any column a writer names"
        // buys: `cratestack-macros` drops every `@default(...)` field from
        // `CreateCredentialInput`, so a defaulted column would be one this
        // literal could not set and the row would carry whatever the DDL
        // invented.
        self.cs
            .credential()
            .create(cratestack_schema::CreateCredentialInput {
                id: new.id,
                staff_member_id: new.staff_member_id,
                kind: new.kind.as_wire_str().to_owned(),
                material: new.material,
                issuer: new.issuer,
                subject: new.subject,
                // The seed migration 0044 requires, and the reason it is not
                // nullable: `NULL < step` is NULL in SQL, so a nullable
                // column would refuse this subject's first code forever.
                counter: 0,
                must_change: new.must_change,
                expires_at: new.expires_at.map(to_chrono),
                created_at: to_chrono(new.now),
                updated_at: to_chrono(new.now),
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "create", error)))?;

        Ok(())
    }

    async fn find_for_staff_member(
        &self,
        staff_member_id: &str,
        kind: CredentialKind,
    ) -> Result<Option<CredentialRow>, DbError> {
        // `find_many().limit(1)` and not `find_unique`, because the key here
        // is `(staff_member_id, kind)` under a partial unique index and not
        // the primary key. `Staff::find_by_email`'s argument for the LIMIT
        // holds verbatim: with the index in place there can only be one, and
        // without it this must still not decode an arbitrary number of
        // credentials into memory because somebody dropped a constraint.
        let rows = self
            .cs
            .credential()
            .find_many()
            .where_(credential::staff_member_id().eq(staff_member_id.to_owned()))
            .where_(credential::kind().eq(kind.as_wire_str().to_owned()))
            .limit(1)
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))?;

        rows.into_iter().next().map(row_from_model).transpose()
    }

    async fn advance_counter(
        &self,
        id: &str,
        counter: i64,
        now: OffsetDateTime,
    ) -> Result<bool, DbError> {
        // `lt(counter)` on the *stored* value, i.e. "the stored counter is
        // below this one". That is the compare half of the compare-and-swap
        // and it is the reason the column is NOT NULL: `NULL < counter` is
        // NULL, so a nullable column would refuse every first code.
        let summary = self
            .cs
            .credential()
            .update_many()
            .where_(credential::id().eq(id.to_owned()))
            .where_(credential::counter().lt(counter))
            .set(cratestack_schema::UpdateCredentialInput {
                counter: Some(counter),
                updated_at: Some(to_chrono(now)),
                ..cratestack_schema::UpdateCredentialInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        Ok(summary.ok == 1)
    }

    async fn replace_material(
        &self,
        id: &str,
        material: &str,
        now: OffsetDateTime,
    ) -> Result<bool, DbError> {
        let summary = self
            .cs
            .credential()
            .update_many()
            .where_(credential::id().eq(id.to_owned()))
            .set(cratestack_schema::UpdateCredentialInput {
                material: Some(Some(material.to_owned())),
                must_change: Some(false),
                updated_at: Some(to_chrono(now)),
                ..cratestack_schema::UpdateCredentialInput::default()
            })
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "update", error)))?;

        Ok(summary.ok == 1)
    }
}

/// The generated model row, in vpay's own types.
///
/// The one fallible step is [`CredentialKind::parse`]: a stored value outside
/// the vocabulary is [`DbError::CredentialKindUnknown`] rather than a
/// default, for the reason that function's doc gives.
fn row_from_model(model: cratestack_schema::models::Credential) -> Result<CredentialRow, DbError> {
    let kind =
        CredentialKind::parse(&model.kind).ok_or_else(|| DbError::CredentialKindUnknown {
            id: model.id.clone(),
            kind: model.kind.clone(),
        })?;

    Ok(CredentialRow {
        id: model.id,
        staff_member_id: model.staff_member_id,
        kind,
        material: model.material,
        issuer: model.issuer,
        subject: model.subject,
        counter: model.counter,
        must_change: model.must_change,
        expires_at: model.expires_at.map(from_chrono),
        created_at: from_chrono(model.created_at),
        updated_at: from_chrono(model.updated_at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `@@allow` slot this module's calls need is occupied — asked of
    /// the descriptor rather than of a database, so it costs milliseconds.
    ///
    /// [`crate::staff`]'s equivalent, moved with the thing that makes it
    /// urgent: the TOTP replay guard is [`Credentials::advance_counter`] now,
    /// so the silent-and-dangerous `update` slot is **this** model's.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use cratestack_schema::models::CREDENTIAL_MODEL as descriptor;

        assert!(
            !descriptor.read_allow_policies.is_empty(),
            "model Credential lost @@allow(\"read\", …): find_for_staff_member would answer `no \
             such credential` for every subject, because a read policy is compiled into the \
             WHERE clause and an empty allow list renders FALSE — every sign-in would fail"
        );
        assert!(
            !descriptor.create_allow_policies.is_empty(),
            "model Credential lost @@allow(\"create\", …): `vpay-server staff add` would fail \
             with a Forbidden naming the model, and TOTP enrolment with it"
        );
        assert!(
            !descriptor.update_allow_policies.is_empty(),
            "model Credential lost @@allow(\"update\", …): advance_counter is an update_many, its \
             policy is part of the statement's own WHERE, and an empty allow list renders \
             FALSE — the TOTP replay guard would match zero rows and answer Ok(false) with no \
             error anywhere"
        );
        assert!(
            !descriptor.delete_allow_policies.is_empty(),
            "model Credential lost @@allow(\"delete\", …): customer erasure will need it, and an \
             anonymised customer's row STAYS, so no cascade covers it"
        );
        assert!(
            descriptor.read_deny_policies.is_empty()
                && descriptor.create_deny_policies.is_empty()
                && descriptor.update_deny_policies.is_empty()
                && descriptor.delete_deny_policies.is_empty(),
            "a @@deny arm appeared on model Credential; every deny is evaluated ahead of every \
             allow, so one added here refuses the system principal this crate runs as"
        );
    }

    /// The vocabulary round-trips, and nothing outside it decodes.
    ///
    /// The `None` half is the one that matters. `CredentialKind::parse` must
    /// not have a default, and the consequence of one here is worse than
    /// `StaffStatus`': a default kind means handing material of one kind to
    /// the verifier for another.
    #[test]
    fn the_kind_vocabulary_round_trips_and_admits_nothing_else() {
        for kind in CredentialKind::ALL {
            assert_eq!(CredentialKind::parse(kind.as_wire_str()), Some(kind));
        }
        assert_eq!(CredentialKind::parse("Password"), None, "case matters");
        assert_eq!(CredentialKind::parse(""), None);
        assert_eq!(CredentialKind::parse("magiclink"), None);
        assert_eq!(CredentialKind::parse("saml"), None);
    }

    /// Eight distinct spellings, and the two rules that are *not* one rule.
    ///
    /// The second assertion is the federation amendment written as a test:
    /// "one password per subject" and "many federated links per subject, at
    /// most one per issuer" are different rules, so `oidc` must **not** be a
    /// singleton kind — if it were, a staff member could hold a Google link
    /// or an Entra link but never both.
    #[test]
    fn the_uniqueness_rules_are_per_kind_and_are_not_one_rule() {
        let spellings: std::collections::HashSet<&str> = CredentialKind::ALL
            .iter()
            .map(|kind| kind.as_wire_str())
            .collect();
        assert_eq!(spellings.len(), 8, "two kinds share a stored spelling");

        for kind in [
            CredentialKind::Password,
            CredentialKind::Totp,
            CredentialKind::Hotp,
        ] {
            assert!(
                kind.is_singleton_per_subject(),
                "{kind:?} is one-per-subject"
            );
        }
        for kind in [
            CredentialKind::Webauthn,
            CredentialKind::MagicLink,
            CredentialKind::EmailOtp,
            CredentialKind::PhoneOtp,
            CredentialKind::Oidc,
        ] {
            assert!(
                !kind.is_singleton_per_subject(),
                "{kind:?} must admit several per subject — for Oidc that is account linking \
                 (a Google link AND an Entra link), and for Webauthn it is several keys"
            );
        }
    }

    /// A federated identity is not a secret, and exactly one kind is
    /// material-free.
    #[test]
    fn only_a_federated_credential_carries_no_material() {
        for kind in CredentialKind::ALL {
            assert_eq!(
                kind.carries_material(),
                kind != CredentialKind::Oidc,
                "{kind:?}: `material` is NULL for oidc and NOT NULL for every other kind — \
                 `credentials_federated_carries_identity_and_no_material` says so at the database"
            );
        }
    }

    /// **No credential material reaches a `{:?}`, and the two public
    /// identifiers still do.**
    ///
    /// Both halves are asserted because both are failures. A struct that
    /// derived `Debug` by accident puts an argon2id digest and a sealed TOTP
    /// seed into `tracing` fields, `anyhow` chains and every failing
    /// assertion's output. A struct that redacted the whole row would hide
    /// the issuer and subject an operator debugging a broken SSO link needs,
    /// and that ends with somebody printing the row by hand.
    #[test]
    fn debug_redacts_material_and_shows_the_public_identity() {
        let secret_bearing = CredentialRow {
            id: "cred_1".to_owned(),
            staff_member_id: Some("stf_1".to_owned()),
            kind: CredentialKind::Password,
            material: Some("$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".to_owned()),
            issuer: None,
            subject: None,
            counter: 42,
            must_change: false,
            expires_at: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };
        let rendered = format!("{secret_bearing:?}");
        for secret in ["$argon2id", "aGFzaA", "c2FsdA"] {
            assert!(
                !rendered.contains(secret),
                "CredentialRow's Debug leaked {secret}: {rendered}"
            );
        }
        assert!(rendered.contains("cred_1"), "the id is what a log needs");
        assert!(rendered.contains("stf_1"));
        assert!(rendered.contains("42"), "the counter is not a secret");

        // A sealed TOTP seed is material too, and the redaction is by column
        // rather than by kind precisely so this needs no second branch.
        let sealed = CredentialRow {
            kind: CredentialKind::Totp,
            material: Some("bm9uY2VzZWFsZWQ".to_owned()),
            ..secret_bearing.clone()
        };
        assert!(
            !format!("{sealed:?}").contains("bm9uY2VzZWFsZWQ"),
            "a sealed TOTP secret reached a Debug"
        );

        // And the other direction: an operator debugging SSO must be able to
        // read which identity a row names.
        let federated = CredentialRow {
            kind: CredentialKind::Oidc,
            material: None,
            issuer: Some("https://accounts.google.com".to_owned()),
            subject: Some("110169484474386276334".to_owned()),
            ..secret_bearing.clone()
        };
        let rendered = format!("{federated:?}");
        assert!(
            rendered.contains("https://accounts.google.com"),
            "the issuer is public and redacting it makes an SSO link undebuggable: {rendered}"
        );
        assert!(
            rendered.contains("110169484474386276334"),
            "the subject is public and is what an operator matches against the IdP: {rendered}"
        );
    }

    /// `NewCredential` is the *other* struct a secret passes through on its
    /// way to the database, and it redacts on the same rule.
    #[test]
    fn debug_on_the_insert_redacts_material_too() {
        let new = NewCredential {
            id: "cred_1".to_owned(),
            staff_member_id: Some("stf_1".to_owned()),
            kind: CredentialKind::Password,
            material: Some("$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".to_owned()),
            issuer: None,
            subject: None,
            must_change: true,
            expires_at: None,
            now: OffsetDateTime::UNIX_EPOCH,
        };
        let rendered = format!("{new:?}");
        assert!(
            !rendered.contains("$argon2id") && !rendered.contains("aGFzaA"),
            "NewCredential's Debug leaked the one-time password's hash: {rendered}"
        );
        assert!(rendered.contains("cred_1"));
    }
}
