//! In-process, fixed-window rate limiting for the sign-in endpoints
//! ([ADR-0017](../../../../../docs/adr/0017-staff-authentication.md)
//! decision 2).
//!
//! # Two keys, and why both
//!
//! Per **email**, so that guessing one person's password is bounded however
//! many addresses an attacker has. Per **IP**, so that spreading the same
//! budget across a thousand addresses is bounded too. Either limit alone
//! leaves the other attack unbounded, and neither is a substitute for the
//! other.
//!
//! # Fixed window, and not a lockout
//!
//! A *lockout* — "five failures and this account is frozen" — is a denial of
//! service an attacker triggers by guessing at somebody else's address, which
//! is why ADR-0017 refuses one. A fixed window costs an attacker the same
//! and costs the account's owner a wait rather than a support ticket.
//!
//! Fixed rather than sliding: a sliding window needs a timestamp per attempt
//! and therefore unbounded memory per key under exactly the load it exists to
//! survive. The cost is the classic one and it is stated rather than hidden —
//! **an attacker who straddles a window boundary gets twice the limit in one
//! instant** — and at these numbers (ten attempts per five minutes) that is
//! twenty guesses, which changes nothing about a 130-bit one-time password or
//! a chosen one behind argon2id.
//!
//! # In-process, and what that costs
//!
//! The counters are this replica's. Three replicas admit three times the
//! attempts one does. The alternative is a shared counter in Postgres, which
//! puts a **write** on the unauthenticated path — a denial-of-service
//! amplifier of a different and worse kind, since an attacker would be
//! choosing how much the database writes. ADR-0017's Consequences records
//! this as the first thing to revisit for a deployment that runs many
//! replicas.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};

use time::{Duration, OffsetDateTime};

/// How long one window lasts.
const WINDOW: Duration = Duration::minutes(5);

/// How many sign-in attempts one key may make per window.
///
/// Ten. High enough that a person who mistypes a generated one-time password
/// twice and then fetches it from a terminal is not locked out of their own
/// first login; low enough that an online guessing attack is not a strategy.
const ATTEMPTS_PER_WINDOW: u32 = 10;

/// How many distinct keys one process will track at once.
///
/// Ten thousand. The map is keyed by *caller-supplied* values — an email
/// address and a client address — so without a bound it is a memory
/// exhaustion an unauthenticated caller drives directly. On overflow the
/// whole map is dropped and rebuilt, which resets every counter: that is a
/// deliberate choice of *availability* over *precision*, and it is the
/// direction an attacker can already achieve by waiting five minutes.
const MAX_TRACKED_KEYS: usize = 10_000;

/// What one key is doing in the current window.
#[derive(Debug, Clone, Copy)]
struct Window {
    /// When this window opened.
    opened_at: OffsetDateTime,
    /// Attempts inside it.
    attempts: u32,
}

/// The sign-in rate limiter for one process.
///
/// A `Mutex<HashMap<..>>` rather than anything lock-free: the critical
/// section is a hash lookup and an increment, it is entered once per sign-in
/// attempt, and sign-in attempts are not a hot path. A lock here that ever
/// became contended would mean vpay was under exactly the attack this type
/// exists for, and the queue in front of the mutex is then a feature.
#[derive(Debug, Default)]
pub struct SignInLimiter {
    windows: Mutex<HashMap<String, Window>>,
}

/// Whether an attempt may proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Inside the budget. The attempt has already been counted.
    Allowed,
    /// Over it. The caller answers `429` and does **no** credential work —
    /// which is the point: a refused attempt must not cost an argon2id
    /// verification, or the limit would be the amplifier.
    Limited,
}

impl SignInLimiter {
    /// A limiter with no history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Counts one sign-in attempt against both keys and says whether it may
    /// proceed.
    ///
    /// **Both keys are always counted, even when the first already refuses.**
    /// Short-circuiting would let an attacker who has exhausted one address's
    /// budget keep hammering a thousand others from the same IP without that
    /// IP's counter moving.
    ///
    /// The email is lower-cased by the caller before it gets here, for the
    /// same reason the column is: two spellings of one address must not be
    /// two budgets.
    #[must_use]
    pub fn check(&self, email: &str, peer: Option<IpAddr>, now: OffsetDateTime) -> Verdict {
        let email_ok = self.count(&format!("email:{email}"), now);
        // A caller with no resolvable peer address — behind a proxy that
        // stripped it, or a test over a channel with none — is counted under
        // one shared key rather than not counted at all. That makes the
        // unknown-peer population share a budget, which is the fail-closed
        // reading: the alternative is an unlimited bucket reachable by
        // removing a header.
        let peer_key = peer.map_or_else(|| "ip:unknown".to_owned(), |ip| format!("ip:{ip}"));
        let peer_ok = self.count(&peer_key, now);

        if email_ok && peer_ok {
            Verdict::Allowed
        } else {
            Verdict::Limited
        }
    }

    /// Counts one attempt against one key. `true` if it was inside the
    /// budget.
    fn count(&self, key: &str, now: OffsetDateTime) -> bool {
        // A poisoned mutex means another thread panicked while holding it.
        // The map is a counter; the data behind it cannot be inconsistent in
        // any way that matters, and refusing every sign-in for the life of
        // the process because one thread panicked would be a worse outcome
        // than continuing with the counts.
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);

