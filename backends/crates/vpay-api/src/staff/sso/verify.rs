//! Verifying an id_token, and the three policy questions asked of it
//! ([ADR-0030](../../../../../../docs/adr/0030-staff-sign-in-federates-to-an-oidc-provider.md)).
//!
//! Pure functions over a token, a key and configuration — no network, no
//! database, no clock but the one passed in — so that every refusal has a
//! unit test that needs no container.
//!
//! # What is checked, in the order it is checked
//!
//! 1. **The algorithm**, from the unverified header, against an allow-list of
//!    asymmetric algorithms ([`ALLOWED_ALGORITHMS`]). `none` and every HMAC
//!    are not in it, and the list — not the token — decides: the header
//!    chooses nothing it was not already offered. This is the algorithm
//!    confusion defence (RFC 8725 §3.1); the *key* defends the other half,
//!    because `jsonwebtoken` refuses a verifying key of one family used with
//!    an algorithm of another.
//! 2. **A `kid`**, required. A token naming no key cannot be matched to one,
//!    and "try every key" is how a signature oracle starts.
//! 3. **The signature**, against the issuer's published key for that `kid`
//!    (resolved by the caller through [`crate::jwks_cache::KeySetCache`]),
//!    and the JWK's own `alg` and `use` if it declares them.
//! 4. **`iss`** equal to the configured issuer byte for byte; **`aud`**
//!    containing the client id; **`azp`**, when present, equal to it — and
//!    required when `aud` names more than one party (OIDC Core §2);
//!    **`exp`** not past, with `jsonwebtoken`'s default 60 s of leeway, which
//!    is the convention `crate::resource_auth` already lives under; **`iat`**
//!    present and not in the future beyond the same leeway; **`sub`** present,
//!    non-empty and within OIDC Core §2's 255 characters.
//!
//! The nonce, the group and the second factor are separate questions
//! ([`IdentityClaims::nonce_matches`], [`IdentityClaims::in_group`],
//! [`IdentityClaims::proves_second_factor`]) because the callback orders them
//! deliberately: ADR-0030 retries the provider's login only when a missing
//! second factor is the *sole* defect, so it cannot be folded into the token's
//! validity.

use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde_json::{Map, Value};
use subtle::ConstantTimeEq as _;

use super::error::SsoError;

/// The signing algorithms an id_token may use.
///
/// Asymmetric only. `none` is not an [`Algorithm`] variant at all, and the
/// three HMAC variants are absent on purpose: an HMAC "signature" verified
/// with a key an attacker can fetch (the issuer's public key) is no signature.
/// `EdDSA` is absent because nothing in this deployment's configuration has
/// asked for it and an allow-list is a list of things somebody decided.
pub(crate) const ALLOWED_ALGORITHMS: [Algorithm; 8] = [
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
];

/// How far past `exp`, or ahead of `iat`, a clock difference is forgiven.
///
/// `jsonwebtoken`'s own default, stated here because `iat` is checked by hand
/// and must agree with the `exp` check the library makes.
const LEEWAY_SECONDS: i64 = 60;

/// The longest `sub` OIDC Core §2 permits, and the width of
/// `credentials.subject`.
const MAX_SUBJECT_CHARS: usize = 255;

/// What the rest of the callback needs from a verified id_token.
///
/// Built only by [`verify_id_token`], so holding one is holding the claims of a
/// token whose signature and registered claims were checked.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct IdentityClaims {
    /// `sub`: the issuer's identifier for the person. The join key.
    pub(crate) subject: String,
    /// `nonce`, if the token carries one.
    nonce: Option<String>,
    /// `email`. **Display data**, never a join key and never a policy input.
    pub(crate) email: Option<String>,
    /// `name`, else `preferred_username`. Display data.
    pub(crate) display_name: Option<String>,
    /// The groups, from the configured claim.
    groups: Vec<String>,
    /// `acr`, if a string.
    acr: Option<String>,
    /// `amr`, if an array of strings.
    amr: Vec<String>,
}

