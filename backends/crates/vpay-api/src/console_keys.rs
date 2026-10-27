//! A person's console key: parsing it, proving its holder holds the private
//! half, and storing it — the logic `POST /svc/v1/console_keys` and
//! `vpay-server staff pair-key` share (VP-6a,
//! [ADR-0036](../../../../docs/adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)).
//!
//! ```text
//!   browser ─ generate P-256 key (non-extractable)
//!           ─ sign "vpay.console-pair.v1\n" + publicKey + "\n" + issuedAt
//!   ──▶ vaam admin phone approves ──▶ vaam ──▶ POST /svc/v1/console_keys ──▶ here
//! ```
//!
//! # What a key is
//!
//! A WebCrypto **non-extractable P-256 key** (COSE `ESP256`, `-9`). The
//! registered `public_key` is its **SEC1 uncompressed point**, 65 bytes
//! (`0x04 || X || Y`), as unpadded base64url: what
//! `crypto.subtle.exportKey("raw", publicKey)` returns and what
//! `CoseVerifyKey::p256_sec1` takes. A compressed point is a valid key in a
//! second spelling and is **refused, not normalised**: what is stored must be
//! exactly what the holder signed.
//!
//! # What the proof of possession proves
//!
//! That the caller held the private half when it signed `issued_at`, within
//! [`PROOF_MAX_SKEW_SECS`] of the server's clock. A raw ECDSA-P-256-SHA-256
//! signature (`r || s`, 64 bytes, unpadded base64url — what WebCrypto's `sign`
//! returns) over [`possession_message`]. It does **not** bind the vaam user id:
//! vaam is trusted for *who* the person is, which is exactly why only a service
//! holding `console_pairing` may call. A high `s` is accepted (WebCrypto does
//! not normalise it) and a proof may be replayed inside its window, which is
//! harmless: registration is idempotent on the thumbprint.
//!
//! The order inside [`pair_through_vaam`] is the order an attacker learns
//! least in: the shape of every field, then a stale request before the
//! signature is looked at, then a bad signature before the database is touched,
//! so a refused pairing writes nothing (no member, no link, no key).
//!
//! # What pairing may never do (owner rulings R2 and 2026-10-09)
//!
//! Create or upgrade an administrator. The body has no such field and
//! `deny_unknown_fields` refuses one that is sent; the member it provisions is
//! `is_admin = false`, bound to `dashboard.merchant_id`; a person who is already
//! linked keeps whatever flag they have, and **an already-linked administrator
//! is refused** ([`ConsoleKeyError::Administrator`]): a service holding
//! `console_pairing` can name any linked person, so this route must not be a way
//! to a key that speaks as an administrator. Administrators pair through
//! `vpay-server staff pair-key`, run by an operator.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cratestack_cose::CoseVerifyKey;
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use time::OffsetDateTime;
use vpay_core::{Category, Classify};
use vpay_db::{
    ConsoleKeyRow, CredentialKind, DbError, NewConsoleKey, NewCredential, NewStaff,
    PersistenceError, Repositories, StaffRow, TxOutcome, UnitOfWork as _,
};

/// The text the proof signs begins with this tag, so a signature made for
/// another purpose is never a pairing proof.
pub const PROOF_TAG: &str = "vpay.console-pair.v1";

/// How far `issued_at` may be from the server's clock, either way, in seconds.
pub const PROOF_MAX_SKEW_SECS: i64 = 300;

/// The federated issuer of a person vaam vouches for: the first half of the
/// `(issuer, subject)` pair that links a staff member to a vaam user id.
///
/// A constant of this repository and not configuration, unlike ADR-0030's
/// accepted OIDC issuers: this is not a login the person performs but a link a
/// service holding `console_pairing` asserts, so there is no second issuer to
/// choose between.
pub const VAAM_ISSUER: &str = "vaam";

