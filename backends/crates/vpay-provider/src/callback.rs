//! What a rail's callback carries besides the identifiers
//! [`parse_callback`](crate::ProviderAdapter::parse_callback) reads, and the
//! one check the route asks of it (RFC-0014).
//!
//! # A verified callback is still a hint
//!
//! The verdict decides exactly one thing: whether
//! `vpay_api::provider_callback` pulls the charge's poll forward. It never
//! decides a status, and it never reaches the charge row. The authenticated
//! [`query_status`](crate::ProviderAdapter::query_status) stays the only
//! thing that moves money, on every rail, whatever any callback said
//! (AGENTS.md, "Callbacks are hints"; RFC-0014 § 1). What a signature buys is
//! that an anonymous POST stops moving a poll on a rail that signs: a wrong
//! signature costs that charge its callback latency and nothing else.
//!
//! # Where the secret comes from
//!
//! A rail that signs is handed its secret in
//! [`ProviderConfig::credentials`](crate::ProviderConfig::credentials), under
//! [`CALLBACK_SIGNING_SECRET`] (and, during a rotation,
//! [`CALLBACK_SIGNING_SECRET_PREVIOUS`]). Two fixed names chosen by the port
//! rather than a key each adapter invents, so that boot can say "this rail
//! signs and no secret is configured" without learning any rail's vocabulary
//! (ADR-0002), and so that the value travels the path every rail secret
//! already travels: written as a `${VAR}` placeholder in livemode, redacted
//! by every `Debug` on the way in, decrypted immediately before use.
//! RFC-0014 § 5 sketched a separate `callbacks.signing_secrets` list; that
//! would have been a fourth secret path through `vpay_config` with its own
//! placeholder rule and its own redaction, and `docs/rfc/0014-verifying-rail-callbacks.md`
//! records why the credentials map was taken instead.
//!
//! # One HMAC comparison for every rail
//!
//! [`hmac_sha256_hex_matches`] is the only place in this workspace that
//! compares a rail's MAC. RFC-0014 § 4: "no adapter writes its own `==` on a
//! MAC". It compares in constant time (`Mac::verify_slice`), so a caller
//! guessing a signature byte by byte learns nothing from how long the
//! refusal took.

use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;

use hmac::{Hmac, Mac};
/// The header types a [`CallbackRequest`] is built from: `http`'s own,
/// which axum hands the route. Re-exported so that a caller building one
/// outside the route (the conformance suite) names them without a
/// dependency of its own.
pub use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::ProviderConfig;

/// The [`ProviderConfig::credentials`] key a rail that signs its callbacks
/// reads its signing secret from.
///
/// Notch Pay calls it the endpoint's "Hash Key" (`hsk_test_…` in its
/// sandbox, RFC-0012 § 6). The configuration name is the port's, not the
/// rail's, for the reason the module header gives.
pub const CALLBACK_SIGNING_SECRET: &str = "callback_signing_secret";

/// The previous signing secret, accepted beside [`CALLBACK_SIGNING_SECRET`]
/// while a rotation is in flight.
///
/// Optional, and only ever a second key: a rotation means a callback signed
/// a moment before the new secret was issued can still arrive, and refusing
/// it would cost that charge its callback latency for no gain.
pub const CALLBACK_SIGNING_SECRET_PREVIOUS: &str = "callback_signing_secret_previous";

/// Which of the two configured signing secrets a value is: the key a
/// signature matched is what tells an operator a rotation can end
/// ([`CallbackVerdict::VerifiedPrevious`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningKey {
    /// [`CALLBACK_SIGNING_SECRET`].
    Current,
    /// [`CALLBACK_SIGNING_SECRET_PREVIOUS`].
    Previous,
}

impl SigningKey {
    /// The credentials key this secret is read from.
    #[must_use]
    pub const fn credential(self) -> &'static str {
        match self {
            Self::Current => CALLBACK_SIGNING_SECRET,
            Self::Previous => CALLBACK_SIGNING_SECRET_PREVIOUS,
        }
    }

    /// What a signature that matched this secret is.
    #[must_use]
    pub const fn verified(self) -> CallbackVerdict {
        match self {
            Self::Current => CallbackVerdict::Verified,
            Self::Previous => CallbackVerdict::VerifiedPrevious,
        }
    }
}

