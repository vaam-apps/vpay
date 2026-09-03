//! The `Vpay-Signature` header: the exact bytes vpay signs, and the exact text
//! it writes.
//!
//! `docs/flows/webhooks.md` names the scheme — Stripe's, copied — and the two
//! SDK verifiers (`sdks/rust/src/webhooks.rs`, `sdks/nodejs/src/webhooks.ts`)
//! are the specification this module is held to. This is the *sending* half of
//! that pair and the only place in the workspace that produces the header.
//!
//! `docs/reference/vpay-worker.md` §"Signing" carries the three properties
//! that make a delivery verifiable — the literal `t` text is what is signed,
//! the bytes signed are the bytes sent, and one `v1=` per configured secret —
//! and the two things this module deliberately does not do.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;

/// Builds the `Vpay-Signature` value for `body` at `now`, one `v1=` per
/// secret.
///
/// `t=<decimal unix seconds>,v1=<hex>[,v1=<hex>]`, lowercase hex, no spaces —
/// the grammar both SDK verifiers parse.
///
/// # An empty `secrets` produces a header with no signature in it
///
/// `t=…` alone is what comes back, and every verifier calls that a
/// *malformed header*. That is the honest answer and it is not this
/// function's to improve on: there is no such thing as an unsigned delivery a
/// receiver should accept, so a bare `t=` is exactly the refusal an endpoint
/// configured with no secret deserves. The caller is nonetheless expected not
/// to reach here — [`crate::webhooks::handle_deliver`] records a failed
/// attempt instead of sending, and boot-time validation is the real guard —
/// because a receiver refusing is a much worse diagnostic than a log line
/// naming the endpoint.
///
/// # A pre-epoch clock produces a header every verifier refuses
///
/// `now.unix_timestamp()` is signed, so a machine whose clock is before 1970
/// writes `t=-…`, which fails both verifiers' `^\d+$` rule as a malformed
/// header. Clamping it to zero would be worse: the delivery would then be
/// signed with a timestamp that is not the one the sender believes, and it
/// would fail the *tolerance* check instead — the same rejection, reported as
/// something the merchant could plausibly debug. A refused header that names
/// the real clock is the failure an operator can act on.
///
/// ```
/// use time::{Duration, OffsetDateTime};
/// use vpay_worker::signature_header;
///
/// let now = OffsetDateTime::UNIX_EPOCH + Duration::seconds(1_753_401_600);
/// let secrets = ["whsec_old".to_owned(), "whsec_new".to_owned()];
/// let header = signature_header(br#"{"id":"evt_1"}"#, now, &secrets);
///
/// let parts: Vec<&str> = header.split(',').collect();
/// // `t=` is plain decimal seconds, no padding and no sign, and it is that
/// // literal text the HMAC covers.
/// assert_eq!(parts[0], "t=1753401600");
/// // One `v1=` per configured secret, in configuration order — which is what
/// // lets a receiver holding *either* secret verify during a rotation.
/// assert_eq!(parts.len(), 3);
/// assert!(parts[1..].iter().all(|part| part.starts_with("v1=")));
/// assert_ne!(parts[1], parts[2]);
/// // SHA-256 is 32 bytes, rendered as 64 lowercase hex characters.
/// assert!(parts[1..].iter().all(|part| part.len() == 3 + 64));
/// assert!(header.chars().all(|c| c != ' '));
///
/// // No secret configured yields a header carrying no signature at all,
/// // which every verifier calls malformed. That is the honest answer.
/// assert_eq!(signature_header(b"{}", now, &[]), "t=1753401600");
/// ```
#[must_use]
pub fn signature_header(body: &[u8], now: OffsetDateTime, secrets: &[String]) -> String {
    // Written once and *reused* for the HMAC below. The whole scheme rests on
    // the header text and the signed text being the same characters, and the
    // cheapest way to guarantee that is to have only one of them exist.
    let timestamp_text = now.unix_timestamp().to_string();

    let mut header = format!("t={timestamp_text}");
    for secret in secrets {
        let signature = sign(&timestamp_text, body, secret);
        header.push_str(",v1=");
        header.push_str(&signature);
    }
    header
}

