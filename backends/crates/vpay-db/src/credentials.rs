//! The `credentials` repository (`backends/migrations/0044_create-credentials.sql`)
//! — how a subject proved who they are, as its own object
//! ([ADR-0019](../../../../docs/adr/0019-credential-model.md)).
//!
//! # What this module does not know
//!
//! It **never hashes, never verifies and never decrypts**, and since VP-6a
//! (migration 0056) it holds no secret at all: the `password` and `totp` rows
//! were deleted with the sign-in that checked them, and
//! `credentials_password_and_totp_are_retired` refuses a new one.
//! [`CredentialRow::material`] stays an opaque `Option<String>` for the kinds
//! the table can still hold.
//!
//! # What is live: one kind, [`CredentialKind::Oidc`]
//!
//! A federated link is `(issuer, subject)` and nothing secret. It is how a
//! person vaam vouches for is found again: the first pairing provisions a
//! staff member and writes the link (`issuer = "vaam"`, `subject = <the vaam
//! user id>`), and every later pairing finds the same member through
//! [`Credentials::find_federated`]. The accepted issuers are configuration, not
//! data: a row whose issuer the caller does not ask for verifies nothing.
//!
//! # Declaring a kind is not implementing it
//!
//! [`CredentialKind`] has **six** variants; one is reachable. The others are
//! shapes the schema can hold. A reader who finds
//! [`CredentialKind::Webauthn`] here must not conclude vpay supports WebAuthn;
//! `docs/status.md` carries the gap.
//!
//! # A subject may hold no credential at all
//!
//! There is no invariant that a staff member has a credential: `vpay-server
//! staff add` writes none, and a member created that way is reached only through
//! a console key (`crate::console_keys`).

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

