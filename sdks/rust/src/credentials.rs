//! A merchant's signing key and the server keys it pins (VP-5).
//!
//! Every `/v1` request is a COSE_Sign1 message signed by an **Ed25519** key
//! the merchant registered (`merchant_clients[].cose_keys`), and every answer
//! is one signed by vpay's response key. Both halves are configured here and
//! nowhere else:
//!
//! * [`Credentials`] — the merchant's private key (a 32-byte seed, or a
//!   callback for a key that never leaves a KMS or an HSM) and the `client_id`
//!   it is registered under. The `client_id` is **not sent**: vpay finds the
//!   merchant by the key that verified. It is kept for logs and for the
//!   operator who has to match a key to a registration.
//! * [`ServerKeys`] — the response keys the client trusts, `{current, next}`.
//!   An answer sealed by any other key is refused (`Error::Unverified`), which is
//!   what makes "pinned" mean something: a stolen merchant key cannot be turned
//!   into a man in the middle by anyone who does not also hold vpay's.
//!
//! There are no tokens: `client_credentials`, `private_key_jwt`, an access
//! token cache and a re-authentication retry are gone with the OAuth path
//! (ADR-0010 is superseded by VP-3/VP-4). RSA keys are not accepted: COSE has
//! no RS256 here, and a registered key is Ed25519 or it is nothing.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cratestack_client_rust::cose::{CoseSigner, CoseVerifyKey, Ed25519Signer, ExternalSigner};
use cratestack_core::CratestackError;

use crate::error::ConfigError;

/// A merchant's Ed25519 signing key, in the shape `/v1` needs.
///
/// [`fmt::Debug`] is hand-written to print the `client_id` and the **public**
/// key only (`tests/debug_redaction.rs`).
pub struct Credentials {
    client_id: String,
    signer: Arc<dyn CoseSigner>,
    public_key: [u8; 32],
}

