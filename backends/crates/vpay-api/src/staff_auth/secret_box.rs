//! AES-256-GCM, the one thing standing between a `staff` table dump and every
//! staff member's second factor (ADR-0017 decision 1).
//!
//! # Why encrypted rather than hashed
//!
//! RFC 6238 verification recomputes an HMAC over the *secret itself*, so
//! there is no one-way form of a TOTP secret that still works. That is the
//! whole difference from [`super::password`], and it is why this module
//! exists at all: the best available property is "an attacker needs the table
//! **and** the key", not "an attacker needs to do work".
//!
//! # The wire format, stated once
//!
//! `base64url(nonce ‖ ciphertext ‖ tag)`, unpadded. Twelve nonce bytes first,
//! because that is what makes the value self-describing — a stored secret
//! carries the nonce it was sealed under, so there is no second column to
//! keep in step and no way for the two to be written apart.
//!
//! Unpadded base64url and not hex, because the column has a length bound and
//! this is a third shorter; and not `BYTEA`, because `cratestack-migrate`
//! does not map `bytea` and the column could then not be named in a generated
//! input at all (migration 0035's header).
//!
//! # The nonce
//!
//! Twelve random bytes per seal, from the OS CSPRNG. GCM's nonce-reuse
//! failure is catastrophic — two messages under one (key, nonce) leak the
//! XOR of their plaintexts and the authentication key — and a counter would
//! need durable state shared by every replica. At 96 bits and a few thousand
//! seals per deployment, a random nonce's collision probability is not a
//! number anyone has to think about; a counter that resets when a pod
//! restarts is.

use aes_gcm::aead::{Aead as _, KeyInit as _};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::password_hash::rand_core::{OsRng, RngCore as _};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::{StaffAuthError, StaffCredentials};

/// AES-256's key length. Not a knob: [`Aes256Gcm`] takes exactly this.
pub const KEY_BYTES: usize = 32;

/// GCM's nonce length, in bytes. Ninety-six bits, which is the only length
/// AES-GCM is specified for without a hashing step.
const NONCE_BYTES: usize = 12;

/// Decodes and length-checks a configured key.
///
/// Separate from [`StaffCredentials::new`] so that the two failures have
/// their own names: "this is not base64url" and "this is base64url of the
/// wrong length" are different mistakes with different fixes, and an operator
/// pasting a Secret needs to be told which one they made.
///
/// # Errors
///
/// [`StaffAuthError::KeyEncoding`] or [`StaffAuthError::KeyLength`].
pub(super) fn decode_key(encoded: &str) -> Result<[u8; KEY_BYTES], StaffAuthError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .map_err(|_| StaffAuthError::KeyEncoding)?;
    <[u8; KEY_BYTES]>::try_from(bytes.as_slice()).map_err(|_| StaffAuthError::KeyLength {
        expected: KEY_BYTES,
    })
}

impl StaffCredentials {
    /// Seals a secret for storage.
    ///
    /// # Errors
    ///
    /// [`StaffAuthError::SecretUnsealable`] if the AEAD refuses, which at
    /// this point can only mean an allocation failure — the key length is
    /// already a type-level fact and the nonce is generated here.
    pub fn seal_secret(&self, plaintext: &[u8]) -> Result<String, StaffAuthError> {
        let mut nonce_bytes = [0_u8; NONCE_BYTES];
        OsRng.fill_bytes(&mut nonce_bytes);

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(self.totp_key()));
        let sealed = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
            .map_err(|_| StaffAuthError::SecretUnsealable)?;