/// The stored spelling of [`CredentialKind::Hotp`], one of the six names that
/// `credentials_kind_is_known` admits and `credentials_password_and_totp_are_retired`
/// leaves open (the other two of the eight, `password` and `totp`, are closed
/// by migration 0056).
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
    /// Every variant, in the order `credentials_kind_is_known` lists them (minus
    /// the two migration 0056 retired).
    ///
    /// Public because the vocabulary is a **contract with a CHECK
    /// constraint**, and a test that walks it is how the two are kept in
    /// step; `a_kind_spelled_here_is_a_kind_the_database_admits` in
    /// `vpay-db/tests/repositories.rs` inserts one row per entry.
    pub const ALL: [Self; 6] = [
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
        matches!(self, Self::Hotp)
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
    /// by material the subject chose. Only ever `true` on a password, and
    /// there are none since migration 0056, so it is always `false`
    /// (`credentials_only_a_password_may_require_change`).
    pub must_change: bool,
    /// When a transient credential stops being presentable. `None` for every
    /// row this deployment can write, because nothing writes a transient kind.
    pub expires_at: Option<OffsetDateTime>,
    /// When the row was created. For a federated link this **is** the date of
    /// the first pairing.
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

/// Reads of the `credentials` table. One method since VP-6a: the password and
/// TOTP credentials, and with them `create`, `advance_counter`,
/// `replace_material` and the per-member read, went with the sign-in that
/// used them. The only writer left is the just-in-time provisioning
/// transaction ([`crate::staff::provision_federated_in_tx`]).
#[async_trait]
pub trait Credentials {
    /// The federated link for `(issuer, subject)`, or `None`.
    ///
    /// **The only way a federated identity is ever looked up**, and the key is
    /// the pair — never an address ([ADR-0019](../../../../docs/adr/0019-credential-model.md)
    /// decision 5). `issuer` is whatever the caller passes, and the caller
    /// passes the *configured* one: which issuers a deployment trusts is
    /// configuration, so a row naming any other issuer is simply never
    /// found, and verifies nothing (ADR-0030, R3).
    ///
    /// `(issuer, subject)` is globally unique under
    /// `credentials_federated_identity_key`, so there is at most one row.
    ///
    /// # Errors
    ///
    /// [`DbError::CredentialKindUnknown`] (only reachable if the kind CHECK
    /// were dropped), [`DbError::Persistence`] otherwise.
    async fn find_federated(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<CredentialRow>, DbError>;
}

#[async_trait]
impl Credentials for crate::repository::PgRepositories {
    async fn find_federated(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<CredentialRow>, DbError> {
        let rows = self
            .cs
            .credential()
            .find_many()
            .where_(credential::kind().eq(CredentialKind::Oidc.as_wire_str().to_owned()))
            .where_(credential::issuer().eq(issuer.to_owned()))
            .where_(credential::subject().eq(subject.to_owned()))
            .limit(1)
            .run(&system_context())
            .await
            .map_err(|error| DbError::from(classify_cratestack(MODEL, "read", error)))?;

        rows.into_iter().next().map(row_from_model).transpose()
    }
}

/// The whole `INSERT` for one credential, used by the just-in-time
/// provisioning transaction ([`crate::staff::provision_federated_in_tx`]).
pub(crate) fn create_input(new: NewCredential) -> cratestack_schema::CreateCredentialInput {
    cratestack_schema::CreateCredentialInput {
        id: new.id,
        staff_member_id: new.staff_member_id,
        kind: new.kind.as_wire_str().to_owned(),
        material: new.material,
        issuer: new.issuer,
        subject: new.subject,
        // NOT NULL since migration 0044, seeded to 0; no kind still in use
        // reads it (it was the TOTP replay guard).
        counter: 0,
        must_change: new.must_change,
        expires_at: new.expires_at.map(to_chrono),
        created_at: to_chrono(new.now),
        updated_at: to_chrono(new.now),
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
    /// `read` serves `find_federated`, and `create` serves the just-in-time
    /// provisioning transaction ([`crate::staff::provision_federated_in_tx`]).
    /// `update` and `delete` have no caller since VP-6a (the TOTP replay guard
    /// and the password change went with the sign-in), and are kept for the
    /// reason the model states: customer erasure will need `delete`.
    #[test]
    fn every_action_this_module_calls_has_an_allow_arm() {
        use cratestack_schema::models::CREDENTIAL_MODEL as descriptor;

        assert!(
            !descriptor.read_allow_policies.is_empty(),
            "model Credential lost @@allow(\"read\", …): find_federated would answer `no such \
             link` for every identity, because a read policy is compiled into the WHERE clause \
             and an empty allow list renders FALSE — every vaam pairing would provision a \
             second member"
        );
        assert!(
            !descriptor.create_allow_policies.is_empty(),
            "model Credential lost @@allow(\"create\", …): a first pairing would fail with a \
             Forbidden naming the model"
        );
        assert!(
            !descriptor.update_allow_policies.is_empty(),
            "model Credential lost @@allow(\"update\", …)"
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
        assert_eq!(CredentialKind::parse("Oidc"), None, "case matters");
        // Retired by migration 0056: the database refuses both, so a row of
        // either is one this crate must not decode as anything.
        assert_eq!(CredentialKind::parse("password"), None);
        assert_eq!(CredentialKind::parse("totp"), None);
        assert_eq!(CredentialKind::parse(""), None);
        assert_eq!(CredentialKind::parse("magiclink"), None);
        assert_eq!(CredentialKind::parse("saml"), None);
    }

    /// Six distinct spellings, and the two rules that are *not* one rule.
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
        assert_eq!(spellings.len(), 6, "two kinds share a stored spelling");

        assert!(
            CredentialKind::Hotp.is_singleton_per_subject(),
            "Hotp is one-per-subject"
        );
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
            kind: CredentialKind::Webauthn,
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

        // A sealed counter seed is material too, and the redaction is by column
        // rather than by kind precisely so this needs no second branch.
        let sealed = CredentialRow {
            kind: CredentialKind::Hotp,
            material: Some("bm9uY2VzZWFsZWQ".to_owned()),
            ..secret_bearing.clone()
        };
        assert!(
            !format!("{sealed:?}").contains("bm9uY2VzZWFsZWQ"),
            "a sealed counter secret reached a Debug"
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
            kind: CredentialKind::Webauthn,
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
            "NewCredential's Debug leaked the material: {rendered}"
        );
        assert!(rendered.contains("cred_1"));
    }
}