/// The length of a SEC1 uncompressed P-256 point.
const SEC1_UNCOMPRESSED_LEN: usize = 65;

/// The longest vaam user id accepted, in characters.
const VAAM_USER_ID_MAX_CHARS: usize = 128;
/// The longest display name accepted, in characters (`staff_members.display_name`).
const DISPLAY_NAME_MAX_CHARS: usize = 256;
/// The longest key label accepted, in characters (`console_keys_label_length`).
pub const LABEL_MAX_CHARS: usize = 64;

/// Why a pairing, or a revocation, was refused.
///
/// Every variant is the caller's to fix or the operator's to read; none carries
/// key material or a value the caller sent beyond the field's name.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConsoleKeyError {
    /// A field is missing its shape. `field` is the request's own name for it.
    #[error("{field}: {reason}")]
    Malformed {
        /// The request field, as the caller named it.
        field: &'static str,
        /// What is wrong, in words that quote nothing the caller sent.
        reason: &'static str,
    },

    /// `issued_at` is further from the server's clock than
    /// [`PROOF_MAX_SKEW_SECS`].
    #[error(
        "the pairing request expired: issued_at must be within {PROOF_MAX_SKEW_SECS} seconds of the server's clock"
    )]
    Stale,

    /// The proof does not verify under the public key.
    #[error("possession_proof does not verify under public_key")]
    ProofRejected,

    /// The person is an administrator. Administrators pair through the command
    /// line, never through a service route.
    #[error(
        "this person is an administrator; administrators pair through `vpay-server staff pair-key`"
    )]
    Administrator,

    /// The member is disabled, or belongs to another merchant than the
    /// dashboard's. One answer for both: which of the two is not the caller's
    /// to learn.
    #[error("this person may not be paired a console key")]
    NotPermitted,

    /// The key is already registered to another member, revoked or expired. A
    /// key is never reinstated and never moved.
    #[error("this public key is already registered and cannot be paired again")]
    KeyTaken,

    /// A concurrent first pairing of one person could not be reconciled, or a
    /// stored link points at no member. Both are the deployment's to look at.
    #[error("the pairing could not be completed; the link for this person is inconsistent")]
    Inconsistent,
}

impl Classify for ConsoleKeyError {
    fn category(&self) -> Category {
        match self {
            Self::Malformed { .. } | Self::Stale | Self::ProofRejected => Category::InvalidRequest,
            Self::Administrator | Self::NotPermitted => Category::Forbidden,
            Self::KeyTaken => Category::Conflict,
            Self::Inconsistent => Category::Internal,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Malformed { .. } => "console_key_malformed",
            Self::Stale => "console_key_pairing_expired",
            Self::ProofRejected => "console_key_possession_not_proved",
            Self::Administrator => "console_key_administrator",
            Self::NotPermitted => "console_key_not_permitted",
            Self::KeyTaken => "console_key_taken",
            Self::Inconsistent => "console_key_inconsistent",
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::Inconsistent => self.category().generic_message().to_owned(),
            other => other.to_string(),
        }
    }
}

/// A key that parsed: the text the caller sent, its decoded point and its
/// RFC 9679 thumbprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedConsoleKey {
    /// The base64url text, exactly as sent — what the proof signs and what is
    /// stored.
    pub public_key: String,
    /// The 65 bytes of the SEC1 uncompressed point.
    pub point: [u8; SEC1_UNCOMPRESSED_LEN],
    /// The thumbprint: SHA-256 of the COSE key, lower-case hex (64 characters),
    /// computed here and never taken from a caller.
    pub thumbprint: String,
}