impl Credentials {
    /// Signs with the Ed25519 key whose 32-byte RFC 8032 seed is `seed`.
    ///
    /// ```
    /// use vpay_sdk::Credentials;
    ///
    /// let credentials = Credentials::ed25519_seed("merchant_acme", &[7; 32]).unwrap();
    /// // The public half is what the operator registers under `cose_keys`.
    /// assert_eq!(credentials.public_key().len(), 32);
    /// ```
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidSeed`] if the signer reports no Ed25519 public
    /// key for the seed. It cannot for an `Ed25519Signer`, which is exactly why
    /// it is an error and not a zeroed key: a credential whose public half is
    /// `[0; 32]` would register nothing and fail at vpay as the coarse `401`.
    pub fn ed25519_seed(
        client_id: impl Into<String>,
        seed: &[u8; 32],
    ) -> Result<Self, ConfigError> {
        let signer = Ed25519Signer::from_seed(seed);
        let public_key = signer
            .verify_key()
            .ed25519_bytes()
            .ok_or(ConfigError::InvalidSeed)?;
        Ok(Self {
            client_id: client_id.into(),
            signer: Arc::new(signer),
            public_key,
        })
    }

    /// [`Self::ed25519_seed`] from the text a seed file holds: 32 bytes as
    /// unpadded base64url, surrounding whitespace ignored. The same format
    /// `cargo xtask gen-envelope-key` writes and vpay's own envelope key files
    /// use.
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidSeed`] if the text is not 32 bytes of unpadded
    /// base64url. The error never quotes it.
    ///
    /// ```
    /// use vpay_sdk::Credentials;
    ///
    /// let seed = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc"; // [7; 32]
    /// let a = Credentials::ed25519_seed_base64url("m", &format!("{seed}\n")).unwrap();
    /// assert_eq!(a.public_key(), Credentials::ed25519_seed("m", &[7; 32]).unwrap().public_key());
    /// assert!(Credentials::ed25519_seed_base64url("m", "not a seed").is_err());
    /// ```
    pub fn ed25519_seed_base64url(
        client_id: impl Into<String>,
        seed: &str,
    ) -> Result<Self, ConfigError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(seed.trim())
            .map_err(|_| ConfigError::InvalidSeed)?;
        let seed = <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| ConfigError::InvalidSeed)?;
        Self::ed25519_seed(client_id, &seed)
    }

    /// Signs with a key this process cannot read: `sign` receives the exact
    /// bytes to sign and returns the 64-byte pure-Ed25519 signature over them
    /// (a KMS `ED25519` key, a Node `crypto.sign(null, …)` callback).
    /// `public_key` is the matching registered key, and a signature that does
    /// not verify under it is refused here rather than failing at vpay as the
    /// coarse `401`.
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidPublicKey`] if `public_key` is not a point.
    pub fn ed25519_external<F, Fut>(
        client_id: impl Into<String>,
        public_key: &[u8; 32],
        sign: F,
    ) -> Result<Self, ConfigError>
    where
        F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<u8>, CratestackError>> + Send + 'static,
    {
        let signer =
            ExternalSigner::ed25519(public_key, sign).map_err(|_| ConfigError::InvalidPublicKey)?;
        Ok(Self {
            client_id: client_id.into(),
            signer: Arc::new(signer),
            public_key: *public_key,
        })
    }

    /// The public half, 32 bytes: what `merchant_clients[].cose_keys` holds.
    #[must_use]
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// [`Self::public_key`] as unpadded base64url, the spelling `cose_keys` takes.
    #[must_use]
    pub fn public_key_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.public_key)
    }

    /// The `client_id` this key is registered under (never sent).
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub(crate) fn signer(&self) -> Arc<dyn CoseSigner> {
        Arc::clone(&self.signer)
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field("public_key", &self.public_key_base64url())
            .field("signer", &"[redacted]")
            .finish()
    }
}

/// The response keys a client trusts: the one vpay seals answers with now, and
/// the one it will after the next rotation.
///
/// Pinning both is what lets a rotation happen without a flag day on the
/// merchant's side: the operator hands out `{current, next}` ahead of time,
/// vpay switches, and an answer under either verifies. Pinning *neither* is not
/// a configuration this crate has: [`crate::ClientBuilder::build`] refuses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerKeys {
    current: [u8; 32],
    next: Option<[u8; 32]>,
}

impl ServerKeys {
    /// Pins `current` and `next`, 32-byte Ed25519 public keys.
    #[must_use]
    pub fn new(current: [u8; 32], next: [u8; 32]) -> Self {
        Self {
            current,
            next: Some(next),
        }
    }

    /// Pins one key and no successor. Honest for a first deployment that has
    /// not prepared a rotation; [`Self::new`] is what an operator should hand
    /// out.
    #[must_use]
    pub fn current_only(current: [u8; 32]) -> Self {
        Self {
            current,
            next: None,
        }
    }

    /// [`Self::new`] from the base64url the operator publishes (the boot log of
    /// `vpay-server`, or `cargo xtask gen-envelope-key`).
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidPublicKey`] if either is not 32 bytes of unpadded
    /// base64url.
    ///
    /// ```
    /// use vpay_sdk::ServerKeys;
    ///
    /// let a = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"; // [1; 32]
    /// let b = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI"; // [2; 32]
    /// assert_eq!(ServerKeys::from_base64url(a, b).unwrap(), ServerKeys::new([1; 32], [2; 32]));
    /// assert!(ServerKeys::from_base64url(a, "short").is_err());
    /// ```
    pub fn from_base64url(current: &str, next: &str) -> Result<Self, ConfigError> {
        let decode = |text: &str| {
            URL_SAFE_NO_PAD
                .decode(text.trim())
                .ok()
                .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
                .ok_or(ConfigError::InvalidPublicKey)
        };
        Ok(Self::new(decode(current)?, decode(next)?))
    }

    /// The pinned keys as verification keys, `current` first.
    pub(crate) fn verify_keys(&self) -> Result<Vec<CoseVerifyKey>, ConfigError> {
        std::iter::once(&self.current)
            .chain(self.next.iter())
            .map(|key| CoseVerifyKey::ed25519(key).map_err(|_| ConfigError::InvalidPublicKey))
            .collect()
    }
}
