//! The RS256 signing key this process holds: loaded once from a file at
//! boot, never persisted, and announced to the database so `/jwks.json` can
//! publish it (ADR-0009, ADR-0010,
//! [docs/flows/dashboard-auth.md](../../../../../docs/flows/dashboard-auth.md)'s
//! "JWKS publication and key rotation").
//!
//! The private half comes from a Kubernetes Secret mounted as a file
//! (`--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE`,
//! [`vpay_config::cli::ServerArgs`]). Migration
//! `0010_reshape-oauth-signing-keys.sql` is the decision this module
//! implements: the PEM is parsed exactly once per process and the database
//! never sees it — `oauth_signing_keys` holds only `kid`, `public_jwk` and a
//! validity window.
//!
//! # Why the `kid` is a thumbprint and not a name
//!
//! Every replica loads the *same* Secret and must agree on the `kid`, with no
//! coordination and no extra config value to keep in sync — otherwise two
//! pods sign tokens under two different `kid`s for the same key, and the
//! `oauth_signing_keys` row one of them wrote describes a `kid` the other
//! never uses. So the `kid` is *derived from the key*: the RFC 7638 JWK
//! thumbprint of its public half. Same PEM ⇒ same `kid`, everywhere, forever;
//! a different PEM ⇒ a different `kid`, which is exactly the signal
//! [`LoadedSigningKey::ensure_active_in_database`] uses to decide whether a
//! rotation happened.
//!
//! # Why `n`/`e` come from `TokenManager`, not from the `rsa` crate directly
//!
//! Both were available: this crate could parse the PEM itself with
//! `rsa::RsaPrivateKey::from_pkcs8_pem`/`from_pkcs1_pem` and derive the
//! modulus and exponent, or it could ask
//! `authkestra_engine::TokenManager::public_jwk()` for the values *it*
//! derived. This module asks the `TokenManager`.
//!
//! The reason is that the whole point of the published JWK is to describe the
//! key authkestra actually signs with. Deriving `n`/`e` independently creates
//! two derivations that agree today and could silently disagree later — a
//! different PEM branch taken, a base64 alphabet difference, a leading-zero
//! convention — and the failure mode is every merchant's token failing
//! signature verification against a JWKS that looks perfectly well-formed.
//! Reading them back off the `TokenManager` makes "vpay publishes the key
//! authkestra signs with" true by construction rather than by test. (The test
//! is there anyway: `the_published_jwk_is_the_key_authkestra_signs_with`.)
//!
//! The cost is that the PEM is parsed twice at boot: once with no `kid` to
//! learn `n`/`e`, then again with the thumbprint as the `kid`, because
//! `TokenManager` has no way to set a `kid` after construction and the
//! thumbprint cannot be known before `n`/`e` are. Twice per process start is
//! not a cost worth designing around. A secondary benefit: `rsa` stays out of
//! this crate's *production* dependency list (it is only a dev-dependency,
//! used by the tests below to mint keys).
//!
//! # What this module deliberately does not do
//!
//! It does not generate keys — `cargo xtask gen-signing-key` does, offline,
//! and its output is what an operator puts in the Secret. It does not zeroize
//! the PEM after parsing: the `String` read from disk is dropped normally and
//! its bytes stay in freed heap until overwritten. Closing that would mean
//! adding `zeroize` and a secret-string type, which is a real change worth
//! making deliberately rather than in passing; stating it is better than
//! implying the handling is airtight.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use authkestra_engine::token::TokenManager;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime};
use vpay_core::{Category, Classify};
use vpay_db::{ActivationOutcome, DbError, SigningKeys};

/// How long a key stays publishable in `/jwks.json` after it has been
/// rotated out.
///
/// **This is a default chosen here, not a decision recorded elsewhere.**
/// `docs/roadmap.md` lists "signing-key rotation overlap window" as an open
/// question — no ADR, flow doc or config value states a length, and this
/// constant does not close that question. It is the value this code uses
/// until a maintainer settles it.
///
/// The one hard constraint the number must satisfy: a token signed by the
/// old key has to keep verifying for the whole of its own lifetime, so the
/// window must exceed the access-token TTL by a margin wide enough that
/// clock skew, a slow rolling deploy and a resource server's own JWKS cache
/// interval all fit inside it. Against the 900 s access-token TTL `/v1` is
/// being built around, 24 h is 96×; against
/// [`crate::op::jwks::JWKS_CACHE_MAX_AGE`] (300 s) it is 288×. A window
/// merely *equal* to the TTL would strand tokens minted moments before a
/// rotation, so being generous here costs nothing but publishing one extra
/// public key for a day.
pub const ROTATION_OVERLAP: Duration = Duration::hours(24);