/// Decodes `public_key` (SEC1 uncompressed P-256, unpadded base64url) and
/// derives its thumbprint.
///
/// The point must be on the curve and in the uncompressed encoding. A
/// compressed encoding is refused, not normalised.
///
/// ```
/// use vpay_api::console_keys::parse_public_key;
///
/// // The generator of P-256, SEC1 uncompressed, unpadded base64url.
/// let g = "BGsX0fLhLEJH-Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT-NC4v4af5uO5-tKfA-eFivOM1drMV7Oy7ZAaDe_UfU";
/// let parsed = parse_public_key(g).unwrap();
/// assert_eq!(parsed.point[0], 0x04);
/// assert_eq!(parsed.thumbprint.len(), 64);
/// // Padding, the wrong length and a point off the curve are refused.
/// assert!(parse_public_key(&format!("{g}=")).is_err());
/// assert!(parse_public_key("AAAA").is_err());
/// ```
///
/// # Errors
///
/// [`ConsoleKeyError::Malformed`] for `public_key`: not unpadded base64url, not
/// 65 bytes, not starting `0x04`, or not a point on P-256.
pub fn parse_public_key(public_key: &str) -> Result<ParsedConsoleKey, ConsoleKeyError> {
    let malformed = |reason| ConsoleKeyError::Malformed {
        field: "public_key",
        reason,
    };
    let decoded = URL_SAFE_NO_PAD.decode(public_key).map_err(|_| {
        malformed("must be the SEC1 uncompressed P-256 point as unpadded base64url")
    })?;
    let point: [u8; SEC1_UNCOMPRESSED_LEN] = decoded
        .try_into()
        .map_err(|_| malformed("must be 65 bytes (a SEC1 uncompressed P-256 point)"))?;
    if point[0] != 0x04 {
        return Err(malformed(
            "must be the uncompressed SEC1 form (first byte 0x04); a compressed point is refused",
        ));
    }
    let key = CoseVerifyKey::p256_sec1(&point)
        .map_err(|_| malformed("is not a point on the P-256 curve"))?;
    Ok(ParsedConsoleKey {
        public_key: public_key.to_owned(),
        point,
        thumbprint: hex::encode(key.thumbprint()),
    })
}

/// The exact bytes the console key signs to prove it holds the private half:
/// `"vpay.console-pair.v1\n" + public_key + "\n" + issued_at` (unix seconds,
/// decimal). `public_key` is the base64url text the caller sent, not the decoded
/// point.
///
/// ```
/// use vpay_api::console_keys::possession_message;
///
/// assert_eq!(
///     possession_message("a2V5", 1_700_000_000),
///     b"vpay.console-pair.v1\na2V5\n1700000000".to_vec()
/// );
/// ```
#[must_use]
pub fn possession_message(public_key: &str, issued_at: i64) -> Vec<u8> {
    format!("{PROOF_TAG}\n{public_key}\n{issued_at}").into_bytes()
}

/// Checks `proof`, a raw ECDSA-P-256-SHA-256 signature over [`possession_message`],
/// against the registered point.
fn verify_possession(
    key: &ParsedConsoleKey,
    issued_at: i64,
    proof: &str,
) -> Result<(), ConsoleKeyError> {
    let signature = URL_SAFE_NO_PAD
        .decode(proof)
        .map_err(|_| ConsoleKeyError::Malformed {
            field: "possession_proof",
            reason: "must be unpadded base64url",
        })?;
    let signature = Signature::from_slice(&signature).map_err(|_| ConsoleKeyError::Malformed {
        field: "possession_proof",
        reason: "must be a 64-byte r||s ECDSA signature",
    })?;
    let verifying_key =
        VerifyingKey::from_sec1_bytes(&key.point).map_err(|_| ConsoleKeyError::ProofRejected)?;
    verifying_key
        .verify(&possession_message(&key.public_key, issued_at), &signature)
        .map_err(|_| ConsoleKeyError::ProofRejected)
}

