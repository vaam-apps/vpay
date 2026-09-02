//! Merchant credentials and `private_key_jwt` client-assertion minting
//! (RFC 7523 §2.2), per `docs/flows/merchant-auth.md`.
//!
//! # A port, and where it deviates
//!
//! This is a **reimplementation**, not a call through, of
//! `authkestra_engine::client_assertion::mint_client_assertion`
//! (`authkestra-engine-0.7.1/src/client_assertion.rs`, pinned `=0.7.1`). That
//! crate is not a dependency of this one — it reaches the test binaries only
//! transitively, through the dev-dependency on `authkestra-op` — and a
//! merchant SDK deliberately does not take vpay's own OP engine as a runtime
//! dependency just to sign a JWT.
//!
//! Mirrored exactly, because the verifier reads them: the claim set and its
//! order (`iss`, `sub`, `aud`, `jti`, `exp`, `iat` — `AssertionClaims` in
//! both), `iss == sub == client_id`, a fresh UUIDv4 `jti` per call, `iat` =
//! now, and the same 300 s ceiling
//! (`MAX_CLIENT_ASSERTION_LIFETIME_SECS`/[`MAX_ASSERTION_LIFETIME_SECS`]).
//!
//! Deviations, both deliberate:
//!
//! 1. **An out-of-range lifetime is refused, not clamped.** The engine's
//!    minter calls `lifetime_secs.clamp(1, MAX_CLIENT_ASSERTION_LIFETIME_SECS)`
//!    because it trusts its caller — vpay's own OP-issuing code — to have
//!    validated already. A merchant SDK has no such caller, and
//!    `docs/flows/merchant-auth.md` requires the SDKs to "reject a configured
//!    lifetime outside that range at construction rather than clamping
//!    silently": a clamped value mints assertions whose `exp` differs from
//!    what the caller asked for, for a reason invisible at the call site. So
//!    [`crate::ClientBuilder::build`] refuses, and [`mint_client_assertion`]
//!    refuses too for anyone calling it directly.
//! 2. **RSA/RS256 only.** The engine takes the algorithm and an
//!    already-built `EncodingKey` as parameters, so it also mints EdDSA and
//!    ES\* assertions. [`Credentials`] accepts only an RSA PEM and always
//!    signs `RS256`, matching `docs/flows/merchant-auth.md`'s header table
//!    ("the registered JWK is RSA"). The OP's `assertion_algorithms` would
//!    accept `RS384`/`RS512`/`PS*` from an RSA key as well; narrowing to one
//!    is a smaller surface, not a compatibility gap. Registering an EC or
//!    Ed25519 merchant key would need a new constructor here.
//!
//! The claim set is not merely asserted against the tables above: it is
//! handed to the real `authkestra_op::client_assertion::verify_client_assertion`
//! at the pinned version — see `tests/op_conformance.rs`.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use uuid::Uuid;

use crate::error::ConfigError;

/// The `client_assertion_type` RFC 7523 §2.2 requires for a `private_key_jwt`
/// assertion. Re-declared here rather than imported from `authkestra-engine`
/// (a dev-only dependency of this crate) so the constant is available in
/// non-test builds too — it is sent on every token request.
pub const CLIENT_ASSERTION_TYPE_JWT_BEARER: &str =
    "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

/// Lower bound on a configurable assertion lifetime. RFC 7523 §3 requires
/// `exp` to be present but sets no floor; `0` or negative would mint an
/// already-expired assertion, so `1` is the practical minimum.
pub const MIN_ASSERTION_LIFETIME_SECS: u64 = 1;

/// Upper bound, matching `authkestra_engine::client_assertion::
/// MAX_CLIENT_ASSERTION_LIFETIME_SECS` at the pinned `=0.7.1` — the same
/// ceiling the OP verifier this crate's conformance test exercises enforces
/// on the receiving end. A longer-lived assertion would only fail there for
/// a reason invisible from here, so the SDK refuses it before it is minted.
pub const MAX_ASSERTION_LIFETIME_SECS: u64 = 300;