        let mut framed = Vec::with_capacity(NONCE_BYTES + sealed.len());
        framed.extend_from_slice(&nonce_bytes);
        framed.extend_from_slice(&sealed);
        Ok(URL_SAFE_NO_PAD.encode(framed))
    }

    /// Opens a sealed secret.
    ///
    /// One error for every failure — wrong key, truncated value, flipped bit,
    /// not base64url at all — because that is AEAD's contract and because
    /// telling them apart would be an oracle over a value an attacker who can
    /// write the column can choose.
    ///
    /// # Errors
    ///
    /// [`StaffAuthError::SecretUnsealable`].
    pub fn open_secret(&self, sealed: &str) -> Result<Vec<u8>, StaffAuthError> {
        let framed = URL_SAFE_NO_PAD
            .decode(sealed)
            .map_err(|_| StaffAuthError::SecretUnsealable)?;
        let (nonce, ciphertext) = framed
            .split_at_checked(NONCE_BYTES)
            .ok_or(StaffAuthError::SecretUnsealable)?;

        Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(self.totp_key()))
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| StaffAuthError::SecretUnsealable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 32 zero bytes, base64url-encoded — 43 unpadded characters.
    fn key_a() -> String {
        URL_SAFE_NO_PAD.encode([0_u8; KEY_BYTES])
    }

    fn key_b() -> String {
        URL_SAFE_NO_PAD.encode([7_u8; KEY_BYTES])
    }

    #[test]
    fn a_sealed_secret_opens_to_what_went_in() {
        let credentials = StaffCredentials::new("pepper", &key_a()).expect("key");
        let secret = b"12345678901234567890";

        let sealed = credentials.seal_secret(secret).expect("sealing succeeds");
        assert_eq!(
            credentials.open_secret(&sealed).expect("opening succeeds"),
            secret
        );
    }

    /// The plaintext must not be recoverable from the stored value by anyone
    /// who does not hold the key — which is the entire claim this module
    /// makes, so it is asserted rather than assumed.
    #[test]
    fn the_sealed_form_does_not_contain_the_plaintext() {
        let credentials = StaffCredentials::new("pepper", &key_a()).expect("key");
        let secret = b"12345678901234567890";
        let sealed = credentials.seal_secret(secret).expect("sealing succeeds");

        let raw = URL_SAFE_NO_PAD.decode(&sealed).expect("base64url");
        assert!(
            !raw.windows(secret.len()).any(|window| window == secret),
            "the sealed form carries the plaintext"
        );
    }

    /// A different key does not open it. This is the assertion that fails if
    /// the AEAD is ever swapped for an encoding.
    #[test]
    fn another_deployments_key_does_not_open_it() {
        let mine = StaffCredentials::new("pepper", &key_a()).expect("key");
        let theirs = StaffCredentials::new("pepper", &key_b()).expect("key");

        let sealed = mine.seal_secret(b"secret").expect("sealing succeeds");
        assert!(theirs.open_secret(&sealed).is_err());
    }

    /// Two seals of one secret differ, because each carries its own nonce —
    /// and GCM's nonce-reuse failure is the one this property prevents.
    #[test]
    fn two_seals_of_one_secret_differ() {
        let credentials = StaffCredentials::new("pepper", &key_a()).expect("key");
        let first = credentials.seal_secret(b"secret").expect("runs");
        let second = credentials.seal_secret(b"secret").expect("runs");
        assert_ne!(first, second, "the nonce must be fresh per seal");
    }

    /// Every corruption is one answer, and none of them panics.
    #[test]
    fn a_tampered_or_truncated_value_is_refused_uniformly() {
        let credentials = StaffCredentials::new("pepper", &key_a()).expect("key");
        let sealed = credentials.seal_secret(b"secret").expect("runs");

        let mut flipped = URL_SAFE_NO_PAD.decode(&sealed).expect("base64url");
        // The last byte is inside the GCM tag: flipping it is the case the
        // tag exists for.
        if let Some(last) = flipped.last_mut() {
            *last ^= 0x01;
        }

        for bad in [
            String::new(),
            "not base64url!!".to_owned(),
            // Shorter than the nonce, so `split_at_checked` is what refuses.
            URL_SAFE_NO_PAD.encode([0_u8; NONCE_BYTES - 1]),
            // Exactly the nonce and no ciphertext at all.
            URL_SAFE_NO_PAD.encode([0_u8; NONCE_BYTES]),
            URL_SAFE_NO_PAD.encode(&flipped),
        ] {
            assert!(
                credentials.open_secret(&bad).is_err(),
                "must refuse: {bad:?}"
            );
        }
    }

    /// The two key failures are named apart, because the fixes differ.
    #[test]
    fn a_key_of_the_wrong_shape_is_refused_at_construction() {
        assert!(matches!(
            StaffCredentials::new("pepper", "not base64url!!"),
            Err(StaffAuthError::KeyEncoding)
        ));
        assert!(matches!(
            StaffCredentials::new("pepper", &URL_SAFE_NO_PAD.encode([0_u8; 16])),
            Err(StaffAuthError::KeyLength { expected: 32 })
        ));
        assert!(StaffCredentials::new("pepper", &key_a()).is_ok());
    }

    /// Neither secret reaches a `{:?}`, and the shape that does is the one an
    /// operator debugging a truncated Secret needs.
    #[test]
    fn debug_prints_lengths_and_no_bytes() {
        let credentials = StaffCredentials::new("a-very-secret-pepper", &key_a()).expect("key");
        let rendered = format!("{credentials:?}");

        assert!(!rendered.contains("a-very-secret-pepper"), "{rendered}");
        assert!(!rendered.contains(&key_a()), "{rendered}");
        assert!(rendered.contains("20 bytes redacted"), "{rendered}");
        assert!(rendered.contains("32 bytes redacted"), "{rendered}");
    }
}