/// The smallest RSA modulus this deployment will sign with.
///
/// 2048 is the floor every current guideline puts on RSA signatures (NIST
/// SP 800-57's 112-bit security level, and what every JWT library and payment
/// scheme audit assumes). A shorter key is refused outright rather than
/// warned about: it would be accepted by `jsonwebtoken` and by every verifier
/// downstream, so nothing else in the system would ever catch it.
/// `cargo xtask gen-signing-key` generates 3072-bit keys, comfortably above
/// this.
const MIN_MODULUS_BITS: usize = 2048;

/// The signing key material a process holds, ready to sign with and ready to
/// publish.
///
/// Cheap to clone (the `TokenManager` is `Arc`-shared) so the assembler can
/// put it in router state next to the pool.
#[derive(Clone)]
pub struct LoadedSigningKey {
    kid: String,
    public_jwk: Value,
    token_manager: Arc<TokenManager>,
}

/// Shows the `kid` and the public JWK — both are published to the world at
/// `/jwks.json`, so neither is a secret — and never the [`TokenManager`],
/// which holds the private half. Hand-written rather than derived because
/// `TokenManager` has no `Debug` impl of its own; that it has none is a
/// helpful accident and this impl does not work around it.
impl fmt::Debug for LoadedSigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedSigningKey")
            .field("kid", &self.kid)
            .field("public_jwk", &self.public_jwk)
            .finish_non_exhaustive()
    }
}

impl LoadedSigningKey {
    /// Parses an RSA private key in PEM form (PKCS#8 `BEGIN PRIVATE KEY` or
    /// PKCS#1 `BEGIN RSA PRIVATE KEY`; both are accepted, because
    /// `TokenManager::new_asymmetric` tries both in that order) and derives
    /// everything else from it.
    ///
    /// `issuer` is stamped into every token this key signs, and must be the
    /// same `iss` the resource-server validator requires
    /// ([`crate::resource_auth::JwtValidator::new`]).
    ///
    /// # Errors
    ///
    /// [`SigningKeyError::NotAnRsaPrivateKey`] if the bytes are not a usable
    /// RSA private key, [`SigningKeyError::KeyTooShort`] if they are one but
    /// below the 2048-bit floor (`MIN_MODULUS_BITS`, private: the floor is a
    /// property of this module, not something a caller may vary), and the two
    /// unreachable-in-practice
    /// variants documented on [`SigningKeyError`] if authkestra's own JWK
    /// derivation ever changes shape.
    pub fn from_pem(pem: &str, issuer: &str) -> Result<Self, SigningKeyError> {
        // First construction: no `kid`, because the `kid` is a function of
        // the `n`/`e` this call is what produces. `new_asymmetric` invents a
        // random UUID `kid` when handed `None`; it is discarded with this
        // value and never reaches a token or the database.
        let probe = TokenManager::new_asymmetric(pem.as_bytes(), Some(issuer.to_owned()), None)
            .map_err(SigningKeyError::NotAnRsaPrivateKey)?;

        let (n, e) = rsa_components(&probe)?;

        let bits = modulus_bits(&n)?;
        if bits < MIN_MODULUS_BITS {
            return Err(SigningKeyError::KeyTooShort { bits });
        }

        let kid = rfc7638_thumbprint(&n, &e);

        let token_manager = TokenManager::new_asymmetric(
            pem.as_bytes(),
            Some(issuer.to_owned()),
            Some(kid.clone()),
        )
        .map_err(SigningKeyError::NotAnRsaPrivateKey)?;

        Ok(Self {
            public_jwk: json!({
                "kty": "RSA",
                "n": n,
                "e": e,
                "alg": "RS256",
                "use": "sig",
                "kid": kid,
            }),
            kid,
            token_manager: Arc::new(token_manager),
        })
    }