/// The signing secrets configured for this rail, current first, blank ones
/// skipped.
///
/// Empty means none is configured, which an adapter answers with
/// [`CallbackVerdict::NoSecret`]. A blank value is treated as absent rather
/// than as a key: HMAC accepts an empty key, and a callback signed with one
/// is a callback anybody could have signed.
#[must_use]
pub fn callback_signing_secrets(config: &ProviderConfig) -> Vec<&str> {
    signing_secrets_in(&config.credentials)
}

/// [`callback_signing_secrets`], each tagged with which of the two it is, for
/// an adapter that must say which one a signature matched.
#[must_use]
pub fn callback_signing_keys(config: &ProviderConfig) -> Vec<(SigningKey, &str)> {
    signing_keys_in(&config.credentials)
}

/// [`callback_signing_secrets`] over a bare credentials map, for the one
/// caller that holds the YAML's map rather than a [`ProviderConfig`]:
/// `vpay_api::boot::boot_seeds`, deciding whether a rail that signs has a
/// secret. One function, so boot and the adapter cannot disagree about what
/// "configured" means (a blank value is not).
#[must_use]
pub fn signing_secrets_in(credentials: &BTreeMap<String, String>) -> Vec<&str> {
    signing_keys_in(credentials)
        .into_iter()
        .map(|(_, value)| value)
        .collect()
}

/// [`signing_secrets_in`], each tagged with which of the two it is, current
/// first. The one place a blank value is decided to be absent.
#[must_use]
pub fn signing_keys_in(credentials: &BTreeMap<String, String>) -> Vec<(SigningKey, &str)> {
    [SigningKey::Current, SigningKey::Previous]
        .into_iter()
        .filter_map(|key| {
            credentials
                .get(key.credential())
                .map(|value| (key, value.as_str()))
        })
        .filter(|(_, value)| !value.trim().is_empty())
        .collect()
}

/// What the route hands an adapter to check. Borrowed; nothing is stored.
///
/// `Debug` is hand-written: it prints header **names** and the body's
/// length, never a header value (a signature) or a body byte, so a `{:?}` of
/// a request in a log line or a panic cannot carry what a forger would want
/// to replay. `debug_prints_header_names_and_never_a_value` is the canary.
pub struct CallbackRequest<'a> {
    /// The request's headers, as the route received them. `http`'s own type,
    /// which axum hands the route and `reqwest` re-exports: no new package.
    pub headers: &'a HeaderMap,
    /// The raw body bytes: the bytes that were signed, not a re-parse.
    pub body: &'a [u8],
    /// When the route received the request, for a scheme with a timestamp
    /// (Stripe's, RFC-0010 § 6). Notch Pay's carries none.
    pub received_at: SystemTime,
}

impl fmt::Debug for CallbackRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.keys().map(|name| name.as_str()).collect();
        f.debug_struct("CallbackRequest")
            .field("headers", &names)
            .field("body_bytes", &self.body.len())
            .field("received_at", &self.received_at)
            .finish()
    }
}