/// Prints the subject and nothing a person typed or the provider chose to
/// say about them: the email and the name are personal data, and the nonce is
/// a credential until it is compared.
impl std::fmt::Debug for IdentityClaims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityClaims")
            .field("subject", &self.subject)
            .field("groups", &self.groups.len())
            .field("acr", &self.acr)
            .field("amr", &self.amr)
            .finish_non_exhaustive()
    }
}

impl IdentityClaims {
    /// Whether the token's nonce is the one this flow sent.
    ///
    /// `expected_hash` is the SHA-256 (hex) the flow stored; the token's nonce
    /// is hashed the same way and the two digests are compared in constant
    /// time. A token with no nonce never matches.
    pub(crate) fn nonce_matches(&self, expected_hash: &str) -> bool {
        let Some(nonce) = self.nonce.as_deref() else {
            return false;
        };
        let presented = crate::staff_auth::tokens::digest(nonce);
        presented.as_bytes().ct_eq(expected_hash.as_bytes()).into()
    }

    /// Whether `group` is among the token's groups, compared exactly.
    ///
    /// Exactly: no case folding, no path handling. A Keycloak mapper that
    /// emits full paths (`/vpay-staff`) is configured to emit the name, or the
    /// configuration names the path — a comparison that "helpfully" ignored a
    /// leading slash would also match `/a/vpay-staff`.
    pub(crate) fn in_group(&self, group: &str) -> bool {
        self.groups.iter().any(|candidate| candidate == group)
    }

    /// Whether the token proves a second factor under the configured rule:
    /// its `acr` is in `accepted_acr`, **or** its `amr` intersects
    /// `accepted_amr` (ADR-0019 § R1 for the `oidc` kind, taken by ADR-0030).
    ///
    /// Exact string comparison, and an empty list accepts nothing — so a
    /// configuration with both lists empty (which boot refuses) would refuse
    /// every token rather than accept every one.
    pub(crate) fn proves_second_factor(
        &self,
        accepted_acr: &[String],
        accepted_amr: &[String],
    ) -> bool {
        let acr_ok = self
            .acr
            .as_deref()
            .is_some_and(|acr| accepted_acr.iter().any(|accepted| accepted == acr));
        let amr_ok = self
            .amr
            .iter()
            .any(|method| accepted_amr.iter().any(|accepted| accepted == method));
        acr_ok || amr_ok
    }
}

impl IdentityClaims {
    /// The token's `acr` and `amr`, for the log line that explains a missing
    /// second factor.
    ///
    /// Safe to log: both are short registered values naming *how* the person
    /// authenticated, not who they are, and the line they appear on is the
    /// one an operator needs when a realm's `amr` mapping is not what
    /// `accepted_amr` assumes — which is the commonest way SSO is
    /// misconfigured (ADR-0030).
    pub(crate) fn authentication_methods(&self) -> (Option<&str>, &[String]) {
        (self.acr.as_deref(), &self.amr)
    }
}

/// What the unverified header says, after the allow-list has spoken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcceptedHeader {
    /// The algorithm, which is in [`ALLOWED_ALGORITHMS`].
    pub(crate) algorithm: Algorithm,
    /// The `kid`.
    pub(crate) kid: String,
}

/// Reads the unverified header and applies the algorithm allow-list and the
/// `kid` requirement.
///
/// # Errors
///
/// [`SsoError::Header`] for a header that does not decode, an algorithm
/// outside [`ALLOWED_ALGORITHMS`], or no `kid`.
pub(crate) fn accept_header(token: &str) -> Result<AcceptedHeader, SsoError> {
    let header = decode_header(token).map_err(|_| SsoError::Header("it does not decode"))?;
    if !ALLOWED_ALGORITHMS.contains(&header.alg) {
        return Err(SsoError::Header("its algorithm is not in the allow-list"));
    }
    let kid = header
        .kid
        .filter(|kid| !kid.is_empty())
        .ok_or(SsoError::Header("it names no kid"))?;
    Ok(AcceptedHeader {
        algorithm: header.alg,
        kid,
    })
}

