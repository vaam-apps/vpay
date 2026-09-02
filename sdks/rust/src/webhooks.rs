//! Outbound-webhook signature verification, per `docs/flows/webhooks.md`.
//!
//! `Vpay-Signature: t=<unix seconds>,v1=<hex hmac>[,v1=<hex hmac>...]`. The
//! signed payload is the literal bytes `"{t}.{raw_body}"` — the **raw**
//! request body, never a parsed-and-reserialised one, HMAC-SHA256 with the
//! endpoint secret, hex-encoded, compared in constant time. More than one
//! `v1=` may be present during a secret rotation; any one matching is
//! enough.
//!
//! # Parity with the Node verifier
//!
//! This is held to the same header grammar as `sdks/nodejs/src/webhooks.ts`,
//! because a delivery either SDK accepts and the other rejects is a defect
//! nobody could see from one side alone:
//!
//! | Input | Both SDKs |
//! |---|---|
//! | `t` | must match `^[0-9]+$`; anything else is [`WebhookError::MalformedHeader`] |
//! | the signed `t` | the **literal header text**, not a re-rendered number — `t=017…` hashes `"017…"` |
//! | a part with no `=` | ignored, so a future scheme element cannot break today's verifier |
//! | an unknown `k=v` | ignored, same reason |
//! | an empty `v1=` | not a candidate signature; a header carrying only one is malformed |

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::WebhookError;
use crate::model::Event;

type HmacSha256 = Hmac<Sha256>;

/// Default tolerance (`docs/flows/webhooks.md`: "reject a timestamp older
/// than 5 minutes").
pub const DEFAULT_TOLERANCE: Duration = Duration::from_secs(300);

struct ParsedHeader<'a> {
    /// The literal `t` text from the header — the bytes that were signed.
    ///
    /// Kept as text, never re-rendered from [`ParsedHeader::timestamp`]: the
    /// number is a lossy re-rendering of what the sender actually signed. A
    /// `t=01753401600` signed over that literal would fail against an HMAC
    /// computed over `"1753401600"`, rejecting a genuine delivery for a
    /// reason invisible from either end. `sdks/nodejs/src/webhooks.ts` says
    /// the same thing at the same place.
    timestamp_text: &'a str,
    /// The same value as an integer, used **only** for the tolerance
    /// comparison.
    timestamp: i64,
    signatures: Vec<&'a str>,
}

/// The only `t` this verifier accepts: one or more decimal digits, nothing
/// else — byte-for-byte the rule `sdks/nodejs/src/webhooks.ts`'s
/// `TIMESTAMP_PATTERN` (`/^\d+$/`) enforces.
///
/// `str::parse::<i64>` alone is not that rule: it accepts a leading `+` or
/// `-`, so `t=+1753401600` and `t=-1` would parse, and the payload signed
/// would then be `"+1753401600.<body>"` while this verifier hashed
/// `"1753401600.<body>"` — a signature computed over bytes the sender never
/// sent. Rejecting the header outright is the only answer that cannot be
/// confused with a signature mismatch.
fn is_ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

fn parse_header(header: &str) -> Result<ParsedHeader<'_>, WebhookError> {
    let mut timestamp_text = None;
    let mut signatures = Vec::new();

    for part in header.split(',') {
        // A part with no `=` is skipped, not fatal. The scheme is
        // `t=…,v1=…` today and this verifier must stay readable by a future
        // sender that adds a bare flag or a `v2=` element beside them — the
        // same forward-compatibility `webhooks.ts` implements by
        // `continue`-ing on `indexOf("=") === -1`. (The previous version of
        // this function claimed that property in a comment while
        // hard-failing on exactly this input.)
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key.trim() {
            "t" => timestamp_text = Some(value.trim()),
            // An empty `v1=` is not a candidate: `hex::decode("")` succeeds
            // and yields an empty `Vec`, which would then be compared
            // against a 32-byte HMAC. That comparison can only ever fail, so
            // dropping the value here — as `webhooks.ts` does with its
            // `value.length > 0` guard — makes a header carrying *only*
            // `v1=` a malformed header rather than a signature mismatch.
            "v1" => {
                let value = value.trim();
                if !value.is_empty() {
                    signatures.push(value);
                }
            }
            _ => {} // Forward-compatible with a future scheme version.
        }
    }

    let timestamp_text = timestamp_text.ok_or(WebhookError::MalformedHeader)?;
    if !is_ascii_digits(timestamp_text) {
        return Err(WebhookError::MalformedHeader);
    }
    if signatures.is_empty() {
        return Err(WebhookError::MalformedHeader);
    }
    // All-digits but wider than `i64` (21+ digits). Not malformed — the
    // header is well formed, the instant it names is simply nowhere near
    // now — so it is reported as the tolerance failure it is, which is also
    // what `webhooks.ts` produces for the same input (`Number()` keeps it
    // finite, and the tolerance check then rejects it).
    let timestamp = timestamp_text
        .parse::<i64>()
        .map_err(|_| WebhookError::TimestampOutOfTolerance)?;

    Ok(ParsedHeader {
        timestamp_text,
        timestamp,
        signatures,
    })
}