/// The answer to "is this callback the rail's?" (RFC-0014 § 2).
///
/// Every reason is a `&'static str`: a fixed sentence, never one that quotes
/// a header or a body byte, because the route logs it and a callback's
/// content is chosen by whoever sent it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackVerdict {
    /// The rail's own check passed, under the current signing secret (or by
    /// whatever the rail's scheme checks, for a rail with one secret).
    Verified,
    /// The rail's own check passed under the **previous** signing secret
    /// ([`CALLBACK_SIGNING_SECRET_PREVIOUS`]): a rotation is in flight.
    /// Admits the pull-forward exactly as [`Verified`](Self::Verified) does;
    /// it is a separate answer only so that its counter tells an operator
    /// when the previous secret stopped being used and can be removed.
    /// _(Added 2026-10-23, from the RFC-0014 security review: until then a
    /// match under either key was `Verified`, and a rotation had no end
    /// signal.)_
    VerifiedPrevious,
    /// The check ran and failed. The route does not pull the poll forward.
    Invalid(&'static str),
    /// This adapter checks nothing: the rail signs nothing (MTN), or vpay's
    /// check for it is not built. Today's behaviour: the poll is pulled
    /// forward.
    NotChecked,
    /// The adapter checks, but holds nothing to check this callback with: no
    /// signing secret is configured. Boot has already said so (RFC-0014
    /// § 6), and the poll is pulled forward as it was before this check
    /// existed.
    NoSecret,
    /// The check could not complete. The route does not pull the poll
    /// forward; the poll ladder still runs. No rail answers this today: it is
    /// PayPal's certificate fetch, RFC-0014 § 4.
    Unavailable(&'static str),
}

impl CallbackVerdict {
    /// Whether the route may pull the charge's poll forward on this verdict.
    ///
    /// RFC-0014 § 3's table: yes for [`Verified`](Self::Verified),
    /// [`VerifiedPrevious`](Self::VerifiedPrevious),
    /// [`NotChecked`](Self::NotChecked) and [`NoSecret`](Self::NoSecret),
    /// which is today's behaviour on a rail that checks nothing; no for
    /// [`Invalid`](Self::Invalid) and [`Unavailable`](Self::Unavailable).
    #[must_use]
    pub const fn admits_pull_forward(&self) -> bool {
        matches!(
            self,
            Self::Verified | Self::VerifiedPrevious | Self::NotChecked | Self::NoSecret
        )
    }

    /// The `verdict` label on `vpay_provider_callbacks_total`, spelled once
    /// (`vpay_core::metrics::callback_verdict`).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        use vpay_core::metrics::callback_verdict;
        match self {
            Self::Verified => callback_verdict::VERIFIED,
            Self::VerifiedPrevious => callback_verdict::VERIFIED_PREVIOUS,
            Self::Invalid(_) => callback_verdict::INVALID,
            Self::NotChecked => callback_verdict::NOT_CHECKED,
            Self::NoSecret => callback_verdict::NO_SECRET,
            Self::Unavailable(_) => callback_verdict::UNAVAILABLE,
        }
    }

    /// The fixed sentence an [`Invalid`](Self::Invalid) or
    /// [`Unavailable`](Self::Unavailable) carries, for the route's `warn`.
    #[must_use]
    pub const fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Invalid(reason) | Self::Unavailable(reason) => Some(reason),
            Self::Verified | Self::VerifiedPrevious | Self::NotChecked | Self::NoSecret => None,
        }
    }
}

/// What vpay's adapter checks on this rail's callbacks (RFC-0014 § 2).
///
/// A statement about **vpay**, not about the rail: a rail that signs and
/// whose check is not built declares [`None`](Self::None), and
/// `docs/status/providers.md`'s Callbacks column records both facts. The
/// capability boot reads to decide whether a missing signing secret refuses
/// a livemode deployment, so "does this rail's callback need a secret?" never
/// becomes `if provider == "notchpay"` (ADR-0002).
///
/// Not persisted, on [`CallbackUrlMode`](crate::CallbackUrlMode)'s terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallbackCheck {
    /// Nothing is checked; every callback is
    /// [`CallbackVerdict::NotChecked`].
    None,
    /// A per-payment token in the body is compared with the one stored on
    /// the charge (Orange's `notif_token`, RFC-0014 § 4). Needs no
    /// configured secret. No adapter declares it yet.
    BodyToken,
    /// A signature over the request, keyed by a configured secret
    /// ([`CALLBACK_SIGNING_SECRET`]). `notchpay` since 2026-10-23.
    Signature,
}

/// Whether `received_hex` is the hex HMAC-SHA256 of `message` keyed by
/// `secret`, compared in constant time.
///
/// Upper- or lower-case hex, with surrounding whitespace ignored; anything
/// that is not hex of exactly 32 bytes is `false`, never an error, because a
/// malformed signature and a wrong one are the same answer to the only
/// question asked. An empty `secret` is `false` whatever the signature:
/// HMAC accepts an empty key, and a MAC under one proves nothing.
///
/// The one MAC comparison in this workspace (RFC-0014 § 4).
///
/// ```
/// use vpay_provider::callback::hmac_sha256_hex_matches;
///
/// // RFC 4231-style known answer: HMAC-SHA256("key", "The quick brown fox…").
/// let signature = "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8";
/// let message = b"The quick brown fox jumps over the lazy dog";
/// assert!(hmac_sha256_hex_matches(b"key", message, signature));
/// assert!(hmac_sha256_hex_matches(b"key", message, &signature.to_uppercase()));
/// assert!(!hmac_sha256_hex_matches(b"another key", message, signature));
/// assert!(!hmac_sha256_hex_matches(b"key", b"a different body", signature));
/// assert!(!hmac_sha256_hex_matches(b"", message, signature));
/// ```
#[must_use]
pub fn hmac_sha256_hex_matches(secret: &[u8], message: &[u8], received_hex: &str) -> bool {
    if secret.is_empty() {
        return false;
    }
    let Ok(received) = hex::decode(received_hex.trim()) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(message);
    // `verify_slice` is the constant-time comparison (`subtle` underneath);
    // it also refuses a slice of the wrong length.
    mac.verify_slice(&received).is_ok()
}