/// What the callback holds a token to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Expectations<'a> {
    /// `staff_auth.sso.issuer`, compared byte for byte.
    pub(crate) issuer: &'a str,
    /// `staff_auth.sso.client_id`.
    pub(crate) client_id: &'a str,
    /// `staff_auth.sso.groups_claim`.
    pub(crate) groups_claim: &'a str,
    /// Unix seconds. A parameter so a test can hold the clock still.
    pub(crate) now: i64,
}

/// Verifies `token` against `key` and extracts the claims the callback needs.
///
/// `header` is [`accept_header`]'s answer for the same token, so the algorithm
/// verified with is the one the allow-list approved.
///
/// # Errors
///
/// [`SsoError::Token`] for a signature that does not verify, a key that cannot
/// verify the token's algorithm, and every registered-claim failure listed in
/// this module's header.
pub(crate) fn verify_id_token(
    token: &str,
    header: &AcceptedHeader,
    key: &Jwk,
    expect: Expectations<'_>,
) -> Result<IdentityClaims, SsoError> {
    // A key declared for encryption, or for another algorithm, is not a
    // candidate however well the signature happens to verify.
    if key
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|usage| !matches!(usage, jsonwebtoken::jwk::PublicKeyUse::Signature))
    {
        return Err(SsoError::Token("the key is not declared for signatures"));
    }
    if let Some(declared) = key.common.key_algorithm {
        let declared = serde_json::to_value(declared).ok();
        let used = serde_json::to_value(header.algorithm).ok();
        if declared.is_none() || declared != used {
            return Err(SsoError::Token("the key is declared for another algorithm"));
        }
    }

    let decoding_key =
        DecodingKey::from_jwk(key).map_err(|_| SsoError::Token("the key cannot verify"))?;

    // The algorithm list is the ONE the header named and the allow-list
    // approved — never `ALLOWED_ALGORITHMS` wholesale, so a token cannot be
    // verified under an algorithm other than the one it announced.
    let mut validation = Validation::new(header.algorithm);
    validation.set_issuer(&[expect.issuer]);
    validation.set_audience(&[expect.client_id]);
    // `aud`/`iss`/`exp` are only checked by the library when present, so their
    // presence is demanded separately (`resource_auth`'s "sharp edge").
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.leeway = u64::try_from(LEEWAY_SECONDS).unwrap_or(60);

    let claims = decode::<Map<String, Value>>(token, &decoding_key, &validation)
        .map_err(|error| SsoError::Token(describe(&error)))?
        .claims;

    check_registered_claims(&claims, expect)?;

    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|sub| !sub.is_empty() && sub.chars().count() <= MAX_SUBJECT_CHARS)
        .ok_or(SsoError::Token(
            "sub is absent, empty or over 255 characters",
        ))?
        .to_owned();

    Ok(IdentityClaims {
        subject,
        nonce: claims
            .get("nonce")
            .and_then(Value::as_str)
            .map(str::to_owned),
        email: claims
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_owned),
        display_name: claims
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| claims.get("preferred_username").and_then(Value::as_str))
            .map(str::to_owned),
        groups: strings(claims.get(expect.groups_claim)),
        acr: claims.get("acr").and_then(Value::as_str).map(str::to_owned),
        amr: strings(claims.get("amr")),
    })
}

/// The claim checks `jsonwebtoken` does not make: `azp` and `iat`.
fn check_registered_claims(
    claims: &Map<String, Value>,
    expect: Expectations<'_>,
) -> Result<(), SsoError> {
    // OIDC Core §2: `azp` MUST be the client id when present, and a token for
    // several audiences must name which party it was issued to.
    let audiences = match claims.get("aud") {
        Some(Value::Array(list)) => list.len(),
        Some(Value::String(_)) => 1,
        _ => 0,
    };
    match claims.get("azp") {
        Some(Value::String(azp)) if azp == expect.client_id => {}
        Some(_) => return Err(SsoError::Token("azp is not this client")),
        None if audiences > 1 => {
            return Err(SsoError::Token("several audiences and no azp"));
        }
        None => {}
    }

    // `iat` is REQUIRED by OIDC Core §2 and ignored by `jsonwebtoken`. A token
    // "issued in the future" past the leeway is one whose clock or whose
    // issuer is wrong, and refusing it costs nothing.
    let issued_at = claims
        .get("iat")
        .and_then(Value::as_i64)
        .ok_or(SsoError::Token("iat is absent"))?;
    if issued_at > expect.now.saturating_add(LEEWAY_SECONDS) {
        return Err(SsoError::Token("iat is in the future"));
    }
    Ok(())
}