    /// [`Self::from_pem`] against a file — the Kubernetes Secret mount.
    ///
    /// # Errors
    ///
    /// [`SigningKeyError::Read`] if the file cannot be read (it does not
    /// exist, the Secret was not mounted, the mode is wrong), plus
    /// everything [`Self::from_pem`] can return.
    pub fn from_file(path: &Path, issuer: &str) -> Result<Self, SigningKeyError> {
        let pem = fs::read_to_string(path).map_err(|source| SigningKeyError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_pem(&pem, issuer)
    }

    /// The RFC 7638 thumbprint every token signed by this key carries in its
    /// JWT header, and the primary key of its `oauth_signing_keys` row.
    #[must_use]
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The JWK `/jwks.json` publishes for this key: `kty`, `n`, `e`, `alg`,
    /// `use`, `kid`. Contains no private key material — see this module's
    /// header for why it is read back off the `TokenManager` rather than
    /// derived a second time.
    #[must_use]
    pub fn public_jwk(&self) -> &Value {
        &self.public_jwk
    }

    /// The signer itself, for the OP's token endpoint. Cloned `Arc`, so the
    /// PEM is parsed once per process no matter how many places hold this.
    #[must_use]
    pub fn token_manager(&self) -> Arc<TokenManager> {
        Arc::clone(&self.token_manager)
    }

    /// Records this key as the active one, rotating only if it is not
    /// already — what every replica calls at boot, before serving traffic.
    ///
    /// The whole read-decide-write runs inside one locked transaction in
    /// [`vpay_db::SigningKeys::ensure_active_signing_key`], so N replicas booting at once
    /// with the same Secret produce one rotation between them, not N. This
    /// method's own job is small and is the reason it lives here rather than
    /// in `vpay-db`: it supplies [`ROTATION_OVERLAP`], which is *policy*, to
    /// a repository layer whose own documentation says it never invents a
    /// window.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query`] if the database cannot be read or written,
    /// and `DbError::SigningKeyRetired` for the deliberate failure when this
    /// `kid` exists but is retired (a rollback to an older key), documented
    /// on [`vpay_db::SigningKeys::ensure_active_signing_key`]. The two are separate
    /// variants because they call for opposite responses from a supervisor:
    /// the first is "wait for Postgres" (exit 69), the second is "fix the
    /// deploy" (exit 78, `Category::Configuration`), and restarting into a
    /// rollback forever is what the single variant used to cause.
    pub async fn ensure_active_in_database(
        &self,
        repositories: &dyn SigningKeys,
    ) -> Result<ActivationOutcome, DbError> {
        repositories
            .ensure_active_signing_key(
                &self.kid,
                &self.public_jwk,
                OffsetDateTime::now_utc() + ROTATION_OVERLAP,
            )
            .await
    }
}

/// Reads the base64url `n` and `e` back off a freshly built
/// [`TokenManager`].
///
/// Every `None` here is unreachable for a manager built by
/// `new_asymmetric`, which populates all three unconditionally — but this
/// crate denies `unwrap`/`expect` in production code for exactly the reason
/// that "unreachable" is a property of today's upstream version, not of the
/// type system. A future authkestra that returned an OKP JWK here would land
/// on [`SigningKeyError::PublicKeyUnavailable`] instead of a panic in a
/// payment binary's startup path.
fn rsa_components(manager: &TokenManager) -> Result<(String, String), SigningKeyError> {
    let jwk = manager
        .public_jwk()
        .ok_or(SigningKeyError::PublicKeyUnavailable)?;
    if jwk.kty != "RSA" {
        return Err(SigningKeyError::PublicKeyUnavailable);
    }
    let n = jwk.n.ok_or(SigningKeyError::PublicKeyUnavailable)?;
    let e = jwk.e.ok_or(SigningKeyError::PublicKeyUnavailable)?;
    Ok((n, e))
}

/// The bit length of a base64url-encoded big-endian RSA modulus.
///
/// Leading zero bytes are skipped before counting: `rsa`'s `to_bytes_be`
/// never emits them, but a JWK that arrived from somewhere else might, and a
/// 2048-bit key padded to 257 bytes must not be counted as 2056-bit.
fn modulus_bits(n_b64: &str) -> Result<usize, SigningKeyError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(n_b64)
        .map_err(|_| SigningKeyError::MalformedModulus)?;