/// Whether `issued_at` is further than [`PROOF_MAX_SKEW_SECS`] from `now`.
///
/// `abs_diff`, not `(now - issued_at).abs()`: `issued_at` is the caller's
/// number, and a subtraction of two `i64`s overflows for a chosen one (it
/// panics in a debug build; in release `now - 2^63` wraps to `i64::MIN`, whose
/// `abs()` is itself, negative, and so never exceeds the window). The unsigned
/// distance between two `i64`s always fits a `u64`.
fn is_stale(now: i64, issued_at: i64) -> bool {
    now.abs_diff(issued_at) > PROOF_MAX_SKEW_SECS.unsigned_abs()
}

/// Everything about a pairing request that can be decided without the
/// database: the key is a P-256 point, `issued_at` is within
/// [`PROOF_MAX_SKEW_SECS`] of `now`, and the signature over
/// [`possession_message`] verifies under that key.
///
/// # Errors
///
/// [`ConsoleKeyError::Malformed`], [`ConsoleKeyError::Stale`] or
/// [`ConsoleKeyError::ProofRejected`].
pub fn check_proof(
    public_key: &str,
    issued_at: i64,
    proof: &str,
    now: OffsetDateTime,
) -> Result<ParsedConsoleKey, ConsoleKeyError> {
    let key = parse_public_key(public_key)?;
    if is_stale(now.unix_timestamp(), issued_at) {
        return Err(ConsoleKeyError::Stale);
    }
    verify_possession(&key, issued_at, proof)?;
    Ok(key)
}

/// A key label, trimmed: 1 to [`LABEL_MAX_CHARS`] characters.
///
/// # Errors
///
/// [`ConsoleKeyError::Malformed`] for `label`.
pub fn clean_label(label: &str) -> Result<String, ConsoleKeyError> {
    let trimmed = label.trim();
    if trimmed.is_empty() || trimmed.chars().count() > LABEL_MAX_CHARS {
        return Err(ConsoleKeyError::Malformed {
            field: "label",
            reason: "must be 1 to 64 characters",
        });
    }
    Ok(trimmed.to_owned())
}

/// What a pairing request carries, after the route has decoded it.
#[derive(Debug, Clone)]
pub struct PairRequest {
    /// The person's id at vaam: `[A-Za-z0-9_-]`, 1 to 128 characters.
    pub vaam_user_id: String,
    /// What the console greets them by; derived from the id when absent.
    pub display_name: Option<String>,
    /// The key, SEC1 uncompressed, unpadded base64url.
    pub public_key: String,
    /// The proof of possession, unpadded base64url.
    pub possession_proof: String,
    /// When the console signed the proof, unix seconds.
    pub issued_at: i64,
    /// A name for the key in logs.
    pub label: Option<String>,
}

/// A key as stored, and whether this call stored it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paired {
    /// The row.
    pub key: ConsoleKeyRow,
    /// `true` when this call created the row; `false` when the same live key
    /// of the same member was already there (idempotent repeat).
    pub created: bool,
}

/// `vaam_user_id`, if it is safe to become the local part of an address.
fn clean_vaam_user_id(id: &str) -> Result<&str, ConsoleKeyError> {
    if id.is_empty()
        || id.chars().count() > VAAM_USER_ID_MAX_CHARS
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ConsoleKeyError::Malformed {
            field: "vaam_user_id",
            reason: "must be 1 to 128 characters of A-Z a-z 0-9 _ -",
        });
    }
    Ok(id)
}

fn display_name_for(
    display_name: Option<&str>,
    vaam_user_id: &str,
) -> Result<String, ConsoleKeyError> {
    let Some(name) = display_name else {
        return Ok(format!("vaam {vaam_user_id}"));
    };
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > DISPLAY_NAME_MAX_CHARS {
        return Err(ConsoleKeyError::Malformed {
            field: "display_name",
            reason: "must be 1 to 256 characters",
        });
    }
    Ok(trimmed.to_owned())
}