/// A merchant's RSA keypair, in the shape `/v1` authentication needs: the
/// `client_id` vpay registered, the private key to sign assertions with, and
/// — only when more than one key is registered for this client — the `kid`
/// naming which one.
///
/// vpay never sees, and this type never exposes, the private key material
/// itself: [`fmt::Debug`] is hand-written specifically to keep it out of log
/// lines and test failure messages (see `tests/debug_redaction.rs`).
pub struct Credentials {
    client_id: String,
    encoding_key: EncodingKey,
    kid: Option<String>,
}

impl Credentials {
    /// Loads an RSA private key from a PKCS#1 or PKCS#8 PEM (`jsonwebtoken`'s
    /// `EncodingKey::from_rsa_pem` accepts either), for `client_id`.
    ///
    /// # Errors
    /// [`ConfigError::InvalidPrivateKey`] if `pem` is not a parseable RSA key.
    pub fn rsa_pem(client_id: impl Into<String>, pem: &str) -> Result<Self, ConfigError> {
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|e| ConfigError::InvalidPrivateKey(e.to_string()))?;
        Ok(Self {
            client_id: client_id.into(),
            encoding_key,
            kid: None,
        })
    }

    /// Names which registered key this credential signs with.
    ///
    /// Required by the OP's `select_key` whenever a merchant has more than
    /// one key registered (`docs/flows/merchant-auth.md`'s header table);
    /// optional, and ignored by the OP's key selection, when the merchant
    /// has registered exactly one.
    #[must_use]
    pub fn with_kid(mut self, kid: impl Into<String>) -> Self {
        self.kid = Some(kid.into());
        self
    }

    /// The `client_id` this credential authenticates as `iss`/`sub` on every
    /// assertion it signs.
    #[must_use]
    pub(crate) fn client_id(&self) -> &str {
        &self.client_id
    }

    pub(crate) fn encoding_key(&self) -> &EncodingKey {
        &self.encoding_key
    }
}

impl Clone for Credentials {
    fn clone(&self) -> Self {
        Self {
            client_id: self.client_id.clone(),
            encoding_key: self.encoding_key.clone(),
            kid: self.kid.clone(),
        }
    }
}

impl fmt::Debug for Credentials {
    /// Hand-written so the private key can never reach a log line or a test
    /// failure message via `{:?}` — see `tests/debug_redaction.rs`, which
    /// fails if this is ever replaced with `#[derive(Debug)]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field("kid", &self.kid)
            .field("encoding_key", &"[redacted]")
            .finish()
    }
}

/// The claims RFC 7523 §3 requires a `private_key_jwt` assertion to carry.
/// Field order and names mirror `authkestra_engine::client_assertion::
/// AssertionClaims` exactly — see the module doc.
#[derive(Debug, Serialize)]
struct AssertionClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    jti: String,
    exp: i64,
    iat: i64,
}

/// Checks `lifetime` against `1..=300` seconds and returns it as whole
/// seconds. Shared by [`mint_client_assertion`] and
/// [`crate::ClientBuilder::build`] so the two can never disagree about the
/// bound.
pub(crate) fn validate_lifetime_secs(lifetime: Duration) -> Result<i64, ConfigError> {
    let secs = lifetime.as_secs();
    if !(MIN_ASSERTION_LIFETIME_SECS..=MAX_ASSERTION_LIFETIME_SECS).contains(&secs) {
        return Err(ConfigError::InvalidAssertionLifetime { lifetime });
    }
    // `secs` is bounded to at most 300 by the check above, so this cast
    // never truncates or changes sign.
    Ok(secs as i64)
}

fn now_unix_seconds() -> Result<i64, ConfigError> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ConfigError::SystemClockBeforeEpoch)?
        .as_secs();
    // Not reachable before the year 2262; no realistic deployment clock
    // exceeds `i64::MAX` seconds since the epoch.
    Ok(secs as i64)
}