/// Constant-time comparison of two hex-encoded HMACs.
///
/// Compares decoded bytes, not the hex strings themselves — a length
/// mismatch after decoding still short-circuits `subtle`'s `ct_eq`, but a
/// same-length mismatch takes the same time regardless of *where* the bytes
/// differ, which is the property that matters here.
fn hex_signatures_match(candidate: &str, expected: &[u8]) -> bool {
    let Ok(candidate_bytes) = hex::decode(candidate) else {
        return false;
    };
    if candidate_bytes.len() != expected.len() {
        return false;
    }
    candidate_bytes.ct_eq(expected).into()
}

/// Verifies `signature_header` over `raw_body` with `secret`, using the
/// current system time, then decodes the body as an [`Event`].
///
/// # Errors
/// [`crate::Error::Webhook`] — see [`WebhookError`] for each rejection
/// reason.
pub fn verify(
    raw_body: &[u8],
    signature_header: &str,
    secret: &str,
    tolerance: Duration,
) -> Result<Event, crate::Error> {
    verify_at(
        raw_body,
        signature_header,
        secret,
        tolerance,
        SystemTime::now(),
    )
}

/// As [`verify`], but with an injectable clock — for deterministic tests
/// that do not want to race a real 5-minute tolerance window.
///
/// # Errors
/// See [`verify`].
pub fn verify_at(
    raw_body: &[u8],
    signature_header: &str,
    secret: &str,
    tolerance: Duration,
    now: SystemTime,
) -> Result<Event, crate::Error> {
    let parsed = parse_header(signature_header)?;

    let now_unix = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| WebhookError::MalformedHeader)?
        .as_secs() as i64;
    // `t` came off an unauthenticated header at this point in the function —
    // checked arithmetic throughout, so a crafted extreme value fails the
    // tolerance check (or, for the one unrepresentable case, is treated as
    // failing it) rather than wrapping or panicking.
    let delta = match now_unix
        .checked_sub(parsed.timestamp)
        .and_then(i64::checked_abs)
    {
        Some(d) => d,
        None => return Err(crate::Error::Webhook(WebhookError::TimestampOutOfTolerance)),
    };
    // `tolerance.as_secs()` fits in `i64` for any tolerance a caller would
    // realistically configure (up to ~292 billion years); this SDK does not
    // attempt to guard against a deliberately absurd `Duration`.
    if delta > tolerance.as_secs() as i64 {
        return Err(crate::Error::Webhook(WebhookError::TimestampOutOfTolerance));
    }

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| WebhookError::SignatureMismatch)?;
    mac.update(parsed.timestamp_text.as_bytes());
    mac.update(b".");
    mac.update(raw_body);
    let expected = mac.finalize().into_bytes();

    let matched = parsed
        .signatures
        .iter()
        .any(|sig| hex_signatures_match(sig, &expected));
    if !matched {
        return Err(crate::Error::Webhook(WebhookError::SignatureMismatch));
    }

    serde_json::from_slice(raw_body)
        .map_err(|e| crate::Error::Webhook(WebhookError::InvalidBody(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "whsec_test_secret";

    fn sample_body() -> Vec<u8> {
        br#"{"id":"evt_1","object":"event","type":"payment_intent.succeeded","created":1753401600,"livemode":false,"data":{"object":{"id":"pi_1"}}}"#.to_vec()
    }

    /// Signs over the **literal** `t` text, exactly as a sender that wrote
    /// that text into the header would have.
    fn sign_text(timestamp_text: &str, body: &[u8], secret: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(timestamp_text.as_bytes());
        mac.update(b".");
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    fn sign(timestamp: i64, body: &[u8], secret: &str) -> String {
        sign_text(&timestamp.to_string(), body, secret)
    }

    #[test]
    fn a_validly_signed_payload_is_accepted() {
        let body = sample_body();
        let t = 1_753_401_600;
        let sig = sign(t, &body, SECRET);
        let header = format!("t={t},v1={sig}");
        let now = UNIX_EPOCH + Duration::from_secs(t as u64 + 10);

        let event = verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).unwrap();
        assert_eq!(event.id, "evt_1");
    }

    #[test]
    fn a_signature_from_the_wrong_secret_is_rejected() {
        let body = sample_body();
        let t = 1_753_401_600;
        let sig = sign(t, &body, "a-different-secret");
        let header = format!("t={t},v1={sig}");
        let now = UNIX_EPOCH + Duration::from_secs(t as u64);

        let err = verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).unwrap_err();
        assert!(matches!(
            err,
            crate::Error::Webhook(WebhookError::SignatureMismatch)
        ));
    }

    #[test]
    fn a_timestamp_outside_tolerance_is_rejected() {
        let body = sample_body();
        let t = 1_753_401_600;
        let sig = sign(t, &body, SECRET);
        let header = format!("t={t},v1={sig}");
        // 301 seconds later, one past the 300s default tolerance.
        let now = UNIX_EPOCH + Duration::from_secs(t as u64 + 301);

        let err = verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).unwrap_err();
        assert!(matches!(
            err,
            crate::Error::Webhook(WebhookError::TimestampOutOfTolerance)
        ));
    }

    #[test]
    fn a_second_v1_value_matching_is_accepted_during_a_secret_rotation() {
        let body = sample_body();
        let t = 1_753_401_600;
        let old_sig = sign(t, &body, "old-secret");
        let new_sig = sign(t, &body, SECRET);
        let header = format!("t={t},v1={old_sig},v1={new_sig}");
        let now = UNIX_EPOCH + Duration::from_secs(t as u64);

        assert!(verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).is_ok());
    }

    #[test]
    fn a_malformed_header_is_rejected() {
        let body = sample_body();
        let now = SystemTime::now();
        for header in ["", "t=notanumber,v1=abc", "v1=abc", "t=123"] {
            let err = verify_at(&body, header, SECRET, DEFAULT_TOLERANCE, now).unwrap_err();
            assert!(
                matches!(err, crate::Error::Webhook(WebhookError::MalformedHeader)),
                "header {header:?} should be malformed, got {err:?}"
            );
        }
    }

    #[test]
    fn a_body_altered_by_one_byte_is_rejected() {
        let body = sample_body();
        let t = 1_753_401_600;
        let sig = sign(t, &body, SECRET);
        let header = format!("t={t},v1={sig}");
        let now = UNIX_EPOCH + Duration::from_secs(t as u64);

        // Index-free by house style (`clippy::indexing_slicing`): pop the
        // last byte and push a different one back.
        let mut altered = body.clone();
        let last = altered.pop().expect("body is not empty");
        altered.push(if last == b'}' { b')' } else { b'}' });

        let err = verify_at(&altered, &header, SECRET, DEFAULT_TOLERANCE, now).unwrap_err();
        assert!(matches!(
            err,
            crate::Error::Webhook(WebhookError::SignatureMismatch)
        ));
    }

    #[test]
    fn a_verified_but_undecodable_body_is_a_distinct_error_from_a_bad_signature() {
        let body = b"not an event".to_vec();
        let t = 1_753_401_600;
        let sig = sign(t, &body, SECRET);
        let header = format!("t={t},v1={sig}");
        let now = UNIX_EPOCH + Duration::from_secs(t as u64);

        let err = verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).unwrap_err();
        assert!(matches!(
            err,
            crate::Error::Webhook(WebhookError::InvalidBody(_))
        ));
    }

    #[test]
    fn the_hmac_covers_the_literal_t_text_not_a_re_rendered_number() {
        // A sender that writes `t=01753401600` signs `"01753401600.<body>"`.
        // Hashing `parsed.timestamp.to_string()` instead would hash
        // `"1753401600.<body>"` and reject a delivery that is perfectly
        // genuine — silently, and only for senders whose `t` does not
        // round-trip through an integer.
        let body = sample_body();
        let sig = sign_text("01753401600", &body, SECRET);
        let header = format!("t=01753401600,v1={sig}");
        let now = UNIX_EPOCH + Duration::from_secs(1_753_401_600);

        let event = verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).unwrap();
        assert_eq!(event.id, "evt_1");
    }

    #[test]
    fn a_t_that_is_not_a_run_of_decimal_digits_is_malformed() {
        // Each of these parses as *something* under a more permissive rule —
        // `str::parse::<i64>` takes the signed forms, JavaScript's `Number()`
        // takes the float and hex forms — and each would then have this
        // verifier hash bytes the sender never signed. `webhooks.ts` refuses
        // exactly this set.
        let body = sample_body();
        let now = SystemTime::now();
        for t in [
            "1753401600.0",
            "",
            "+1753401600",
            "-1",
            "0x65566CC0",
            "DEADBEEF",
            "1_753_401_600",
            // Not `" 1753401600"`: both SDKs `trim()` each part before
            // reading it, so surrounding whitespace is accepted and the
            // *trimmed* text is what gets signed — pinned by
            // `an_unparseable_part_and_an_unknown_key_are_both_ignored`.
        ] {
            let sig = sign_text(t, &body, SECRET);
            let header = format!("t={t},v1={sig}");
            let err = verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).unwrap_err();
            assert!(
                matches!(err, crate::Error::Webhook(WebhookError::MalformedHeader)),
                "t={t:?} should be malformed, got {err:?}"
            );
        }
    }

    #[test]
    fn an_unparseable_part_and_an_unknown_key_are_both_ignored() {
        // Forward compatibility, which the old parser's comment claimed and
        // its code did not have: a bare `junk` part made the whole header
        // malformed.
        let body = sample_body();
        let sig = sign_text("1", &body, SECRET);
        // Whitespace around a part is trimmed before it is read, so the
        // signature is still computed over the bare `"1"` — same as Node.
        let header = format!("t= 1 , v1={sig} ,junk,v2=whatever,=novalue");
        let now = UNIX_EPOCH + Duration::from_secs(1);

        let event = verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).unwrap();
        assert_eq!(event.id, "evt_1");
    }

    #[test]
    fn an_empty_v1_is_never_treated_as_a_match() {
        // `hex::decode("")` succeeds, so an empty candidate reaches the
        // comparison as a zero-length "HMAC". It must not be a candidate at
        // all: with no other `v1`, the header carries no signature and is
        // malformed.
        let body = sample_body();
        let now = UNIX_EPOCH + Duration::from_secs(1_753_401_600);
        let err = verify_at(&body, "t=1753401600,v1=", SECRET, DEFAULT_TOLERANCE, now).unwrap_err();
        assert!(
            matches!(err, crate::Error::Webhook(WebhookError::MalformedHeader)),
            "got {err:?}"
        );

        // And an empty one alongside a real one does not disturb the real one.
        let sig = sign(1_753_401_600, &body, SECRET);
        let header = format!("t=1753401600,v1=,v1={sig}");
        assert!(verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).is_ok());
    }

    #[test]
    fn a_timestamp_exactly_on_the_tolerance_boundary_is_accepted() {
        // `|now - t| > tolerance` rejects; `== tolerance` is inside the
        // window. One second either way is the difference between a merchant
        // dropping a genuine delivery and accepting a replay one second past
        // the cutoff, and nothing else in this file pins which it is.
        let body = sample_body();
        let t = 1_753_401_600;
        let sig = sign(t, &body, SECRET);
        let header = format!("t={t},v1={sig}");

        for now_secs in [t as u64 + 300, t as u64 - 300] {
            let now = UNIX_EPOCH + Duration::from_secs(now_secs);
            assert!(
                verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).is_ok(),
                "delta of exactly the tolerance must be accepted (now={now_secs})"
            );
        }
    }

    #[test]
    fn a_t_too_wide_for_an_i64_is_a_tolerance_failure_not_a_malformed_header() {
        // All digits, so the header grammar is satisfied; the instant it
        // names is simply not now. `webhooks.ts` classifies it the same way.
        let body = sample_body();
        let t = "99999999999999999999999";
        let sig = sign_text(t, &body, SECRET);
        let header = format!("t={t},v1={sig}");
        let now = UNIX_EPOCH + Duration::from_secs(1_753_401_600);

        let err = verify_at(&body, &header, SECRET, DEFAULT_TOLERANCE, now).unwrap_err();
        assert!(
            matches!(
                err,
                crate::Error::Webhook(WebhookError::TimestampOutOfTolerance)
            ),
            "got {err:?}"
        );
    }
}