/// Pairs `request` for the person it names, on behalf of a service that holds
/// `console_pairing`, into the dashboard's tenant `tenant`.
///
/// 1. every field is checked, then the proof (no database yet);
/// 2. the person is found by their link `(vaam, vaam_user_id)`, or provisioned
///    just in time as a **plain member** of `tenant` with the link, in one
///    transaction;
/// 3. a member who is an administrator, disabled, or of another merchant is
///    refused — nothing about them is changed;
/// 4. the key is stored, or found to be this member's live key already.
///
/// # Errors
///
/// [`ApiError`](crate::ApiError) wrapping a [`ConsoleKeyError`] for every
/// refusal above, and the database's own error for a failed read or write.
pub async fn pair_through_vaam(
    repositories: &Arc<dyn Repositories>,
    tenant: &str,
    request: PairRequest,
    now: OffsetDateTime,
) -> Result<Paired, crate::ApiError> {
    let vaam_user_id = clean_vaam_user_id(&request.vaam_user_id)?;
    let display_name = display_name_for(request.display_name.as_deref(), vaam_user_id)?;
    let label = request.label.as_deref().map(clean_label).transpose()?;
    let key = check_proof(
        &request.public_key,
        request.issued_at,
        &request.possession_proof,
        now,
    )?;

    let member = link_member(repositories, tenant, vaam_user_id, &display_name, now).await?;
    require_pairable(&member, tenant)?;
    store_key(repositories.as_ref(), &member, &key, label, now).await
}

/// Refuses a member a service route may not attach a key to.
///
/// The administrator test is on the row as it is **now**, read after any
/// provisioning, so a person promoted since their first pairing is refused on
/// the next.
fn require_pairable(member: &StaffRow, tenant: &str) -> Result<(), ConsoleKeyError> {
    // Both refusals are one `403` on the wire (the credential's answer, not the
    // object's); the log is where an operator reads which.
    if member.is_admin {
        tracing::warn!(
            staff_id = %member.id,
            "a service asked to pair a console key for an administrator; refusing: \
             administrators pair through `vpay-server staff pair-key`"
        );
        return Err(ConsoleKeyError::Administrator);
    }
    if !member.is_active() || member.merchant_id != tenant {
        tracing::warn!(
            staff_id = %member.id,
            "a service asked to pair a console key for a disabled member or one of another \
             merchant; refusing"
        );
        return Err(ConsoleKeyError::NotPermitted);
    }
    Ok(())
}

/// The member a vaam user id names: found through the link, or provisioned.
async fn link_member(
    repositories: &Arc<dyn Repositories>,
    tenant: &str,
    vaam_user_id: &str,
    display_name: &str,
    now: OffsetDateTime,
) -> Result<StaffRow, crate::ApiError> {
    if let Some(member) = linked(repositories.as_ref(), vaam_user_id).await? {
        return Ok(member);
    }

    let staff_id = vpay_core::ids::staff_id();
    let new_staff = NewStaff {
        id: staff_id.clone(),
        // From CONFIGURATION, never from the request (ADR-0019 § R2).
        merchant_id: tenant.to_owned(),
        // `.invalid` is reserved by RFC 2606 and never resolves; the column is
        // required, unique and lower-case, and the person has no address here.
        email: format!("{}@vaam.invalid", vaam_user_id.to_lowercase()),
        display_name: display_name.to_owned(),
        // Never from the request, and never true: pairing creates no
        // administrator (owner ruling R2).
        is_admin: false,
        now,
    };
    let link = NewCredential {
        id: vpay_core::ids::credential_id(),
        staff_member_id: Some(staff_id),
        kind: CredentialKind::Oidc,
        material: None,
        issuer: Some(VAAM_ISSUER.to_owned()),
        subject: Some(vaam_user_id.to_owned()),
        must_change: false,
        expires_at: None,
        now,
    };
    let outcome = repositories
        .transaction(|tx| {
            Box::pin(async move {
                tx.provision_federated_staff_in_tx(&new_staff, link).await?;
                Ok::<_, DbError>(TxOutcome::Commit(()))
            })
        })
        .await;
    match outcome {
        Ok(_) => {}
        // The person was provisioned a moment ago by a concurrent request (the
        // link's unique index), or the synthetic address collides (a case-only
        // difference of two vaam ids). Ask again: a link now means the former.
        Err(DbError::Persistence(PersistenceError::Unique { .. })) => {}
        Err(error) => return Err(error.into()),
    }
    // Re-read through the link rather than trusting either branch: whichever
    // request won, this is the member the identity names.
    linked(repositories.as_ref(), vaam_user_id)
        .await?
        .ok_or_else(|| ConsoleKeyError::KeyTaken.into())
}