    let mut significant = bytes.iter().skip_while(|byte| **byte == 0);
    let Some(first) = significant.next() else {
        return Ok(0);
    };
    let remaining = significant.count();
    Ok((remaining * 8) + (8 - first.leading_zeros() as usize))
}

/// The RFC 7638 §3 JWK thumbprint of an RSA public key, base64url with no
/// padding.
///
/// The construction is fixed by the RFC and is the whole reason this is a
/// stable identifier: SHA-256 over the JSON object containing *only* the
/// required members for the key type (`e`, `kty`, `n` for RSA), with no
/// whitespace and the member names in lexicographic order. Any deviation —
/// a different member order, a pretty-printed separator, an extra `alg` —
/// produces a different digest, so this builds the string by hand rather
/// than through `serde_json`, whose member ordering is a property of a
/// feature flag (`preserve_order`) rather than of this function.
///
/// The values need no JSON escaping: both are base64url, an alphabet of
/// `A-Za-z0-9-_` with no `"` or `\`.
///
/// Pinned to the RFC's own worked example in the tests, not to this
/// implementation's output.
fn rfc7638_thumbprint(n: &str, e: &str) -> String {
    let canonical = format!(r#"{{"e":"{e}","kty":"RSA","n":"{n}"}}"#);
    URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes()))
}

/// Everything that can go wrong turning a Secret mount into a usable signing
/// key. All of it is [`Category::Configuration`]: a missing, unreadable,
/// wrong-format or too-short key is a deploy that must be fixed, never
/// something to retry and never something a caller did.
///
/// **No variant carries key material.** The two that wrap a library error
/// keep it as a `source` rather than folding it into this type's own
/// `Display`, and the wrapped errors are parser diagnostics ("invalid PKCS#8
/// ASN.1", "invalid RSA key") that describe the *shape* of the input and
/// never echo it. [`Self::Read`] does name the file path, deliberately: a
/// path is not a secret and "which file did it try" is the first thing an
/// operator needs.
#[derive(Debug, thiserror::Error)]
pub enum SigningKeyError {
    /// The key file could not be read: not mounted, wrong path, wrong mode.
    /// By far the most likely failure in a real deploy.
    #[error("cannot read the OAuth signing key file at {path}")]
    Read {
        /// The path that was tried. Echoed because it is not secret and is
        /// the whole diagnosis.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The file was read but is not a usable RSA private key — an EC or
    /// Ed25519 key, a public key, a certificate, a truncated PEM, or
    /// something that is not PEM at all.
    #[error(
        "the OAuth signing key is not a usable RSA private key \
         (expected a PKCS#8 or PKCS#1 RSA private-key PEM)"
    )]
    NotAnRsaPrivateKey(#[source] authkestra_engine::auth::error::AuthError),

    /// The key parsed, but its modulus is below the 2048-bit floor
    /// (`MIN_MODULUS_BITS`).
    /// Refused rather than warned about: nothing downstream would catch it.
    #[error("the OAuth signing key is {bits}-bit; RS256 signing requires at least 2048")]
    KeyTooShort {
        /// The modulus length actually found. A public property of a public
        /// key, safe to log and safe to tell an operator.
        bits: usize,
    },

    /// `TokenManager` did not expose an RSA public JWK for a key it had just
    /// accepted. Unreachable at the pinned `authkestra-engine` version —
    /// `new_asymmetric` always populates `kty`/`n`/`e` — and exists so that
    /// an upstream change lands as a typed startup error rather than as an
    /// `unwrap` in a payment binary.
    #[error("authkestra did not expose an RSA public JWK for this signing key")]
    PublicKeyUnavailable,

    /// The `n` authkestra produced is not valid base64url. Unreachable for
    /// the same reason as [`Self::PublicKeyUnavailable`]; kept for the same
    /// reason.
    #[error("the signing key's modulus is not valid base64url")]
    MalformedModulus,
}

impl Classify for SigningKeyError {
    /// Every variant is the deployment's problem, fixed by mounting a
    /// correct Secret and redeploying — never by retrying, and never by a
    /// caller changing their request. [`Category::Configuration`] is what
    /// turns that into exit code 78 at startup and a 500 (not a 503) if it
    /// were ever to surface on a request path.
    fn category(&self) -> Category {
        Category::Configuration
    }

