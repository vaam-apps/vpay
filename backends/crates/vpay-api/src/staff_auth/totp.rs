//! RFC 6238 time-based one-time passwords — the second of a staff member's
//! two factors, and the one number the replay guard turns (ADR-0017
//! decision 1).
//!
//! # The three parameters, and why each is what it is
//!
//! * **HMAC-SHA1**, RFC 6238's default. Not a security choice — SHA-256 is
//!   also specified and this code would be one line different — but an
//!   *interoperability* one: `algorithm=SHA256` in an `otpauth://` URI is
//!   honoured by some authenticator apps and silently ignored by others,
//!   which produces an app that shows six digits vpay will never accept and a
//!   staff member with no way to find out why. SHA-1's weakness is collision
//!   resistance; HMAC-SHA1 does not depend on it.
//! * **30-second step** ([`STEP_SECONDS`]), RFC 6238's recommendation and
//!   every authenticator's default.
//! * **±1 step** ([`SKEW_STEPS`]), RFC 6238 §5.2's own guidance: "at most one
//!   time step". A wider window is a wider replay surface for exactly the
//!   same usability.
//!
//! # The window and the replay guard are one design, not two
//!
//! Accepting ±1 step means three codes are valid at any instant, which is
//! what makes TOTP usable on a phone whose clock drifts. It also means an
//! attacker who reads one code over somebody's shoulder has up to 90 seconds
//! to use it. [`Totp::verify`] therefore returns the **step it matched**, and
//! `vpay_db::Staff::record_totp_step` refuses to record a step that is not
//! strictly greater than the last accepted one — so the second presentation
//! of one code is refused however fast it arrives.
//!
//! Neither half works alone: the window without the guard admits replay, and
//! the guard without the window would refuse a phone that is four seconds
//! fast. That is why this module returns a step rather than a boolean.

use hmac::{Hmac, Mac as _};
use sha1::Sha1;
use subtle::ConstantTimeEq as _;

/// Seconds per time step. RFC 6238's `X`.
pub const STEP_SECONDS: i64 = 30;

/// How many steps either side of the current one are accepted. RFC 6238
/// §5.2's "at most one time step".
pub const SKEW_STEPS: i64 = 1;

/// How many digits a code has. Six: RFC 6238's default and every
/// authenticator's.
pub const DIGITS: u32 = 6;

/// The shared secret's length, in bytes. Twenty — RFC 4226 §4 R6's minimum is
/// 128 bits and its recommendation is 160, which is also HMAC-SHA1's block
/// output and what every authenticator expects from a base32 secret.
pub const SECRET_BYTES: usize = 20;

/// `10^DIGITS`, the modulus RFC 4226 §5.3 applies to the truncated value.
const MODULUS: u32 = 1_000_000;

/// The RFC 6238 step a Unix timestamp falls in.
///
/// A free function because both [`Totp::verify`] and the tests need it, and
/// because it is the number the database stores — `staff_members.last_totp_step` is
/// this, not a code and not an instant.
#[must_use]
pub fn step_at(unix_seconds: i64) -> i64 {
    unix_seconds.div_euclid(STEP_SECONDS)
}

/// One staff member's TOTP secret, in the clear.
///
/// Held only for the length of a verification: it is opened out of
/// `staff_members.totp_secret` by [`crate::staff_auth::StaffCredentials::open_secret`],
/// used, and dropped. Deliberately **not** `Clone`: a second factor with two
/// owners is a second factor somebody forgot to drop.
///
/// Its `Debug` prints the type name and nothing else — not even a length,
/// which for a fixed-size secret would be a constant dressed as information.
/// The workspace lints `missing_debug_implementations`, so "no `Debug` at
/// all" is not available; the next best thing is one that carries nothing,
/// and it is written by hand precisely so that a future `#[derive(Debug)]`
/// is a visible change rather than an omission.
pub struct Totp {
    secret: Vec<u8>,
}