/// The hex HMAC-SHA256 of `message` keyed by `secret`: what a rail that signs
/// as Notch Pay does would put in its header.
///
/// For tests and stubs that need to produce a genuine signature; the route
/// never signs anything. Public so the conformance suite and the integration
/// tests sign with the same function the check is proven against, rather
/// than each writing its own HMAC.
///
/// `None` never happens: HMAC accepts a key of any length. It is an
/// `Option` only because `Mac::new_from_slice` is typed fallible, and this
/// crate does not panic in non-test code (ADR-0007).
#[must_use]
pub fn hmac_sha256_hex(secret: &[u8], message: &[u8]) -> Option<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).ok()?;
    mac.update(message);
    Some(hex::encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const KEY: &[u8] = b"key";
    const MESSAGE: &[u8] = b"The quick brown fox jumps over the lazy dog";
    /// HMAC-SHA256("key", "The quick brown fox jumps over the lazy dog"), the
    /// widely published known answer.
    const KNOWN: &str = "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8";

    fn config_with(credentials: &[(&str, &str)]) -> ProviderConfig {
        ProviderConfig {
            base_url: "https://rail.example".to_owned(),
            callback_url: "https://vpay.example/provider/rail/callback".to_owned(),
            currency: vpay_core::Currency::Xaf,
            settings: BTreeMap::new(),
            credentials: credentials
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            connect_timeout: Duration::from_secs(1),
            request_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn the_known_answer_is_produced_and_accepted() {
        assert_eq!(hmac_sha256_hex(KEY, MESSAGE).as_deref(), Some(KNOWN));
        assert!(hmac_sha256_hex_matches(KEY, MESSAGE, KNOWN));
    }

    /// The body is compared byte for byte: one byte more, one byte less, or
    /// the same JSON re-serialised is a different message.
    #[test]
    fn the_raw_body_is_what_is_compared() {
        let body = br#"{"event":"payment.complete","data":{"reference":"x"}}"#;
        let signature = hmac_sha256_hex(KEY, body).expect("HMAC takes any key");
        assert!(hmac_sha256_hex_matches(KEY, body, &signature));
        let spaced = br#"{"event": "payment.complete", "data": {"reference": "x"}}"#;
        assert!(!hmac_sha256_hex_matches(KEY, spaced, &signature));
        let mut longer = body.to_vec();
        longer.push(b'\n');
        assert!(!hmac_sha256_hex_matches(KEY, &longer, &signature));
        let shorter = body.get(..body.len() - 1).expect("the body is not empty");
        assert!(!hmac_sha256_hex_matches(KEY, shorter, &signature));
    }

    #[test]
    fn a_malformed_or_truncated_signature_is_simply_false() {
        for received in [
            "",
            "not hex",
            KNOWN.get(..62).expect("within the constant"),
            KNOWN.get(..63).expect("within the constant"),
            &format!("{KNOWN}00"),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd9",
        ] {
            assert!(
                !hmac_sha256_hex_matches(KEY, MESSAGE, received),
                "{received:?} must not verify"
            );
        }
        assert!(hmac_sha256_hex_matches(
            KEY,
            MESSAGE,
            &format!("  {KNOWN}\n")
        ));
    }

    #[test]
    fn an_empty_secret_verifies_nothing() {
        let signed_with_empty = hmac_sha256_hex(b"", MESSAGE).expect("HMAC takes any key");
        assert!(!hmac_sha256_hex_matches(b"", MESSAGE, &signed_with_empty));
    }

    #[test]
    fn the_signing_secrets_are_current_first_and_skip_blanks() {
        assert!(callback_signing_secrets(&config_with(&[])).is_empty());
        assert!(
            callback_signing_secrets(&config_with(&[(CALLBACK_SIGNING_SECRET, "  ")])).is_empty()
        );
        assert_eq!(
            callback_signing_secrets(&config_with(&[
                (CALLBACK_SIGNING_SECRET_PREVIOUS, "old"),
                (CALLBACK_SIGNING_SECRET, "new"),
                ("public_key", "not a signing secret"),
            ])),
            ["new", "old"]
        );
        assert_eq!(
            callback_signing_secrets(&config_with(&[(CALLBACK_SIGNING_SECRET_PREVIOUS, "old")])),
            ["old"]
        );
    }

    /// Which key a secret is travels with it, current first, so an adapter
    /// can answer `VerifiedPrevious` for a match under the previous one.
    #[test]
    fn the_signing_keys_say_which_secret_each_is() {
        assert_eq!(
            callback_signing_keys(&config_with(&[
                (CALLBACK_SIGNING_SECRET_PREVIOUS, "old"),
                (CALLBACK_SIGNING_SECRET, "new"),
            ])),
            [(SigningKey::Current, "new"), (SigningKey::Previous, "old")]
        );
        assert_eq!(
            callback_signing_keys(&config_with(&[
                (CALLBACK_SIGNING_SECRET, " "),
                (CALLBACK_SIGNING_SECRET_PREVIOUS, "old"),
            ])),
            [(SigningKey::Previous, "old")]
        );
        assert_eq!(SigningKey::Current.verified(), CallbackVerdict::Verified);
        assert_eq!(
            SigningKey::Previous.verified(),
            CallbackVerdict::VerifiedPrevious
        );
    }

    #[test]
    fn only_a_verdict_that_is_not_a_refusal_admits_the_pull_forward() {
        assert!(CallbackVerdict::Verified.admits_pull_forward());
        assert!(CallbackVerdict::VerifiedPrevious.admits_pull_forward());
        assert!(CallbackVerdict::NotChecked.admits_pull_forward());
        assert!(CallbackVerdict::NoSecret.admits_pull_forward());
        assert!(!CallbackVerdict::Invalid("x").admits_pull_forward());
        assert!(!CallbackVerdict::Unavailable("x").admits_pull_forward());
    }

    #[test]
    fn every_verdict_has_a_label_from_the_metric_vocabulary() {
        use vpay_core::metrics::callback_verdict;
        let labels = [
            CallbackVerdict::Verified.label(),
            CallbackVerdict::VerifiedPrevious.label(),
            CallbackVerdict::Invalid("x").label(),
            CallbackVerdict::NotChecked.label(),
            CallbackVerdict::NoSecret.label(),
            CallbackVerdict::Unavailable("x").label(),
        ];
        assert_eq!(labels, callback_verdict::ALL);
    }

    /// Constant time is a property of the comparison, and a timing test on
    /// a shared CI runner would prove nothing either way. So this pins the
    /// structure instead: the one comparison delegates to `Mac::verify_slice`
    /// (`subtle::ConstantTimeEq` underneath), and the function contains no
    /// `==`, `!=` or `.eq(` a later edit could have swapped in. Replacing
    /// `verify_slice` with `hex::encode(…) == received_hex`, the obvious
    /// short-circuiting comparison, fails here.
    #[test]
    fn the_comparison_is_the_constant_time_one() {
        let source = include_str!("callback.rs");
        let start = source
            .find("pub fn hmac_sha256_hex_matches(")
            .expect("the comparison is in this file");
        let length = source
            .get(start..)
            .and_then(|rest| rest.find("\n}\n"))
            .expect("the function closes");
        let body = source.get(start..start + length).expect("within the file");
        assert!(body.contains(".verify_slice("), "{body}");
        for comparison in ["==", "!=", ".eq("] {
            assert!(!body.contains(comparison), "{comparison} in:\n{body}");
        }
    }

    /// The canary RFC-0014 § 7 asks for: a header's value and a body byte
    /// never reach a `{:?}`.
    #[test]
    fn debug_prints_header_names_and_never_a_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-notch-signature",
            HeaderValue::from_static("canary-signature-value"),
        );
        let request = CallbackRequest {
            headers: &headers,
            body: b"canary-body-byte",
            received_at: SystemTime::UNIX_EPOCH,
        };
        let rendered = format!("{request:?}");
        assert!(rendered.contains("x-notch-signature"), "{rendered}");
        assert!(!rendered.contains("canary-signature-value"), "{rendered}");
        assert!(!rendered.contains("canary-body-byte"), "{rendered}");
    }
}
