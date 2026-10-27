//! The key this process seals `/v1` responses with, and the one it will seal
//! with after the next rotation (VP-3).
//!
//! Both are Ed25519 **seeds**: 32 random bytes, written as unpadded base64url
//! text on one line (a trailing newline is fine), in a file a Kubernetes Secret
//! mounts. Never an environment variable holding the key, and never a value in
//! the YAML: the same rule as the OAuth key's, for the same reasons
//! (`vpay_config::cli::ServerArgs::oauth_signing_key_file`).
//!
//! The format is the one `merchant_clients[].cose_keys` already uses for
//! *public* keys (unpadded base64url, `vpay_config::CoseKey`), so an operator
//! reads one encoding in both directions, and `cargo xtask gen-envelope-key`
//! writes a seed and prints the public half in it.
//!
//! # Why `next` is loaded at all
//!
//! The process signs with `current` only. `next` is read so that a rotation
//! prepared wrongly fails the boot that would have shipped it rather than the
//! merchant who pins it: a `next` that is not a seed, or that is the same seed
//! as `current` (rotating to a copy of the old key rotates nothing, and a
//! merchant who pinned `{current, next}` would then have pinned one key twice).
//! It also gives the operator the pair of public keys to hand to merchants, in
//! the boot log, without a second tool.
//!
//! # Not zeroized
//!
//! The seed text is read into a `String` and the decoded bytes into an array;
//! both are dropped normally and stay in freed memory until overwritten, as the
//! OAuth key's PEM does (`crate::op::keys`' closing paragraph). The signer
//! itself wipes its expanded key on drop (cratestack's `Ed25519Signer`). Closing
//! the rest is a `zeroize` dependency and a secret-string type, which is a real
//! change worth making deliberately.

use std::fmt;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cratestack_cose::Ed25519Signer;
use vpay_core::{Category, Classify};

/// Which of the two key files a failure is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// `--envelope-signing-key-file`: signs now.
    Current,
    /// `--envelope-signing-key-next-file`: signs after the next rotation.
    Next,
}

impl Slot {
    /// The flag, both spellings, because the message is the whole fix.
    fn flag(self) -> &'static str {
        match self {
            Self::Current => "--envelope-signing-key-file / VPAY_ENVELOPE_SIGNING_KEY_FILE",
            Self::Next => "--envelope-signing-key-next-file / VPAY_ENVELOPE_SIGNING_KEY_NEXT_FILE",
        }
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Current => "current",
            Self::Next => "next",
        })
    }
}

/// Why the envelope keys could not be loaded.
///
/// Every variant is [`Category::Configuration`]: a missing, unreadable or
/// malformed Secret is a deploy to fix, never something to retry and never
/// something a caller did. **No variant carries key material** — the file's
/// path is named (it is not secret and is the first thing an operator needs),
/// its contents never are.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeKeyError {
    /// The file could not be read: not mounted, wrong path, wrong mode.
    #[error("cannot read the {slot} envelope signing key file at {path} ({flag})", flag = .slot.flag())]
    Read {
        /// Which key.
        slot: Slot,
        /// The path that was tried.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The file was read but does not hold a seed: not unpadded base64url, or
    /// not 32 bytes once decoded. The reason says which, never what was read.
    #[error(
        "the {slot} envelope signing key is not an Ed25519 seed: {reason}. Expected 32 random \
         bytes as unpadded base64url on one line ({flag}); `cargo xtask gen-envelope-key` writes one",
        flag = .slot.flag()
    )]
    NotASeed {
        /// Which key.
        slot: Slot,
        /// What is wrong with it, in words that quote nothing from the file.
        reason: &'static str,
    },

    /// `current` and `next` are one key. A rotation to a copy of the key it
    /// replaces rotates nothing.
    #[error(
        "the current and next envelope signing keys are the same seed; the next key must be a \
         different key or a rotation changes nothing ({} and {})",
        Slot::Current.flag(),
        Slot::Next.flag()
    )]
    SameKey,
}

impl Classify for EnvelopeKeyError {
    /// A deploy that must be fixed, so exit `78` at boot.
    fn category(&self) -> Category {
        Category::Configuration
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Read { .. } => "envelope_key_unreadable",
            Self::NotASeed { .. } => "envelope_key_not_a_seed",
            Self::SameKey => "envelope_keys_identical",
        }
    }
}