/// A claim as a list of strings: an array's string elements, or one string.
fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(one)) => vec![one.clone()],
        _ => Vec::new(),
    }
}

/// A static reason for the log, from `jsonwebtoken`'s error kind.
///
/// By kind and never by `Display`: the library's text can embed the offending
/// claim value, and a claim value is the provider's or the caller's.
fn describe(error: &jsonwebtoken::errors::Error) -> &'static str {
    use jsonwebtoken::errors::ErrorKind;
    match error.kind() {
        ErrorKind::InvalidSignature => "the signature does not verify",
        ErrorKind::ExpiredSignature => "the token has expired",
        ErrorKind::InvalidIssuer => "iss is not the configured issuer",
        ErrorKind::InvalidAudience => "aud does not contain this client",
        ErrorKind::MissingRequiredClaim(_) => "a required claim is absent",
        ErrorKind::InvalidAlgorithm | ErrorKind::InvalidAlgorithmName => {
            "the algorithm does not match the key"
        }
        ErrorKind::ImmatureSignature => "the token is not yet valid",
        _ => "the token is malformed",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use jsonwebtoken::{EncodingKey, Header};
    use rsa::pkcs1::EncodeRsaPrivateKey as _;
    use rsa::traits::PublicKeyParts as _;
    use serde_json::json;

    const ISSUER: &str = "https://idp.example.test/realms/staff";
    const CLIENT: &str = "vpay-staff";

    struct Rsa {
        encoding: EncodingKey,
        jwk: Jwk,
    }

    fn rsa(kid: &str) -> Rsa {
        let private = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).expect("keygen");
        let pem = private
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .expect("pem")
            .to_string();
        let public = private.to_public_key();
        let jwk: Jwk = serde_json::from_value(json!({
            "kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid,
            "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
        }))
        .expect("jwk");
        Rsa {
            encoding: EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key"),
            jwk,
        }
    }

    /// Sets one claim. Not `claims["k"] = v`: `clippy::indexing_slicing` is denied,
    /// and `Value`'s `IndexMut` panics on a type mismatch.
    fn put(claims: &mut Value, key: &str, value: Value) {
        if let Some(object) = claims.as_object_mut() {
            object.insert(key.to_owned(), value);
        }
    }

    /// A token that verifies, minted around the real now: the `exp` check
    /// reads the system clock, so a fixed instant would rot.
    fn live_claims() -> Value {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        json!({
            "iss": ISSUER, "aud": CLIENT, "azp": CLIENT, "sub": "user-1",
            "exp": now + 300, "iat": now - 5, "nonce": "n-1",
            "email": "ada@example.test", "name": "Ada",
            "groups": ["vpay-staff", "other"], "acr": "1", "amr": ["pwd", "otp"],
        })
    }

    fn sign(key: &Rsa, alg: Algorithm, kid: Option<&str>, claims: &Value) -> String {
        let mut header = Header::new(alg);
        header.kid = kid.map(str::to_owned);
        jsonwebtoken::encode(&header, claims, &key.encoding).expect("encode")
    }

    fn check_live(key: &Rsa, token: &str) -> Result<IdentityClaims, SsoError> {
        let header = accept_header(token)?;
        verify_id_token(
            token,
            &header,
            &key.jwk,
            Expectations {
                issuer: ISSUER,
                client_id: CLIENT,
                groups_claim: "groups",
                now: time::OffsetDateTime::now_utc().unix_timestamp(),
            },
        )
    }

    #[test]
    fn a_good_token_verifies_and_yields_its_claims() {
        let key = rsa("k1");
        let token = sign(&key, Algorithm::RS256, Some("k1"), &live_claims());
        let claims = check_live(&key, &token).expect("a good token verifies");

        assert_eq!(claims.subject, "user-1");
        assert!(claims.in_group("vpay-staff"));
        assert!(!claims.in_group("vpay"), "exact comparison");
        assert_eq!(claims.email.as_deref(), Some("ada@example.test"));
        assert_eq!(claims.display_name.as_deref(), Some("Ada"));
    }

    #[test]
    fn the_wrong_issuer_audience_azp_or_a_missing_azp_with_two_audiences_is_refused() {
        let key = rsa("k1");
        type Mutation = fn(&mut Value);
        let cases: [(&str, Mutation); 5] = [
            ("iss", |c| {
                put(c, "iss", json!("https://evil.example.test/realms/staff"))
            }),
            ("iss trailing slash", |c| {
                put(c, "iss", json!(format!("{ISSUER}/")))
            }),
            ("aud", |c| put(c, "aud", json!("another-client"))),
            ("azp", |c| put(c, "azp", json!("another-client"))),
            ("two audiences, no azp", |c| {
                put(c, "aud", json!([CLIENT, "account"]));
                c.as_object_mut().unwrap().remove("azp");
            }),
        ];
        for (name, mutate) in cases {
            let mut claims = live_claims();
            mutate(&mut claims);
            let token = sign(&key, Algorithm::RS256, Some("k1"), &claims);
            assert!(
                matches!(check_live(&key, &token), Err(SsoError::Token(_))),
                "{name} must be refused"
            );
        }
    }

    #[test]
    fn an_audience_list_that_contains_the_client_with_an_azp_is_accepted() {
        let key = rsa("k1");
        let mut claims = live_claims();
        put(&mut claims, "aud", json!([CLIENT, "account"]));
        let token = sign(&key, Algorithm::RS256, Some("k1"), &claims);
        check_live(&key, &token).expect("aud may be a list when azp names this client");
    }

    #[test]
    fn expiry_is_enforced_with_sixty_seconds_of_leeway_and_iat_may_not_be_in_the_future() {
        let key = rsa("k1");
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        let mut inside = live_claims();
        put(&mut inside, "exp", json!(now - 30));
        let token = sign(&key, Algorithm::RS256, Some("k1"), &inside);
        check_live(&key, &token).expect("30 s past exp is inside the 60 s leeway");

        let mut expired = live_claims();
        put(&mut expired, "exp", json!(now - 120));
        let token = sign(&key, Algorithm::RS256, Some("k1"), &expired);
        assert_eq!(
            check_live(&key, &token),
            Err(SsoError::Token("the token has expired"))
        );

        let mut future = live_claims();
        put(&mut future, "iat", json!(now + 600));
        let token = sign(&key, Algorithm::RS256, Some("k1"), &future);
        assert_eq!(
            check_live(&key, &token),
            Err(SsoError::Token("iat is in the future"))
        );

        let mut no_iat = live_claims();
        no_iat.as_object_mut().unwrap().remove("iat");
        let token = sign(&key, Algorithm::RS256, Some("k1"), &no_iat);
        assert_eq!(
            check_live(&key, &token),
            Err(SsoError::Token("iat is absent"))
        );
    }

    #[test]
    fn a_missing_kid_is_refused_before_any_key_is_chosen() {
        let key = rsa("k1");
        let token = sign(&key, Algorithm::RS256, None, &live_claims());
        assert_eq!(
            accept_header(&token),
            Err(SsoError::Header("it names no kid"))
        );
    }

    #[test]
    fn hmac_and_none_are_outside_the_allow_list() {
        // An HS256 token signed with a secret: the allow-list refuses it from
        // the header alone, before any key is looked at.
        let hs = jsonwebtoken::encode(
            &{
                let mut header = Header::new(Algorithm::HS256);
                header.kid = Some("k1".to_owned());
                header
            },
            &live_claims(),
            &EncodingKey::from_secret(b"secret"),
        )
        .expect("encode");
        assert_eq!(
            accept_header(&hs),
            Err(SsoError::Header("its algorithm is not in the allow-list"))
        );

        // `alg: none`, hand-built: jsonwebtoken cannot even name it, so the
        // header does not decode, which is a refusal too.
        let none = format!(
            "{}.{}.",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"none","kid":"k1"}"#),
            URL_SAFE_NO_PAD.encode(live_claims().to_string())
        );
        assert!(matches!(accept_header(&none), Err(SsoError::Header(_))));
    }

    #[test]
    fn a_signature_from_another_key_is_refused() {
        let signer = rsa("k1");
        let other = rsa("k1");
        let token = sign(&signer, Algorithm::RS256, Some("k1"), &live_claims());
        assert_eq!(
            check_live(&other, &token),
            Err(SsoError::Token("the signature does not verify"))
        );
    }

    #[test]
    fn a_key_declared_for_another_algorithm_or_for_encryption_is_not_a_candidate() {
        let mut key = rsa("k1");
        let token = sign(&key, Algorithm::RS256, Some("k1"), &live_claims());

        key.jwk = serde_json::from_value(json!({
            "kty": "RSA", "use": "sig", "alg": "RS512", "kid": "k1",
            "n": "AQAB", "e": "AQAB",
        }))
        .expect("jwk");
        assert_eq!(
            check_live(&key, &token),
            Err(SsoError::Token("the key is declared for another algorithm"))
        );

        key.jwk = serde_json::from_value(json!({
            "kty": "RSA", "use": "enc", "kid": "k1", "n": "AQAB", "e": "AQAB",
        }))
        .expect("jwk");
        assert_eq!(
            check_live(&key, &token),
            Err(SsoError::Token("the key is not declared for signatures"))
        );
    }

    #[test]
    fn the_nonce_is_compared_by_hash_and_a_missing_nonce_never_matches() {
        let key = rsa("k1");
        let token = sign(&key, Algorithm::RS256, Some("k1"), &live_claims());
        let claims = check_live(&key, &token).expect("verifies");

        assert!(claims.nonce_matches(&crate::staff_auth::tokens::digest("n-1")));
        assert!(!claims.nonce_matches(&crate::staff_auth::tokens::digest("n-2")));
        assert!(!claims.nonce_matches(""));

        let mut without = live_claims();
        without.as_object_mut().unwrap().remove("nonce");
        let token = sign(&key, Algorithm::RS256, Some("k1"), &without);
        let claims = check_live(&key, &token).expect("verifies");
        assert!(!claims.nonce_matches(&crate::staff_auth::tokens::digest("")));
    }

    /// ADR-0019 § R1 for the `oidc` kind, and the measured Keycloak shapes:
    /// `acr "1"` with `amr ["pwd"]` is a password and nothing else.
    #[test]
    fn the_second_factor_rule_is_acr_or_amr_and_a_password_alone_is_not_one() {
        let key = rsa("k1");
        let claims_with = |acr: Value, amr: Value| {
            let mut claims = live_claims();
            put(&mut claims, "acr", acr);
            put(&mut claims, "amr", amr);
            let token = sign(&key, Algorithm::RS256, Some("k1"), &claims);
            check_live(&key, &token).expect("verifies")
        };
        let none: Vec<String> = Vec::new();
        let otp = vec!["otp".to_owned()];
        let mfa = vec!["mfa".to_owned()];

        // The measured production shape: accepted_amr = ["otp"], acr list empty.
        assert!(!claims_with(json!("1"), json!(["pwd"])).proves_second_factor(&none, &otp));
        assert!(!claims_with(json!("1"), json!([])).proves_second_factor(&none, &otp));
        assert!(claims_with(json!("0"), json!(["pwd", "otp"])).proves_second_factor(&none, &otp));
        // acr alone suffices when it is the configured rule.
        assert!(claims_with(json!("mfa"), json!([])).proves_second_factor(&mfa, &none));
        // Neither list configured accepts nothing, rather than everything.
        assert!(!claims_with(json!("mfa"), json!(["otp"])).proves_second_factor(&none, &none));
        // No claims at all.
        let mut bare = live_claims();
        let object = bare.as_object_mut().unwrap();
        object.remove("acr");
        object.remove("amr");
        let token = sign(&key, Algorithm::RS256, Some("k1"), &bare);
        let claims = check_live(&key, &token).expect("verifies");
        assert!(!claims.proves_second_factor(&mfa, &otp));
    }

    #[test]
    fn groups_come_from_the_configured_claim_as_a_list_or_one_string() {
        let key = rsa("k1");
        let mut one = live_claims();
        put(&mut one, "roles", json!("vpay-staff"));
        let token = sign(&key, Algorithm::RS256, Some("k1"), &one);
        let header = accept_header(&token).expect("header");
        let claims = verify_id_token(
            &token,
            &header,
            &key.jwk,
            Expectations {
                issuer: ISSUER,
                client_id: CLIENT,
                groups_claim: "roles",
                now: time::OffsetDateTime::now_utc().unix_timestamp(),
            },
        )
        .expect("verifies");
        assert!(claims.in_group("vpay-staff"));
        assert!(!claims.in_group("other"), "the default claim is not read");
    }

    #[test]
    fn the_debug_of_the_claims_prints_no_personal_data() {
        let key = rsa("k1");
        let token = sign(&key, Algorithm::RS256, Some("k1"), &live_claims());
        let claims = check_live(&key, &token).expect("verifies");
        let printed = format!("{claims:?}");
        assert!(!printed.contains("ada@example.test"), "{printed}");
        assert!(!printed.contains("n-1"), "{printed}");
    }

    /// ES256 verifies too: the JWK is an EC key (`crv`, `x`, `y`), which is
    /// why the issuer's key set is `jsonwebtoken`'s own `Jwk` and not
    /// authkestra's RSA-and-Ed25519 one.
    #[test]
    fn an_es256_token_verifies_against_an_ec_jwk() {
        use p256::ecdsa::SigningKey;
        use p256::pkcs8::EncodePrivateKey as _;

        let signing = SigningKey::random(&mut rand::rngs::OsRng);
        let pem = signing
            .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
            .expect("pkcs8 pem")
            .to_string();
        let point = signing.verifying_key().to_encoded_point(false);
        let jwk: Jwk = serde_json::from_value(json!({
            "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": "ec1",
            "x": URL_SAFE_NO_PAD.encode(point.x().expect("x")),
            "y": URL_SAFE_NO_PAD.encode(point.y().expect("y")),
        }))
        .expect("ec jwk");

        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("ec1".to_owned());
        let token = jsonwebtoken::encode(
            &header,
            &live_claims(),
            &EncodingKey::from_ec_pem(pem.as_bytes()).expect("ec encoding key"),
        )
        .expect("encode");

        let accepted = accept_header(&token).expect("ES256 is in the allow-list");
        assert_eq!(accepted.algorithm, Algorithm::ES256);
        let claims = verify_id_token(
            &token,
            &accepted,
            &jwk,
            Expectations {
                issuer: ISSUER,
                client_id: CLIENT,
                groups_claim: "groups",
                now: time::OffsetDateTime::now_utc().unix_timestamp(),
            },
        )
        .expect("an ES256 token verifies against its EC key");
        assert_eq!(claims.subject, "user-1");

        // And an RSA key cannot verify it: the key family must match.
        let rsa_key = rsa("ec1");
        assert!(matches!(
            verify_id_token(
                &token,
                &accepted,
                &rsa_key.jwk,
                Expectations {
                    issuer: ISSUER,
                    client_id: CLIENT,
                    groups_claim: "groups",
                    now: time::OffsetDateTime::now_utc().unix_timestamp(),
                },
            ),
            Err(SsoError::Token(_))
        ));
    }

    #[test]
    fn an_over_long_or_empty_subject_is_refused() {
        let key = rsa("k1");
        for sub in [String::new(), "x".repeat(256)] {
            let mut claims = live_claims();
            put(&mut claims, "sub", json!(sub));
            let token = sign(&key, Algorithm::RS256, Some("k1"), &claims);
            assert!(
                matches!(check_live(&key, &token), Err(SsoError::Token(_))),
                "a sub of {} chars",
                sub.len()
            );
        }
    }
}
