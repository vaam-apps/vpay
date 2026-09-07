//! The opaque strings: session ids, authorization codes, TOTP secrets, and
//! the one-time password `vpay-server staff add` prints (ADR-0017).
//!
//! # One rule, four uses
//!
//! > **Mint from the OS CSPRNG, hand out the token, store only its SHA-256.**
//!
//! `staff_sessions.id` and `oauth_authorization_codes.code_hash` are digests,
//! never the credential, so a dump of either table yields nothing that can be
//! presented. It is the same property `idempotency_keys` established for a
//! request hash and the same one `vpay_db::client_assertion` relies on.
//!
//! SHA-256 and not argon2, and the difference from [`super::password`] is
//! worth being exact about: these values are 256 bits of CSPRNG output, so
//! there is no dictionary to slow an attacker down against and a work factor
//! would buy nothing but latency on every request. A password is low-entropy
//! and chosen by a person; these are not.
//!
//! # The lookup is by digest, so the comparison is the index
//!
//! Nothing here compares two digests in Rust. A caller hashes the presented
//! token and looks the row up by primary key, so the "comparison" is
//! Postgres's index lookup — which is not a timing oracle over a secret,
//! because the value being indexed is already a hash of it.

use argon2::password_hash::rand_core::{OsRng, RngCore as _};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};

/// How many random bytes a session token or an authorization code carries.
///
/// Thirty-two — 256 bits. The value is guessed-at over the internet by
/// anybody who can reach `/dash/v1`, so it has to be far beyond any online
/// guessing budget, and 256 bits is also exactly what the SHA-256 it is
/// stored under can distinguish.
const TOKEN_BYTES: usize = 32;

/// How many characters the one-time password the CLI prints has.
///
/// Twenty-six, from a 32-character alphabet: 130 bits. It is typed by a
/// person exactly once and then replaced, so it is long rather than
/// memorable, and it is generated rather than chosen because an operator
/// inventing passwords for other people is how "Welcome123" happens.
const ONE_TIME_PASSWORD_CHARS: usize = 26;

/// The one-time password's alphabet: Crockford base32, which is
/// `vpay_core::ids`' alphabet for the same reason — no `i`, `l`, `o` or `u`,
/// so nothing in it can be misread off a terminal or misheard over a phone.
const PASSWORD_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// A freshly minted opaque credential: the value to hand out, and the digest
/// to store.
///
/// One struct with both halves, so a caller cannot store the token by
/// accident — the only way to get the digest is to have been given the pair,
/// and the field names say which is which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedToken {
    /// What the caller is handed. Returned **once** and never stored.
    pub token: String,
    /// What goes in the database: `sha256(token)`, lower-case hex, 64
    /// characters — which is exactly what
    /// `staff_sessions_id_length` and
    /// `oauth_authorization_codes_code_hash_length` bound.
    pub digest: String,
}

/// Mints one opaque credential.
#[must_use]
pub fn mint() -> MintedToken {
    let mut bytes = [0_u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let digest = digest(&token);
    MintedToken { token, digest }
}

/// `sha256(token)`, lower-case hex.
///
/// The one function that turns a presented credential into the key it is
/// looked up by. A free function rather than a method on [`MintedToken`],
/// because the presenting side has only a `&str` and must reach the same
/// answer through the same code.
#[must_use]
pub fn digest(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex(&hasher.finalize())
}

/// A fresh RFC 6238 secret, [`super::totp::SECRET_BYTES`] long.
#[must_use]
pub fn totp_secret() -> Vec<u8> {
    let mut secret = vec![0_u8; super::totp::SECRET_BYTES];
    OsRng.fill_bytes(&mut secret);
    secret
}

/// A fresh one-time password for `vpay-server staff add`.
///
/// Rejection-free: the alphabet is exactly 32 characters, so five bits of
/// CSPRNG output map onto it with no modulo bias at all. An alphabet whose
/// length is not a power of two needs rejection sampling, and getting that
/// subtly wrong is how a password generator quietly loses entropy.
#[must_use]
pub fn one_time_password() -> String {
    let mut bytes = vec![0_u8; ONE_TIME_PASSWORD_CHARS];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|byte| {
            let index = usize::from(byte & 0x1f);
            char::from(PASSWORD_ALPHABET.get(index).copied().unwrap_or(b'0'))
        })
        .collect()
}

/// Lower-case hex, fixed width. Two characters per byte, always.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut out, byte| {
        // Infallible: `String`'s `Write` never errors. Written this way
        // rather than with `expect`, which this crate denies outside tests.
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The digest is what the two `char_length(...) = 64` CHECKs bound, so a
    /// change to the hash would be a change to the schema.
    #[test]
    fn a_digest_is_sixty_four_lower_case_hex_characters() {
        let minted = mint();
        assert_eq!(minted.digest.len(), 64, "{}", minted.digest);
        assert!(
            minted
                .digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "{}",
            minted.digest
        );
    }

    /// The digest is `sha256(token)` and the presenting side reaches it
    /// through the same function — pinned against a published vector so that
    /// "the same function" is not the only thing holding the two together.
    #[test]
    fn the_digest_is_sha256_of_the_token() {
        let minted = mint();
        assert_eq!(digest(&minted.token), minted.digest);

        assert_eq!(
            digest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "NIST's own SHA-256 vector for \"abc\""
        );
    }

    /// Two mints differ. A generator that repeated would hand two staff
    /// members one session.
    #[test]
    fn two_mints_are_different() {
        let first = mint();
        let second = mint();
        assert_ne!(first.token, second.token);
        assert_ne!(first.digest, second.digest);
    }

    /// The token itself is never derivable from the digest by anything here,
    /// and — the property that actually matters — the digest does not contain
    /// it.
    #[test]
    fn the_digest_does_not_contain_the_token() {
        let minted = mint();
        assert!(!minted.digest.contains(&minted.token));
    }

    /// The one-time password is typed by a person off a terminal, so every
    /// character has to be one that survives that.
    #[test]
    fn a_one_time_password_is_unambiguous_and_long_enough() {
        for _ in 0..64 {
            let password = one_time_password();
            assert_eq!(password.len(), ONE_TIME_PASSWORD_CHARS);
            assert!(
                password.bytes().all(|b| PASSWORD_ALPHABET.contains(&b)),
                "{password}"
            );
            for confusable in ['i', 'l', 'o', 'u'] {
                assert!(
                    !password.contains(confusable),
                    "{password} contains {confusable}, which is misread off a terminal"
                );
            }
        }
    }

    /// A TOTP secret is the length RFC 4226 §4 R6 recommends, and two of them
    /// differ.
    #[test]
    fn a_totp_secret_is_twenty_random_bytes() {
        let first = totp_secret();
        assert_eq!(first.len(), super::super::totp::SECRET_BYTES);
        assert_ne!(first, totp_secret());
        assert!(
            first.iter().any(|byte| *byte != 0),
            "twenty zero bytes is not a secret"
        );
    }
}