/// The two Ed25519 keys of the response envelope.
///
/// Holds the signer for `current` and only the **public** half of `next`: the
/// process never signs with the latter, so after loading it nothing in memory
/// can.
pub struct EnvelopeKeys {
    current: Ed25519Signer,
    current_public: [u8; 32],
    next_public: [u8; 32],
}

impl fmt::Debug for EnvelopeKeys {
    /// Public keys only. A derived `Debug` would print the signer, which is
    /// safe (it prints a `kid`), but "this type is safe to log" should not
    /// depend on a dependency's `Debug`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvelopeKeys")
            .field("current", &URL_SAFE_NO_PAD.encode(self.current_public))
            .field("next", &URL_SAFE_NO_PAD.encode(self.next_public))
            .finish()
    }
}

impl EnvelopeKeys {
    /// Parses two seeds given as text.
    ///
    /// ```
    /// use base64::Engine as _;
    /// use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    /// use vpay_api::envelope::EnvelopeKeys;
    ///
    /// let seed = |byte: u8| URL_SAFE_NO_PAD.encode([byte; 32]);
    /// let keys = EnvelopeKeys::from_seeds(&seed(1), &format!("{}\n", seed(2))).unwrap();
    /// assert_ne!(keys.current_public(), keys.next_public());
    ///
    /// // One seed twice rotates nothing.
    /// assert!(EnvelopeKeys::from_seeds(&seed(1), &seed(1)).is_err());
    /// // A key that is not 32 bytes is not a seed.
    /// assert!(EnvelopeKeys::from_seeds(&URL_SAFE_NO_PAD.encode([1u8; 31]), &seed(2)).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`EnvelopeKeyError::NotASeed`] for either, [`EnvelopeKeyError::SameKey`]
    /// if they decode to one seed.
    pub fn from_seeds(current: &str, next: &str) -> Result<Self, EnvelopeKeyError> {
        let current = decode_seed(Slot::Current, current)?;
        let next = decode_seed(Slot::Next, next)?;
        // Compared as seeds, not as public keys: two equal seeds are one key,
        // and nothing else is.
        if current == next {
            return Err(EnvelopeKeyError::SameKey);
        }
        let current = Ed25519Signer::from_seed(&current);
        let next = Ed25519Signer::from_seed(&next);
        Ok(Self {
            current_public: public_bytes(&current),
            next_public: public_bytes(&next),
            current,
        })
    }

    /// Reads and parses the two Secret mounts.
    ///
    /// # Errors
    ///
    /// [`EnvelopeKeyError::Read`] for a file that cannot be read, then
    /// whatever [`Self::from_seeds`] refuses.
    pub fn from_files(current: &Path, next: &Path) -> Result<Self, EnvelopeKeyError> {
        let read = |slot: Slot, path: &Path| {
            std::fs::read_to_string(path).map_err(|source| EnvelopeKeyError::Read {
                slot,
                path: path.to_owned(),
                source,
            })
        };
        Self::from_seeds(&read(Slot::Current, current)?, &read(Slot::Next, next)?)
    }

    /// The public key a merchant pins as the key answers are sealed with now.
    #[must_use]
    pub fn current_public(&self) -> [u8; 32] {
        self.current_public
    }

    /// The public key a merchant pins as the one answers will be sealed with
    /// after the next rotation.
    #[must_use]
    pub fn next_public(&self) -> [u8; 32] {
        self.next_public
    }

    /// Both public keys as unpadded base64url, `(current, next)` — the
    /// spelling a merchant's configuration takes and the boot log prints.
    #[must_use]
    pub fn public_keys_base64url(&self) -> (String, String) {
        (
            URL_SAFE_NO_PAD.encode(self.current_public),
            URL_SAFE_NO_PAD.encode(self.next_public),
        )
    }

    /// The signer that seals responses.
    #[must_use]
    pub fn into_signer(self) -> Ed25519Signer {
        self.current
    }
}

/// `ed25519_bytes` is `Some` for an Ed25519 key by construction; the fallback
/// is the all-zero key, which no seed derives, rather than a panic in a payment
/// binary — and the same values feed the boot log, where an operator would see
/// the zeros.
fn public_bytes(signer: &Ed25519Signer) -> [u8; 32] {
    signer.verify_key().ed25519_bytes().unwrap_or([0; 32])
}