impl std::fmt::Debug for Totp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Totp([redacted])")
    }
}

impl Totp {
    /// Wraps an already-opened secret.
    #[must_use]
    pub fn new(secret: Vec<u8>) -> Self {
        Self { secret }
    }

    /// The code for one step. Public because [`Self::verify`] is defined in
    /// terms of it and because a test that recomputed it independently would
    /// be testing its own arithmetic.
    #[must_use]
    pub fn code_at_step(&self, step: i64) -> String {
        // RFC 4226 §5.2: the counter is an 8-byte big-endian value. `as u64`
        // on a negative step would be a pre-1970 instant, which cannot arise
        // from a system clock this process trusts; it still wraps rather than
        // panics, which is the behaviour this crate's lint policy requires.
        let counter = (step as u64).to_be_bytes();

        // `new_from_slice` accepts any key length (HMAC pads or hashes), so
        // this cannot fail for a secret of any size — but the API is
        // fallible, and this crate denies `expect`, so a failure falls
        // through to a code no caller will match.
        let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(&self.secret) else {
            return String::new();
        };
        mac.update(&counter);
        let digest = mac.finalize().into_bytes();

        // RFC 4226 §5.3, dynamic truncation.
        let offset = usize::from(digest.last().copied().unwrap_or(0) & 0x0f);
        let selected = digest.get(offset..offset + 4).unwrap_or(&[0, 0, 0, 0]);
        let binary = u32::from_be_bytes([
            selected.first().copied().unwrap_or(0) & 0x7f,
            selected.get(1).copied().unwrap_or(0),
            selected.get(2).copied().unwrap_or(0),
            selected.get(3).copied().unwrap_or(0),
        ]);

        format!(
            "{:0width$}",
            binary % MODULUS,
            width = DIGITS as usize
        )
    }

    /// Which step `code` is the code for, at `unix_seconds`, or `None`.
    ///
    /// Returns the **step** rather than a boolean, because the caller's next
    /// move is to record it: see this module's header for why the skew window
    /// and the replay guard are one design.
    ///
    /// The candidate steps are tried oldest first, and every one is compared
    /// in constant time — `subtle::ConstantTimeEq`, not `==`. A `==` on two
    /// six-digit strings exits at the first differing byte, which is a timing
    /// oracle over a six-digit space an attacker may present a thousand times
    /// a minute. Every candidate is compared even after one has matched, for
    /// the same reason.
    #[must_use]
    pub fn verify(&self, code: &str, unix_seconds: i64) -> Option<i64> {
        let current = step_at(unix_seconds);
        let mut matched: Option<i64> = None;

        for offset in -SKEW_STEPS..=SKEW_STEPS {
            let step = current.saturating_add(offset);
            let candidate = self.code_at_step(step);
            // `bool::from(...ct_eq(...))` and not `if ...` so the loop body
            // does the same work whichever way the comparison went.
            if bool::from(candidate.as_bytes().ct_eq(code.as_bytes())) {
                matched = Some(step);
            }
        }

        matched
    }
}