        if windows.len() >= MAX_TRACKED_KEYS && !windows.contains_key(key) {
            tracing::warn!(
                tracked = windows.len(),
                "the staff sign-in rate limiter is tracking its maximum number of keys and is \
                 resetting every counter; this is what an attack on the login endpoint looks \
                 like from inside"
            );
            windows.clear();
        }

        let window = windows.entry(key.to_owned()).or_insert(Window {
            opened_at: now,
            attempts: 0,
        });

        // A window that has elapsed is *replaced*, not extended: that is what
        // makes this fixed rather than sliding, and it is the boundary
        // behaviour the module header states plainly.
        if now - window.opened_at >= WINDOW {
            *window = Window {
                opened_at: now,
                attempts: 0,
            };
        }

        window.attempts = window.attempts.saturating_add(1);
        window.attempts <= ATTEMPTS_PER_WINDOW
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn peer() -> Option<IpAddr> {
        Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)))
    }

    #[test]
    fn the_budget_is_ten_attempts_and_the_eleventh_is_refused() {
        let limiter = SignInLimiter::new();
        let now = OffsetDateTime::UNIX_EPOCH;

        for attempt in 1..=ATTEMPTS_PER_WINDOW {
            assert_eq!(
                limiter.check("ada@example.test", peer(), now),
                Verdict::Allowed,
                "attempt {attempt} is inside the budget"
            );
        }
        assert_eq!(
            limiter.check("ada@example.test", peer(), now),
            Verdict::Limited
        );
    }

    /// The per-email budget binds even from a fresh address — otherwise a
    /// botnet is an unlimited guessing budget against one account.
    #[test]
    fn the_email_budget_binds_across_addresses() {
        let limiter = SignInLimiter::new();
        let now = OffsetDateTime::UNIX_EPOCH;

        for octet in 0..ATTEMPTS_PER_WINDOW {
            let from = Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, octet as u8)));
            assert_eq!(
                limiter.check("ada@example.test", from, now),
                Verdict::Allowed
            );
        }
        let fresh = Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 200)));
        assert_eq!(
            limiter.check("ada@example.test", fresh, now),
            Verdict::Limited,
            "a new source address must not reset one account's budget"
        );
    }

    /// The per-IP budget binds even across addresses — otherwise credential
    /// stuffing against a list of addresses is unbounded from one host.
    #[test]
    fn the_address_budget_binds_across_emails() {
        let limiter = SignInLimiter::new();
        let now = OffsetDateTime::UNIX_EPOCH;

        for n in 0..ATTEMPTS_PER_WINDOW {
            assert_eq!(
                limiter.check(&format!("person-{n}@example.test"), peer(), now),
                Verdict::Allowed
            );
        }
        assert_eq!(
            limiter.check("someone-else@example.test", peer(), now),
            Verdict::Limited,
            "a new address must not reset one source's budget"
        );
    }

    /// Both counters move on every attempt, including a refused one. The
    /// decisive mutation is short-circuiting `check` on `email_ok`: with
    /// that, the IP counter stops moving once one address is exhausted.
    #[test]
    fn a_refused_attempt_still_counts_against_the_other_key() {
        let limiter = SignInLimiter::new();
        let now = OffsetDateTime::UNIX_EPOCH;

        // Exhaust one address's budget, from one host.
        for _ in 0..=ATTEMPTS_PER_WINDOW {
            let _ = limiter.check("ada@example.test", peer(), now);
        }
        // The host is now over its own budget too, because every one of
        // those attempts counted against it as well.
        assert_eq!(
            limiter.check("someone-else@example.test", peer(), now),
            Verdict::Limited
        );
    }

    /// The window is five minutes, and it *resets* rather than sliding.
    #[test]
    fn the_window_resets_after_five_minutes() {
        let limiter = SignInLimiter::new();
        let now = OffsetDateTime::UNIX_EPOCH;

        for _ in 0..=ATTEMPTS_PER_WINDOW {
            let _ = limiter.check("ada@example.test", peer(), now);
        }
        assert_eq!(
            limiter.check("ada@example.test", peer(), now + Duration::minutes(4)),
            Verdict::Limited,
            "still inside the window"
        );
        assert_eq!(
            limiter.check("ada@example.test", peer(), now + WINDOW),
            Verdict::Allowed,
            "a new window"
        );
    }

    /// A caller with no resolvable address shares one budget rather than
    /// having none. The mutation this catches is skipping the peer count when
    /// `peer` is `None`, which would make an unlimited bucket reachable by
    /// removing whatever supplies the address.
    #[test]
    fn an_unknown_peer_is_counted_rather_than_exempt() {
        let limiter = SignInLimiter::new();
        let now = OffsetDateTime::UNIX_EPOCH;

        for n in 0..=ATTEMPTS_PER_WINDOW {
            let _ = limiter.check(&format!("person-{n}@example.test"), None, now);
        }
        assert_eq!(
            limiter.check("yet-another@example.test", None, now),
            Verdict::Limited
        );
    }
}