/// HMAC-SHA256 over `timestamp_text || "." || body`, lowercase hex.
///
/// `Hmac::new_from_slice` is infallible for HMAC (it accepts a key of any
/// length, hashing over-long ones), but its signature is fallible for the
/// `KeyInit` trait's sake. `unwrap` is denied in this crate, and an empty
/// signature would be a *silently* unverifiable delivery — so the error arm
/// returns an empty string, which cannot match any 32-byte HMAC and which
/// both verifiers drop as an empty `v1=` rather than treat as a candidate.
/// Unreachable in practice; the arm exists so the impossible case cannot
/// become a panic in a payment path.
fn sign(timestamp_text: &str, body: &[u8], secret: &str) -> String {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(timestamp_text.as_bytes());
    mac.update(b".");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use time::OffsetDateTime;
    use vpay_sdk::webhooks::{DEFAULT_TOLERANCE, verify_at};

    use super::signature_header;

    /// A body shaped like the one `crate::webhooks::event_bytes` produces:
    /// the SDK's `verify_at` decodes it as an `Event` after the signature
    /// checks out, so a parity test cannot use arbitrary bytes.
    const BODY: &[u8] = br#"{"id":"evt_1","object":"event","type":"payment_intent.succeeded","created":1753401600,"livemode":false,"data":{"object":{"id":"pi_1"}}}"#;

    const T: i64 = 1_753_401_600;

    fn at(unix: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(unix).expect("a fixed, valid timestamp")
    }

    fn system_time(unix: i64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(u64::try_from(unix).expect("non-negative in these tests"))
    }

    /// The whole contract in one assertion: what this crate emits, the SDK a
    /// merchant installs accepts. Not a re-implementation of the HMAC here —
    /// that would only prove two functions in one commit agree.
    #[test]
    fn one_secret_verifies_through_the_sdk() {
        let secrets = vec!["whsec_test_secret".to_owned()];
        let header = signature_header(BODY, at(T), &secrets);

        let event = verify_at(
            BODY,
            &header,
            "whsec_test_secret",
            DEFAULT_TOLERANCE,
            system_time(T),
        )
        .expect("the SDK must accept a header this crate produced");
        assert_eq!(event.id, "evt_1");
    }

    /// Rotation: two secrets, two `v1=` values, and a receiver holding
    /// *either* one verifies. If only the last secret were signed, the
    /// endpoint still on the old one would reject every delivery for the
    /// whole overlap — the failure rotation exists to avoid.
    #[test]
    fn each_of_two_secrets_is_independently_accepted() {
        let secrets = vec!["old-secret".to_owned(), "new-secret".to_owned()];
        let header = signature_header(BODY, at(T), &secrets);

        assert_eq!(
            header.matches("v1=").count(),
            2,
            "one v1= per configured secret: {header}"
        );

        for (index, secret) in secrets.iter().enumerate() {
            assert!(
                verify_at(BODY, &header, secret, DEFAULT_TOLERANCE, system_time(T)).is_ok(),
                "secrets[{index}] was configured but its signature is not in {header}"
            );
        }
        // And a secret that was never configured still does not verify, so
        // the test above is about *these* two values and not about the
        // verifier being lenient.
        assert!(
            verify_at(
                BODY,
                &header,
                "not-configured",
                DEFAULT_TOLERANCE,
                system_time(T)
            )
            .is_err()
        );
    }

    /// The bytes are the contract. A body altered after signing must not
    /// verify — this is the property the `payload_sha256` compare in
    /// `handle_deliver` exists to keep true across attempts.
    #[test]
    fn a_tampered_body_is_refused() {
        let secrets = vec!["whsec_test_secret".to_owned()];
        let header = signature_header(BODY, at(T), &secrets);

        let mut altered = BODY.to_vec();
        let last = altered.pop().expect("the body is not empty");
        altered.push(if last == b'}' { b')' } else { b'}' });

        assert!(
            verify_at(
                &altered,
                &header,
                "whsec_test_secret",
                DEFAULT_TOLERANCE,
                system_time(T)
            )
            .is_err(),
            "a one-byte change must break the signature"
        );
    }

    /// `t` is the literal decimal text, and it is that same text that was
    /// signed. Transcribed from the SDK's own vector
    /// (`the_hmac_covers_the_literal_t_text_not_a_re_rendered_number`): the
    /// verifier hashes the header's characters, so a sender that signed
    /// anything else — a zero-padded form, a re-parsed integer, a
    /// millisecond value — produces deliveries nobody can verify.
    #[test]
    fn the_t_written_is_the_t_signed() {
        let secrets = vec!["whsec_test_secret".to_owned()];
        let header = signature_header(BODY, at(T), &secrets);

        assert!(
            header.starts_with(&format!("t={T},v1=")),
            "plain decimal seconds, no padding and no sign: {header}"
        );
        // Seconds, not milliseconds: the SDK's tolerance is 300 *seconds*, so
        // a millisecond `t` would be ~55 000 years out and every delivery
        // would fail as out-of-tolerance rather than as a wrong signature.
        let t_text = header
            .split(',')
            .next()
            .and_then(|part| part.strip_prefix("t="))
            .expect("the header always opens with t=");
        assert_eq!(t_text, "1753401600");
        assert!(t_text.bytes().all(|b| b.is_ascii_digit()));

        // And the signature is over that text: verifying at exactly `T`
        // succeeds, which the previous test already showed, while a header
        // whose `t` is rewritten to an equal-but-differently-spelled value
        // does not.
        let repadded = header.replacen("t=1753401600", "t=01753401600", 1);
        assert!(
            verify_at(
                BODY,
                &repadded,
                "whsec_test_secret",
                DEFAULT_TOLERANCE,
                system_time(T)
            )
            .is_err(),
            "the HMAC covers the literal t text, so re-spelling t must break it"
        );
    }

    /// Hex is lowercase and full width. Both verifiers `hex::decode` the
    /// candidate, so uppercase would in fact still verify — but the header is
    /// also what an operator greps for in a receiver's access log, and one
    /// deployment emitting uppercase would make that search silently
    /// deployment-specific.
    #[test]
    fn every_signature_is_64_lowercase_hex_characters() {
        let secrets = vec!["a".to_owned(), "b".to_owned()];
        let header = signature_header(BODY, at(T), &secrets);

        let values: Vec<&str> = header
            .split(',')
            .filter_map(|part| part.strip_prefix("v1="))
            .collect();
        assert_eq!(values.len(), 2);
        for value in values {
            assert_eq!(value.len(), 64, "SHA-256 is 32 bytes");
            assert!(
                value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "lowercase hex only: {value}"
            );
        }
    }

    /// No secrets means no `v1=`, which is a malformed header rather than an
    /// accidentally-unsigned delivery. Pinned so that a future refactor
    /// cannot make "no secret configured" quietly mean "send it anyway".
    #[test]
    fn no_secrets_yields_a_header_carrying_no_signature() {
        let header = signature_header(BODY, at(T), &[]);
        assert_eq!(header, "t=1753401600");
        assert!(
            verify_at(BODY, &header, "any", DEFAULT_TOLERANCE, system_time(T)).is_err(),
            "a header with no v1= must never verify"
        );
    }
}