/// RFC 4648 §6 base32, upper case and **unpadded** — how every authenticator
/// expects a secret in an `otpauth://` URI.
///
/// Hand-written rather than a dependency: it is twenty lines, the alphabet is
/// fixed by the RFC, and the alternative is a crate in the graph of a payment
/// binary for one encoding used on one screen. The tests below pin it against
/// RFC 4648 §10's own vectors, which is what makes "hand-written" safe rather
/// than merely small.
#[must_use]
pub fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u16 = 0;
    let mut bits: u8 = 0;

    for byte in bytes {
        buffer = (buffer << 8) | u16::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = usize::from((buffer >> bits) & 0x1f);
            out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'A')));
        }
    }

    if bits > 0 {
        let index = usize::from((buffer << (5 - bits)) & 0x1f);
        out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'A')));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 §10's own base32 vectors. The hand-written encoder is only
    /// safe because these are here.
    #[test]
    fn base32_matches_rfc_4648_section_10() {
        for (input, expected) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32(input.as_bytes()), expected, "input {input:?}");
        }
    }

    /// RFC 6238 Appendix B's test vectors, for the SHA-1 secret
    /// `12345678901234567890`. These are the reason this module can claim
    /// "RFC 6238" rather than "a TOTP-shaped thing".
    #[test]
    fn the_codes_match_rfc_6238_appendix_b() {
        let totp = Totp::new(b"12345678901234567890".to_vec());

        for (unix_seconds, expected) in [
            (59_i64, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ] {
            assert_eq!(
                totp.code_at_step(step_at(unix_seconds)),
                expected,
                "RFC 6238 Appendix B, T = {unix_seconds}"
            );
        }
    }

    /// A code is accepted one step either side and refused two — and the
    /// returned step is the one the guard will record.
    #[test]
    fn the_window_is_exactly_one_step_wide_in_each_direction() {
        let totp = Totp::new(b"12345678901234567890".to_vec());
        let now = 1_234_567_890_i64;
        let step = step_at(now);

        assert_eq!(
            totp.verify(&totp.code_at_step(step), now),
            Some(step),
            "the current step"
        );
        assert_eq!(
            totp.verify(&totp.code_at_step(step - 1), now),
            Some(step - 1),
            "one step behind: a phone whose clock is slow"
        );
        assert_eq!(
            totp.verify(&totp.code_at_step(step + 1), now),
            Some(step + 1),
            "one step ahead: a phone whose clock is fast"
        );
        assert_eq!(
            totp.verify(&totp.code_at_step(step - 2), now),
            None,
            "two steps is outside RFC 6238 5.2's guidance and must be refused"
        );
        assert_eq!(totp.verify(&totp.code_at_step(step + 2), now), None);
    }

    /// The step is what the database records, so it has to be the step and
    /// not the instant. Sixty seconds is two steps; twenty-nine is none.
    #[test]
    fn a_step_is_thirty_seconds_wide() {
        assert_eq!(step_at(0), 0);
        assert_eq!(step_at(29), 0);
        assert_eq!(step_at(30), 1);
        assert_eq!(step_at(59), 1);
        assert_eq!(step_at(60), 2);
    }

    /// Nothing outside the window, and nothing shaped wrongly, is accepted.
    #[test]
    fn junk_is_refused() {
        let totp = Totp::new(b"12345678901234567890".to_vec());
        let now = 1_234_567_890_i64;

        for code in ["", "000000", "5924", "0059240", "abcdef", "  005924  "] {
            // `005924` IS the code for this instant, so the padded and
            // truncated forms must be refused as different strings rather
            // than trimmed into a match.
            if code == "005924" {
                continue;
            }
            assert_eq!(totp.verify(code, now), None, "accepted {code:?}");
        }
        assert_eq!(totp.verify("005924", now), Some(step_at(now)));
    }

    /// A different secret produces different codes. The assertion that fails
    /// if the HMAC key is ever dropped on the floor.
    #[test]
    fn another_secret_does_not_produce_the_same_code() {
        let now = 1_234_567_890_i64;
        let mine = Totp::new(b"12345678901234567890".to_vec());
        let theirs = Totp::new(b"09876543210987654321".to_vec());

        assert_eq!(theirs.verify(&mine.code_at_step(step_at(now)), now), None);
    }

    /// Every code is six digits, padded — a truncated `0`-leading code is the
    /// classic TOTP bug and it makes one code in ten fail for one user in
    /// ten.
    #[test]
    fn every_code_is_six_digits() {
        let totp = Totp::new(b"12345678901234567890".to_vec());
        for step in 0..2_000_i64 {
            let code = totp.code_at_step(step);
            assert_eq!(code.len(), DIGITS as usize, "step {step} gave {code}");
            assert!(code.bytes().all(|b| b.is_ascii_digit()), "{code}");
        }
    }
}