/// The member linked to `(vaam, vaam_user_id)`, if any.
async fn linked(
    repositories: &dyn Repositories,
    vaam_user_id: &str,
) -> Result<Option<StaffRow>, crate::ApiError> {
    let Some(link) = repositories
        .find_federated(VAAM_ISSUER, vaam_user_id)
        .await?
    else {
        return Ok(None);
    };
    let staff_id = link.staff_member_id.ok_or(ConsoleKeyError::Inconsistent)?;
    Ok(Some(
        repositories
            .find(&staff_id)
            .await?
            .ok_or(ConsoleKeyError::Inconsistent)?,
    ))
}

/// Stores `key` for `member`, or finds that this member's live key is already
/// stored (an idempotent repeat). The one write both entry points make, so the
/// 30-day life, the thumbprint's uniqueness and "a revoked or expired key is
/// never reinstated" are decided in one place.
///
/// # Errors
///
/// [`ConsoleKeyError::KeyTaken`] when the thumbprint is another member's,
/// revoked or expired; the database's own error otherwise.
pub async fn store_key(
    repositories: &dyn Repositories,
    member: &StaffRow,
    key: &ParsedConsoleKey,
    label: Option<String>,
    now: OffsetDateTime,
) -> Result<Paired, crate::ApiError> {
    let new = NewConsoleKey {
        id: vpay_core::ids::console_key_id(),
        staff_member_id: member.id.clone(),
        public_key: key.public_key.clone(),
        thumbprint: key.thumbprint.clone(),
        label,
        now,
    };
    match repositories.pair_console_key(new).await {
        Ok(()) => {
            let row = repositories
                .find_console_key(&key.thumbprint)
                .await?
                .ok_or(ConsoleKeyError::Inconsistent)?;
            Ok(Paired {
                key: row,
                created: true,
            })
        }
        Err(DbError::Persistence(PersistenceError::Unique { .. })) => {
            let existing = repositories
                .find_console_key(&key.thumbprint)
                .await?
                .ok_or(ConsoleKeyError::Inconsistent)?;
            if existing.staff_member_id == member.id && existing.is_live(now) {
                Ok(Paired {
                    key: existing,
                    created: false,
                })
            } else {
                Err(ConsoleKeyError::KeyTaken.into())
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Revokes the key with this thumbprint, and answers the row as it stands.
///
/// Idempotent: a key already revoked is answered with its first `revoked_at`.
///
/// # Errors
///
/// [`ConsoleKeyError::Malformed`] for a thumbprint that is not 64 lower-case hex
/// characters, `404` for one nothing registered.
pub async fn revoke(
    repositories: &dyn Repositories,
    thumbprint: &str,
    now: OffsetDateTime,
) -> Result<ConsoleKeyRow, crate::ApiError> {
    if thumbprint.len() != 64
        || !thumbprint
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ConsoleKeyError::Malformed {
            field: "thumbprint",
            reason: "must be 64 lower-case hex characters",
        }
        .into());
    }
    repositories.revoke_console_key(thumbprint, now).await?;
    repositories
        .find_console_key(thumbprint)
        .await?
        .ok_or_else(|| crate::ApiError::NotFound {
            resource: "console_key",
            id: thumbprint.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer as _;

    use super::*;

    /// A throwaway P-256 key derived from a scalar built at run time, never a
    /// stored value.
    fn signing_key(seed: u8) -> SigningKey {
        let mut scalar = [seed; 32];
        // Keep the scalar well inside the group order.
        scalar[0] = 1;
        SigningKey::from_slice(&scalar).expect("a valid scalar")
    }

    fn public_text(key: &SigningKey) -> String {
        URL_SAFE_NO_PAD.encode(key.verifying_key().to_encoded_point(false).as_bytes())
    }

    fn proof(key: &SigningKey, public: &str, issued_at: i64) -> String {
        let signature: Signature = key.sign(&possession_message(public, issued_at));
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    }

    #[test]
    fn an_uncompressed_point_parses_and_a_compressed_one_is_refused_not_normalised() {
        let key = signing_key(3);
        let good = public_text(&key);
        let parsed = parse_public_key(&good).expect("uncompressed parses");
        assert_eq!(parsed.public_key, good, "the text is kept exactly as sent");
        assert_eq!(parsed.thumbprint.len(), 64);

        let compressed =
            URL_SAFE_NO_PAD.encode(key.verifying_key().to_encoded_point(true).as_bytes());
        let error = parse_public_key(&compressed).expect_err("a compressed point is refused");
        assert!(
            matches!(
                error,
                ConsoleKeyError::Malformed {
                    field: "public_key",
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn a_point_off_the_curve_and_padding_are_refused() {
        let mut bytes = signing_key(4)
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        if let Some(last) = bytes.last_mut() {
            *last ^= 0x01;
        }
        assert!(parse_public_key(&URL_SAFE_NO_PAD.encode(&bytes)).is_err());
        let padded = format!("{}=", public_text(&signing_key(4)));
        assert!(parse_public_key(&padded).is_err());
    }

    #[test]
    fn a_proof_inside_its_window_and_under_the_key_verifies() {
        let key = signing_key(5);
        let public = public_text(&key);
        let now = OffsetDateTime::now_utc();
        let issued = now.unix_timestamp();
        assert!(check_proof(&public, issued, &proof(&key, &public, issued), now).is_ok());
        // The window is symmetric and inclusive at 300 s.
        for offset in [-PROOF_MAX_SKEW_SECS, PROOF_MAX_SKEW_SECS] {
            let at = issued + offset;
            assert!(
                check_proof(&public, at, &proof(&key, &public, at), now).is_ok(),
                "{offset}"
            );
        }
    }

    /// `issued_at` is the caller's number. Whatever it is, it is stale or fresh
    /// by an unsigned distance, never by a subtraction that can overflow:
    /// `now - 2^63` is the value for which `(now - issued_at).abs()` wrapped to
    /// `i64::MIN` and the window never closed.
    #[test]
    fn an_issued_at_chosen_to_overflow_the_subtraction_is_stale_not_fresh() {
        let now = OffsetDateTime::now_utc();
        let at = now.unix_timestamp();
        let key = signing_key(8);
        let public = public_text(&key);

        for issued_at in [
            i64::MIN,
            i64::MAX,
            // `now - 2^63`: representable, and the one the old code wrapped on.
            at + i64::MIN,
            at - i64::MAX,
            i64::MAX - 1,
        ] {
            // The proof is not even valid: staleness is decided first.
            assert_eq!(
                check_proof(&public, issued_at, "AAAA", now),
                Err(ConsoleKeyError::Stale),
                "{issued_at}"
            );
        }
        assert!(is_stale(at, at - PROOF_MAX_SKEW_SECS - 1));
        assert!(is_stale(at, at + PROOF_MAX_SKEW_SECS + 1));
        assert!(!is_stale(at, at - PROOF_MAX_SKEW_SECS));
        assert!(!is_stale(at, at + PROOF_MAX_SKEW_SECS));
        assert!(!is_stale(at, at));
        // The extremes of the clock itself, against the extremes of the claim.
        assert!(is_stale(i64::MIN, i64::MAX));
        assert!(is_stale(i64::MAX, i64::MIN));
        assert!(!is_stale(i64::MAX, i64::MAX));
    }

    /// Each guard, one mutation each in the proof of this change: widen the
    /// window and `301` passes; accept any signature and the wrong-key case
    /// passes; sign a different text and the message case passes.
    #[test]
    fn a_stale_future_or_wrongly_signed_proof_is_refused() {
        let key = signing_key(6);
        let other = signing_key(7);
        let public = public_text(&key);
        let now = OffsetDateTime::now_utc();
        let issued = now.unix_timestamp();

        for offset in [-(PROOF_MAX_SKEW_SECS + 1), PROOF_MAX_SKEW_SECS + 1] {
            let at = issued + offset;
            assert_eq!(
                check_proof(&public, at, &proof(&key, &public, at), now),
                Err(ConsoleKeyError::Stale),
                "{offset}"
            );
        }
        // Signed by a key that is not the one being registered.
        assert_eq!(
            check_proof(&public, issued, &proof(&other, &public, issued), now),
            Err(ConsoleKeyError::ProofRejected)
        );
        // Signed over a different instant than the one claimed.
        assert_eq!(
            check_proof(&public, issued, &proof(&key, &public, issued - 1), now),
            Err(ConsoleKeyError::ProofRejected)
        );
        // Signed over a different tag: a signature made for another purpose.
        let wrong: Signature =
            key.sign(format!("vpay.something-else\n{public}\n{issued}").as_bytes());
        assert_eq!(
            check_proof(
                &public,
                issued,
                &URL_SAFE_NO_PAD.encode(wrong.to_bytes()),
                now
            ),
            Err(ConsoleKeyError::ProofRejected)
        );
        // Not a signature at all.
        assert!(matches!(
            check_proof(&public, issued, "AAAA", now),
            Err(ConsoleKeyError::Malformed {
                field: "possession_proof",
                ..
            })
        ));
    }

    #[test]
    fn user_ids_labels_and_names_are_bounded() {
        assert!(clean_vaam_user_id("usr_01-AZ").is_ok());
        for bad in ["", "a b", "a@b", "é", &"x".repeat(129)] {
            assert!(clean_vaam_user_id(bad).is_err(), "{bad:?}");
        }
        assert_eq!(clean_label("  ci  ").as_deref(), Ok("ci"));
        assert!(clean_label("  ").is_err());
        assert!(clean_label(&"x".repeat(65)).is_err());
        assert_eq!(display_name_for(None, "usr_1").as_deref(), Ok("vaam usr_1"));
        assert!(display_name_for(Some(" "), "usr_1").is_err());
    }

    #[test]
    fn administrators_and_foreign_or_disabled_members_are_not_pairable() {
        let member = |is_admin: bool, merchant: &str, status: vpay_db::StaffStatus| StaffRow {
            id: "stf_1".to_owned(),
            merchant_id: merchant.to_owned(),
            email: "a@b.invalid".to_owned(),
            display_name: "A".to_owned(),
            status,
            is_admin,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            last_sign_in_at: None,
        };
        let active = vpay_db::StaffStatus::Active;
        assert_eq!(
            require_pairable(&member(false, "acme", active), "acme"),
            Ok(())
        );
        assert_eq!(
            require_pairable(&member(true, "acme", active), "acme"),
            Err(ConsoleKeyError::Administrator)
        );
        assert_eq!(
            require_pairable(&member(false, "other", active), "acme"),
            Err(ConsoleKeyError::NotPermitted)
        );
        assert_eq!(
            require_pairable(
                &member(false, "acme", vpay_db::StaffStatus::Disabled),
                "acme"
            ),
            Err(ConsoleKeyError::NotPermitted)
        );
    }
}