    /// More specific than the category's `misconfigured` default, because
    /// these four failures have four different fixes and an operator reads
    /// the code before the message.
    fn code(&self) -> &'static str {
        match self {
            Self::Read { .. } => "signing_key_unreadable",
            Self::NotAnRsaPrivateKey(_) => "signing_key_not_rsa",
            Self::KeyTooShort { .. } => "signing_key_too_short",
            Self::PublicKeyUnavailable | Self::MalformedModulus => "signing_key_unusable",
        }
    }
}

#[cfg(test)]
mod tests {
    use rsa::pkcs1::EncodeRsaPrivateKey as _;
    use rsa::pkcs8::{EncodePrivateKey as _, EncodePublicKey as _, LineEnding};
    use vpay_core::{Retry, Severity};

    use super::*;

    const ISSUER: &str = "https://vpay.test";

    /// Generates a real RSA keypair and returns its PKCS#8 PEM. Slow enough
    /// (a second or two at 2048 bits) that tests share the two below rather
    /// than each minting their own.
    fn pkcs8_pem(bits: usize) -> String {
        let mut rng = rand::rngs::OsRng;
        let key = rsa::RsaPrivateKey::new(&mut rng, bits).expect("rsa key generation succeeds");
        key.to_pkcs8_pem(LineEnding::LF)
            .expect("pkcs8 pem encoding succeeds")
            .to_string()
    }

    fn loaded_2048() -> (String, LoadedSigningKey) {
        let pem = pkcs8_pem(2048);
        let key = LoadedSigningKey::from_pem(&pem, ISSUER).expect("a 2048-bit rsa key is accepted");
        (pem, key)
    }

