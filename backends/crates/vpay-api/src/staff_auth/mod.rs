//! The two credentials a staff member proves themselves with, and the one
//! secret each of them needs from configuration
//! ([ADR-0017](../../../../../docs/adr/0017-staff-authentication.md)
//! decision 1).
//!
//! Four pieces, each its own module so each can be tested on its own:
//! [`password`] (argon2id with a deployment pepper), [`totp`] (RFC 6238, and
//! the one number the replay guard turns), [`secret_box`] (AES-256-GCM, which
//! is how the TOTP secret survives a database dump) and [`tokens`] (the
//! opaque strings — session ids, authorization codes, the one-time password
//! the CLI prints).
//!
//! # Why this lives in `vpay-api` and not in `vpay-db`
//!
//! Both secrets below are *deployment* secrets. `vpay-db` writes and reads
//! opaque strings and never learns what they mean, which is what keeps the
//! pepper and the AEAD key out of the crate that holds a connection pool —
//! and out of `vpay-worker-bin`, which links `vpay-db` and has no business
//! being able to verify a staff password.
//!
//! # What is deliberately not here
//!
//! No lockout counter, no password history, no "security questions". Rate
//! limiting is [`crate::staff::rate_limit`]'s, in-process and fixed-window,
//! for ADR-0017's stated reason: a durable lockout is a denial of service an
//! attacker triggers by guessing at somebody else's address.

use std::fmt;

pub mod password;
pub mod secret_box;
pub mod tokens;
pub mod totp;

/// Every deployment secret the staff credential path needs, in one value.
///
/// One struct rather than two parameters threaded through every call site,
/// so that "this deployment cannot check a staff password" is a single
/// `Option` at boot rather than a condition each handler re-derives. See
/// `vpay_config::StaffAuth`, which is what refuses to boot in livemode when
/// either half is missing.
///
/// **Not `Clone`, and not `Copy`.** It is shared as `Arc<StaffCredentials>`
/// through router state. Handing out copies of key material makes it easy to
/// end up with two values that could disagree about which key sealed a
/// stored secret — and a TOTP secret sealed under a key nobody still holds
/// is a staff member who can never sign in again.
pub struct StaffCredentials {
    /// argon2's secret input (`Argon2::new_with_secret`). Mixed into every
    /// hash and every verification, so a stolen `staff` table is not by
    /// itself an offline cracking target.
    ///
    /// **Losing this invalidates every `password_hash` in the database.** It
    /// belongs in the same Secret as the signing key and has the same backup
    /// story; ADR-0017's Consequences says so in the place an operator will
    /// look.
    pepper: Vec<u8>,
    /// The AES-256-GCM key `staff.totp_secret` is sealed under. Deliberately
    /// **not** the pepper: one is an argon2 secret input and the other an
    /// AEAD key, and reusing one value for both would mean rotating either
    /// forces rotating both.
    totp_key: [u8; secret_box::KEY_BYTES],
}

/// Prints the *shape* of each secret and never a byte of either.
///
/// Length rather than presence, because the length is the one thing an
/// operator debugging a boot failure needs (a pepper truncated by a shell
/// heredoc is the shape of this mistake) and it is not the secret.
impl fmt::Debug for StaffCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaffCredentials")
            .field("pepper", &format_args!("[{} bytes redacted]", self.pepper.len()))
            .field(
                "totp_key",
                &format_args!("[{} bytes redacted]", self.totp_key.len()),
            )
            .finish()
    }
}

impl StaffCredentials {
    /// Assembles the pair from a validated configuration.
    ///
    /// # Errors
    ///
    /// [`StaffAuthError::KeyLength`] if the TOTP key is not exactly
    /// [`secret_box::KEY_BYTES`] bytes after base64url decoding, or
    /// [`StaffAuthError::KeyEncoding`] if it is not base64url at all. Both
    /// are boot failures rather than per-request ones: a deployment whose key
    /// is the wrong length cannot seal or open anything, and finding that out
    /// on the first sign-in attempt would be finding it out from a person who
    /// cannot log in.
    pub fn new(pepper: &str, totp_key_b64: &str) -> Result<Self, StaffAuthError> {
        let key = secret_box::decode_key(totp_key_b64)?;
        Ok(Self {
            pepper: pepper.as_bytes().to_vec(),
            totp_key: key,
        })
    }

    /// argon2's secret input.
    pub(crate) fn pepper(&self) -> &[u8] {
        &self.pepper
    }

    /// The AEAD key.
    pub(crate) fn totp_key(&self) -> &[u8; secret_box::KEY_BYTES] {
        &self.totp_key
    }
}

/// Everything the credential path can fail with.
///
/// A leaf in ADR-0011's sense: it classifies itself once and the HTTP
/// boundary derives status, `type`, `code` and message from that. Every
/// variant here is either a *deployment* fault or an internal one — **a wrong
/// password is not in this enum**, and that is deliberate: a wrong password
/// is an ordinary answer (`Ok(false)`), not an error, and giving it an error
/// variant is how it ends up in a log line beside the address that produced
/// it.
#[derive(Debug, thiserror::Error)]
pub enum StaffAuthError {
    /// The configured TOTP encryption key is not valid base64url.
    ///
    /// The `Display` names neither the value nor its length: an operator has
    /// the value in their own Secret, and this string reaches a log.
    #[error("the configured staff TOTP encryption key is not valid base64url")]
    KeyEncoding,

    /// The configured TOTP encryption key decoded to the wrong number of
    /// bytes. The *expected* length is named — it is a constant in this
    /// source file, not a secret — and the actual one is not.
    #[error(
        "the configured staff TOTP encryption key is not {expected} bytes; AES-256-GCM takes \
         exactly that"
    )]
    KeyLength {
        /// [`secret_box::KEY_BYTES`].
        expected: usize,
    },

    /// A stored TOTP secret could not be opened: wrong key, truncated
    /// ciphertext, or a tampered tag.
    ///
    /// Deliberately undifferentiated. AEAD's whole contract is that these are
    /// one answer, and telling them apart in a message would be an oracle
    /// over a value an attacker can supply if they can write the column.
    #[error("a stored staff TOTP secret could not be opened with this deployment's key")]
    SecretUnsealable,

    /// argon2 refused to hash or to parse a stored hash.
    ///
    /// Carries the library's own text because it names a *parameter* problem
    /// (an impossible memory cost, a malformed PHC string) and never the
    /// password: `argon2::Error` and `password_hash::Error` are both
    /// parameter-level and neither embeds an input.
    #[error("argon2 could not complete a staff password operation: {0}")]
    Password(String),
}

impl vpay_core::Classify for StaffAuthError {
    /// All four are `Internal`, and the uniformity is the point.
    ///
    /// Two are misconfiguration and two are corruption, but none of them is
    /// anything a *caller* did, and none of them may ever reach a person as
    /// "wrong password". `Category::Internal` answers `500` and pages, which
    /// is the correct advice for every one: a deployment whose key is wrong
    /// and a database whose column was tampered with both need an operator,
    /// and neither needs the person at the login form to try again.
    fn category(&self) -> vpay_core::Category {
        vpay_core::Category::Internal
    }
}