fn decode_seed(slot: Slot, text: &str) -> Result<[u8; 32], EnvelopeKeyError> {
    let not_a_seed = |reason| EnvelopeKeyError::NotASeed { slot, reason };
    let text = text.trim();
    if text.is_empty() {
        return Err(not_a_seed("the file is empty"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| not_a_seed("it is not unpadded base64url"))?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| not_a_seed("it does not decode to 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(byte: u8) -> String {
        URL_SAFE_NO_PAD.encode([byte; 32])
    }

    #[test]
    fn a_seed_is_a_pure_function_of_its_text() {
        let a = EnvelopeKeys::from_seeds(&seed(1), &seed(2)).expect("keys");
        let b = EnvelopeKeys::from_seeds(&seed(1), &seed(2)).expect("keys");
        assert_eq!(a.current_public(), b.current_public());
        assert_eq!(a.next_public(), b.next_public());
        assert_ne!(a.current_public(), [0; 32]);
    }

    #[test]
    fn surrounding_whitespace_is_ignored_and_the_inside_is_not() {
        let padded = format!("\n  {}  \r\n", seed(1));
        assert!(EnvelopeKeys::from_seeds(&padded, &seed(2)).is_ok());
        let split = format!("{} {}", &seed(1)[..20], &seed(1)[20..]);
        assert!(EnvelopeKeys::from_seeds(&split, &seed(2)).is_err());
    }

    #[test]
    fn identical_seeds_are_refused_and_the_message_names_both_flags() {
        let error = EnvelopeKeys::from_seeds(&seed(7), &seed(7)).expect_err("same key");
        assert!(matches!(error, EnvelopeKeyError::SameKey));
        let message = error.to_string();
        assert!(message.contains("--envelope-signing-key-file"), "{message}");
        assert!(
            message.contains("--envelope-signing-key-next-file"),
            "{message}"
        );
        assert_eq!(error.category(), Category::Configuration);
    }

    #[test]
    fn each_malformed_seed_is_refused_naming_its_slot_and_quoting_nothing() {
        // 31 bytes: valid unpadded base64url, one short of a seed, and the kind
        // of string that must never be echoed back.
        let short = URL_SAFE_NO_PAD.encode([0x5a_u8; 31]);
        let secret_looking = short.as_str();
        for (text, fragment) in [
            ("", "empty"),
            (secret_looking, "32 bytes"),
            ("not base64url!!", "base64url"),
            // The standard alphabet is not the URL-safe one.
            ("++++++++++++++++++++++++++++++++++++++++++++", "base64url"),
        ] {
            let error = EnvelopeKeys::from_seeds(text, &seed(2)).expect_err("not a seed");
            let message = error.to_string();
            assert!(message.contains("current"), "{message}");
            assert!(message.contains(fragment), "{fragment}: {message}");
            assert!(!message.contains(secret_looking), "{message}");

            let error = EnvelopeKeys::from_seeds(&seed(1), text).expect_err("not a seed");
            assert!(error.to_string().contains("next"), "{error}");
        }
        // Padding is not in the format.
        let padded = base64::engine::general_purpose::URL_SAFE.encode([1u8; 31]);
        assert!(EnvelopeKeys::from_seeds(&padded, &seed(2)).is_err());
    }

    #[test]
    fn a_missing_file_names_its_path_and_flag() {
        let error = EnvelopeKeys::from_files(
            Path::new("/nonexistent/vpay/current"),
            Path::new("/nonexistent/vpay/next"),
        )
        .expect_err("no such file");
        let message = error.to_string();
        assert!(message.contains("/nonexistent/vpay/current"), "{message}");
        assert!(
            message.contains("VPAY_ENVELOPE_SIGNING_KEY_FILE"),
            "{message}"
        );
        assert_eq!(error.category(), Category::Configuration);
    }

    #[test]
    fn debug_prints_public_keys_only() {
        let keys = EnvelopeKeys::from_seeds(&seed(1), &seed(2)).expect("keys");
        let debug = format!("{keys:?}");
        assert!(!debug.contains(&seed(1)), "{debug}");
        assert!(!debug.contains(&seed(2)), "{debug}");
        assert!(debug.contains(&URL_SAFE_NO_PAD.encode(keys.current_public())));
    }
}