    fn jwk_str(key: &LoadedSigningKey, member: &str) -> String {
        key.public_jwk()
            .get(member)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("published jwk has a string `{member}`"))
            .to_string()
    }

    /// The decisive test for the `kid`: RFC 7638 §3.1's own worked example,
    /// modulus and exponent verbatim from the RFC, against the thumbprint
    /// the RFC states.
    ///
    /// Pinned to the standard rather than to this implementation's own
    /// output, so it fails for any deviation that would make vpay's `kid`
    /// disagree with what any other RFC-conformant implementation computes
    /// for the same key — swapping the member order to `{"kty","e","n"}`,
    /// adding a space after the colons, or including `alg`/`use` all change
    /// the digest and fail here.
    #[test]
    fn the_thumbprint_matches_rfc_7638s_own_worked_example() {
        let n = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4\
                 cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiF\
                 V4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6C\
                 f0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9\
                 c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTW\
                 hAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1\
                 jF44-csFCur-kEgU8awapJzKnqDKgw"
            .replace([' ', '\n'], "");

        assert_eq!(
            rfc7638_thumbprint(&n, "AQAB"),
            "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs",
            "the thumbprint must be SHA-256 over {{\"e\",\"kty\",\"n\"}} in that order, \
             with no whitespace — RFC 7638 §3.1"
        );
    }

    /// The property replicas depend on: the `kid` is a function of the key
    /// alone. Same PEM twice ⇒ same `kid` (no coordination needed between
    /// pods); a different key ⇒ a different `kid` (so a rotation is
    /// detectable). Also proves PKCS#1 and PKCS#8 encodings of the *same*
    /// key agree, since the Secret's format is the operator's choice.
    #[test]
    fn the_kid_is_a_function_of_the_key_and_not_of_the_encoding_or_the_process() {
        let (pem, first) = loaded_2048();
        let again = LoadedSigningKey::from_pem(&pem, ISSUER).expect("the same pem loads again");
        assert_eq!(
            first.kid(),
            again.kid(),
            "two replicas holding the same PEM must compute the same kid"
        );

        let mut rng = rand::rngs::OsRng;
        let key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa key generation succeeds");
        let pkcs8 = key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("pkcs8 encoding succeeds")
            .to_string();
        let pkcs1 = key
            .to_pkcs1_pem(LineEnding::LF)
            .expect("pkcs1 encoding succeeds")
            .to_string();
        let from_pkcs8 = LoadedSigningKey::from_pem(&pkcs8, ISSUER).expect("pkcs8 is accepted");
        let from_pkcs1 = LoadedSigningKey::from_pem(&pkcs1, ISSUER).expect("pkcs1 is accepted");
        assert_eq!(
            from_pkcs8.kid(),
            from_pkcs1.kid(),
            "the kid identifies the key, not the container format it arrived in"
        );

        assert_ne!(
            first.kid(),
            from_pkcs8.kid(),
            "a different key must produce a different kid, or a rotation is undetectable"
        );
    }

    /// The published JWK must be the key authkestra signs with — the whole
    /// reason `n`/`e` are read back off the `TokenManager` (module docs).
    /// Compares member by member against `TokenManager::public_jwk()`,
    /// including the `kid` a token's header will carry.
    #[test]
    fn the_published_jwk_is_the_key_authkestra_signs_with() {
        let (_pem, key) = loaded_2048();
        let signing_jwk = key
            .token_manager()
            .public_jwk()
            .expect("the token manager exposes the public jwk it signs with");

        assert_eq!(signing_jwk.kty, "RSA");
        assert_eq!(signing_jwk.alg.as_deref(), Some("RS256"));
        assert_eq!(
            signing_jwk.kid.as_deref(),
            Some(key.kid()),
            "authkestra must stamp the thumbprint kid on every token it signs, or the JWKS \
             lookup finds nothing"
        );
        assert_eq!(signing_jwk.n.as_deref(), Some(jwk_str(&key, "n").as_str()));
        assert_eq!(signing_jwk.e.as_deref(), Some(jwk_str(&key, "e").as_str()));
    }

    /// The exact JSON `/jwks.json` will publish, and the internal
    /// consistency the validator relies on: the `kid` member *is* the
    /// thumbprint of the `n`/`e` members alongside it.
    #[test]
    fn the_published_jwk_has_the_six_members_a_verifier_needs_and_a_self_consistent_kid() {
        let (_pem, key) = loaded_2048();

        assert_eq!(jwk_str(&key, "kty"), "RSA");
        assert_eq!(jwk_str(&key, "alg"), "RS256");
        assert_eq!(jwk_str(&key, "use"), "sig");
        assert_eq!(jwk_str(&key, "kid"), key.kid());
        assert_eq!(jwk_str(&key, "e"), "AQAB");
        assert_eq!(
            key.public_jwk().as_object().map(serde_json::Map::len),
            Some(6),
            "publishing anything beyond kty/n/e/alg/use/kid is unintended"
        );

        assert_eq!(
            rfc7638_thumbprint(&jwk_str(&key, "n"), &jwk_str(&key, "e")),
            key.kid(),
            "the kid must be the thumbprint of the very JWK it is published inside"
        );
    }

    /// 2048 is a floor, not a suggestion. A 1024-bit key parses fine and
    /// would be accepted by every verifier downstream, so this is the only
    /// place it can be caught.
    #[test]
    fn a_key_below_2048_bits_is_refused() {
        let pem = pkcs8_pem(1024);
        let error = LoadedSigningKey::from_pem(&pem, ISSUER)
            .expect_err("a 1024-bit key must not be accepted");

        assert!(
            matches!(error, SigningKeyError::KeyTooShort { bits: 1024 }),
            "expected KeyTooShort {{ bits: 1024 }}, got {error:?}"
        );
        assert_eq!(error.code(), "signing_key_too_short");
        assert_eq!(error.category(), Category::Configuration);
        // Derived, not chosen at the call site: a bad key file is not
        // retryable and is loud enough to page nobody but the deployer.
        assert_eq!(error.retry(), Retry::Never);
        assert_eq!(error.severity(), Severity::Error);
    }

    /// The modulus bit count must come from the key, not from the length of
    /// its base64 — a padded leading zero byte must not read as a longer
    /// key, and 2047 bits must not round up to 2048.
    #[test]
    fn modulus_bits_counts_significant_bits_only() {
        fn bits(bytes: &[u8]) -> usize {
            modulus_bits(&URL_SAFE_NO_PAD.encode(bytes)).expect("the fixture is valid base64url")
        }

        // 0x01 => 1 bit; 0x00 0x01 => still 1 bit; 0xFF => 8 bits.
        assert_eq!(bits(&[0x01]), 1);
        assert_eq!(bits(&[0x00, 0x01]), 1);
        assert_eq!(bits(&[0xFF]), 8);

        // A 2048-bit modulus padded to 257 bytes is 2048 bits, not 2056.
        let mut padded = vec![0x00, 0x80];
        padded.extend(std::iter::repeat_n(0x00, 255));
        assert_eq!(bits(&padded), 2048);

        // One bit short of the floor is 2047, and `from_pem` refuses it.
        let mut short = vec![0x7F];
        short.extend(std::iter::repeat_n(0xFF, 255));
        assert_eq!(bits(&short), 2047);
    }

    /// Everything that is not an RSA private key is one error, not a
    /// half-loaded key: an Ed25519 key, an RSA *public* key, an empty file,
    /// and plain prose.
    #[test]
    fn anything_that_is_not_an_rsa_private_key_is_refused() {
        let mut rng = rand::rngs::OsRng;
        let rsa_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen succeeds");
        let public_pem = rsa_key
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .expect("public pem encoding succeeds");

        for (label, pem) in [
            ("an RSA public key", public_pem.as_str()),
            ("an empty file", ""),
            ("prose", "this is not a key at all\n"),
            (
                "a truncated PEM",
                "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
            ),
        ] {
            let Err(error) = LoadedSigningKey::from_pem(pem, ISSUER) else {
                panic!("{label} must be refused, not loaded as a signing key");
            };
            assert!(
                matches!(error, SigningKeyError::NotAnRsaPrivateKey(_)),
                "{label} should be NotAnRsaPrivateKey, got {error:?}"
            );
            assert_eq!(error.code(), "signing_key_not_rsa");
        }
    }

    /// A missing Secret mount is the most likely production failure, so the
    /// error has to name the path — and only the path.
    #[test]
    fn a_missing_key_file_names_the_path_it_tried() {
        let path = Path::new("/nonexistent/vpay/oauth-signing-key.pem");
        let error = LoadedSigningKey::from_file(path, ISSUER)
            .expect_err("a path that does not exist must not load");

        assert!(matches!(error, SigningKeyError::Read { .. }));
        assert_eq!(error.code(), "signing_key_unreadable");
        assert!(
            error.to_string().contains("/nonexistent/vpay/"),
            "the operator needs to see which file was tried: {error}"
        );
    }

    /// No error's `Display`, and nothing in its `source` chain, may contain
    /// any run of the PEM it was handed. This is the test that would fail if
    /// a variant were ever changed to interpolate the input for
    /// "debuggability".
    #[test]
    fn no_error_message_or_source_chain_echoes_the_pem() {
        use std::error::Error as _;

        let pem = pkcs8_pem(2048);
        // Corrupt the body while keeping the PEM armour, so the failure
        // happens deep inside the parser with the real bytes in hand.
        let corrupted = pem.replacen("MII", "MIQ", 1);

        let error = LoadedSigningKey::from_pem(&corrupted, ISSUER)
            .expect_err("a corrupted pem body must not parse");

        let mut rendered = error.to_string();
        let mut source: Option<&(dyn std::error::Error + 'static)> = error.source();
        while let Some(current) = source {
            rendered.push(' ');
            rendered.push_str(&current.to_string());
            source = current.source();
        }

        let body: String = corrupted
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        for window in body.as_bytes().windows(16) {
            let fragment = std::str::from_utf8(window).expect("base64 is ascii");
            assert!(
                !rendered.contains(fragment),
                "a fragment of the key file leaked into the error chain: {rendered}"
            );
        }
    }

    /// The overlap window is only correct relative to the access-token TTL:
    /// a key must stay publishable for longer than any token it signed can
    /// live, with room for skew and a rolling deploy. Fails if someone
    /// shortens `ROTATION_OVERLAP` to something a 900 s token could outlive.
    #[test]
    fn the_rotation_overlap_dwarfs_the_access_token_ttl_it_has_to_cover() {
        let access_token_ttl = Duration::seconds(900);
        assert!(
            ROTATION_OVERLAP >= access_token_ttl * 24,
            "a retired key must outlive by a wide margin every token it signed; \
             {ROTATION_OVERLAP} is not enough against a {access_token_ttl} token"
        );
        assert_eq!(ROTATION_OVERLAP, Duration::hours(24));
    }
}