/// Mints a fresh `private_key_jwt` client assertion authenticating
/// `credentials.client_id()` to `audience` (the token endpoint URL, per RFC
/// 7523 §3 — `docs/flows/merchant-auth.md`'s header table).
///
/// A fresh `jti` (UUIDv4) is generated on every call — reusing one would
/// hand a replay-tracking verifier (`authkestra_op::client_assertion::
/// ClientAssertionStore`) a second presentation of an id it already spent,
/// indistinguishable from an actual replay.
///
/// Exposed `pub` under [`crate::auth`] beyond what a [`crate::Client`]
/// needs internally: minting a bare assertion is legitimately useful to a
/// merchant debugging their own registration against vpay's OP directly.
///
/// # Errors
/// [`ConfigError::InvalidAssertionLifetime`] if `lifetime` is outside
/// `1..=300` seconds; [`ConfigError::SystemClockBeforeEpoch`] if the system
/// clock reads before 1970; [`ConfigError::Signing`] if `jsonwebtoken`
/// itself fails to produce a signature.
pub fn mint_client_assertion(
    credentials: &Credentials,
    audience: &str,
    lifetime: Duration,
) -> Result<String, ConfigError> {
    let lifetime_secs = validate_lifetime_secs(lifetime)?;
    let now = now_unix_seconds()?;

    let claims = AssertionClaims {
        iss: credentials.client_id(),
        sub: credentials.client_id(),
        aud: audience,
        jti: Uuid::new_v4().to_string(),
        exp: now + lifetime_secs,
        iat: now,
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = credentials.kid.clone();

    encode(&header, &claims, credentials.encoding_key())
        .map_err(|e| ConfigError::Signing(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RSA_PEM: &str = include_str!("../tests/fixtures/test_rsa_key_pkcs8.pem");

    #[test]
    fn rejects_a_lifetime_below_one_second() {
        let err = validate_lifetime_secs(Duration::from_millis(500));
        assert!(matches!(
            err,
            Err(ConfigError::InvalidAssertionLifetime { .. })
        ));
    }

    #[test]
    fn rejects_a_lifetime_above_three_hundred_seconds() {
        let err = validate_lifetime_secs(Duration::from_secs(301));
        assert!(matches!(
            err,
            Err(ConfigError::InvalidAssertionLifetime { .. })
        ));
    }

    #[test]
    fn accepts_the_boundary_values() {
        assert_eq!(validate_lifetime_secs(Duration::from_secs(1)).unwrap(), 1);
        assert_eq!(
            validate_lifetime_secs(Duration::from_secs(300)).unwrap(),
            300
        );
    }

    /// Decodes one dot-separated segment of a compact JWS as JSON.
    ///
    /// Index-free (`.nth`/`.get`) rather than `parts[n]`/`value["k"]`:
    /// `clippy::indexing_slicing` is on workspace-wide and this repository's
    /// house style answers it by not indexing, including in tests.
    fn decode_segment(jwt: &str, index: usize) -> serde_json::Value {
        use base64::Engine as _;
        let segment = jwt.split('.').nth(index).expect("jwt has three segments");
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(segment)
            .expect("jwt segments are base64url");
        serde_json::from_slice(&bytes).expect("jwt segments are JSON")
    }

    fn claim<'a>(value: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
        value.get(key).expect("claim is present")
    }

    #[test]
    fn mints_an_assertion_with_the_expected_claim_shape() {
        let creds = Credentials::rsa_pem("svc-1", TEST_RSA_PEM).unwrap();
        let jwt = mint_client_assertion(
            &creds,
            "https://vpay.example/v1/oauth/token",
            Duration::from_secs(60),
        )
        .unwrap();

        let header = decode_segment(&jwt, 0);
        let payload = decode_segment(&jwt, 1);

        assert_eq!(claim(&header, "alg"), "RS256");
        assert_eq!(claim(&header, "typ"), "JWT");
        assert_eq!(claim(&payload, "iss"), "svc-1");
        assert_eq!(claim(&payload, "sub"), "svc-1");
        assert_eq!(
            claim(&payload, "aud"),
            "https://vpay.example/v1/oauth/token"
        );
        assert!(claim(&payload, "jti").is_string());
        assert!(claim(&payload, "exp").is_i64());
        assert!(claim(&payload, "iat").is_i64());

        // The claim set is exactly `authkestra_engine`'s own — no more, no
        // less (see the module doc). A stray extra claim is not a wire error,
        // but it is a silent divergence from the minter this is a port of.
        let claims = payload.as_object().expect("payload is a JSON object");
        let mut names: Vec<&str> = claims.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["aud", "exp", "iat", "iss", "jti", "sub"]);
    }

    #[test]
    fn omits_kid_from_the_header_when_none_was_configured() {
        // `select_key` only tolerates a missing `kid` when the client has
        // exactly one registered key; emitting an empty or invented one
        // instead would fail against every multi-key registration.
        let creds = Credentials::rsa_pem("svc-1", TEST_RSA_PEM).unwrap();
        let jwt = mint_client_assertion(&creds, "aud", Duration::from_secs(60)).unwrap();
        assert!(decode_segment(&jwt, 0).get("kid").is_none());
    }

    #[test]
    fn stamps_a_configured_kid_onto_the_header() {
        let creds = Credentials::rsa_pem("svc-1", TEST_RSA_PEM)
            .unwrap()
            .with_kid("key-2");
        let jwt = mint_client_assertion(&creds, "aud", Duration::from_secs(60)).unwrap();
        assert_eq!(claim(&decode_segment(&jwt, 0), "kid"), "key-2");
    }

    #[test]
    fn exp_is_iat_plus_exactly_the_configured_lifetime() {
        let creds = Credentials::rsa_pem("svc-1", TEST_RSA_PEM).unwrap();
        let jwt = mint_client_assertion(&creds, "aud", Duration::from_secs(120)).unwrap();
        let payload = decode_segment(&jwt, 1);
        let iat = claim(&payload, "iat").as_i64().expect("iat is an integer");
        let exp = claim(&payload, "exp").as_i64().expect("exp is an integer");
        assert_eq!(exp - iat, 120);
    }

    #[test]
    fn mints_a_fresh_jti_on_every_call() {
        // Compares the decoded `jti`s, not the two compact JWSs: those differ
        // whenever *any* claim does — `iat` alone would do it across a second
        // boundary — so `assert_ne!(a, b)` would still pass with a hard-coded
        // `jti`. The `jti` is the claim the OP spends exactly once
        // (`ClientAssertionStore`); a repeat is indistinguishable from a
        // replay and would be refused.
        let creds = Credentials::rsa_pem("svc-1", TEST_RSA_PEM).unwrap();
        let a = mint_client_assertion(&creds, "aud", Duration::from_secs(60)).unwrap();
        let b = mint_client_assertion(&creds, "aud", Duration::from_secs(60)).unwrap();

        let jti_a = claim(&decode_segment(&a, 1), "jti")
            .as_str()
            .expect("jti is a string")
            .to_string();
        let jti_b = claim(&decode_segment(&b, 1), "jti")
            .as_str()
            .expect("jti is a string")
            .to_string();
        assert_ne!(jti_a, jti_b, "each assertion must carry its own jti");
        // And each is a UUIDv4, the form the wire contract names.
        for jti in [&jti_a, &jti_b] {
            let parsed = uuid::Uuid::parse_str(jti).expect("the jti is a UUID");
            assert_eq!(parsed.get_version_num(), 4);
        }
    }

    #[test]
    fn credentials_debug_output_never_contains_the_pem() {
        let creds = Credentials::rsa_pem("svc-1", TEST_RSA_PEM)
            .unwrap()
            .with_kid("key-1");
        let debug = format!("{creds:?}");
        assert!(!debug.contains("BEGIN"));
        assert!(!debug.contains("PRIVATE KEY"));
        assert!(debug.contains("svc-1"));
        assert!(debug.contains("key-1"));
    }
}
