//! The rail-token cache both adapters keep their bearer in.
//!
//! A rail token is minted from credentials, lives for a stated period, and
//! must not be reused across merchants. That much is the same on every rail;
//! the endpoint, the grant, the body shape and the refresh margin are not.
//! This module holds only the first part; `docs/reference/rails.md` records
//! what each adapter keeps for itself and why the refresh margin in
//! particular is passed in rather than defaulted here.

use std::fmt;
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};

/// A digest of the credentials a token was minted from, for use as a cache
/// key.
///
/// Each field is length-prefixed before hashing, so `["ab", "c"]` and
/// `["a", "bc"]` cannot produce one digest by concatenating differently. The
/// whole point of the key is that distinct credentials are distinct, and a
/// concatenation collision would defeat it in exactly the case it exists for.
///
/// Pass the **secret** halves too, not just the identifying ones: a rotated
/// secret must evict the cache on the next call rather than when the bearer
/// ages out. A digest is what makes that safe to hold — the cache keeps a
/// SHA-256, never a credential.
///
/// ```
/// use vpay_provider::token::fingerprint;
///
/// assert_eq!(fingerprint(&["id", "secret"]), fingerprint(&["id", "secret"]));
/// assert_ne!(fingerprint(&["id", "secret"]), fingerprint(&["id", "rotated"]));
/// // The length prefix: a shifted field boundary is a different key.
/// assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
/// ```
#[must_use]
pub fn fingerprint(fields: &[&str]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for field in fields {
        let length = u64::try_from(field.len()).unwrap_or(u64::MAX);
        hasher.update(length.to_be_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.finalize().into()
}

/// When a token minted at `minted_at` stops being usable.
///
/// `margin` is the caller's, never a default of this module's: it is the one
/// number the two adapters do not share, and each pins its own with a test.
///
/// Saturating on both ends, because neither end is the caller's to get wrong:
/// a `lifetime` shorter than `margin` clamps to "already expired" rather than
/// wrapping into a token that never expires, and a rail answering an absurd
/// `expires_in` yields `minted_at` rather than overflowing an [`Instant`]
/// (`Instant + Duration` panics) and taking a worker down with it.
///
/// ```
/// use std::time::{Duration, Instant};
/// use vpay_provider::token::usable_until;
///
/// let minted_at = Instant::now();
/// let hour = Duration::from_secs(3_600);
/// let minute = Duration::from_secs(60);
///
/// assert_eq!(usable_until(minted_at, hour, minute), minted_at + Duration::from_secs(3_540));
/// // A lifetime inside the margin is spent on arrival, never renewed by it.
/// assert_eq!(usable_until(minted_at, Duration::from_secs(30), minute), minted_at);
/// assert_eq!(usable_until(minted_at, Duration::MAX, minute), minted_at);
/// ```
#[must_use]
pub fn usable_until(minted_at: Instant, lifetime: Duration, margin: Duration) -> Instant {
    minted_at
        .checked_add(lifetime.saturating_sub(margin))
        .unwrap_or(minted_at)
}

/// One rail bearer, in memory only, with the moment it stops being usable and
/// the fingerprint of the credentials that produced it.
///
/// Never persisted: a token is short-lived and re-mintable from credentials
/// we already hold, so writing it to the database would put a bearer for a
/// merchant's payment account into backups and replicas for no benefit.
pub struct CachedToken {
    value: String,
    /// Already reduced by the caller's margin. An [`Instant`], not a wall
    /// clock: a token's remaining life is elapsed time, and a machine whose
    /// wall clock steps backwards must not resurrect an expired token.
    expires_at: Instant,
    fingerprint: [u8; 32],
}

impl CachedToken {
    /// Builds the cache entry for a freshly minted token.
    ///
    /// `minted_at` is the caller's, and must be read **before** the token
    /// request was sent rather than after the response arrived: the rail's
    /// `expires_in` counts from the rail's own mint, so measuring from
    /// arrival silently grants the token the round trip as extra life — on a
    /// slow rail, the whole of the margin. Both adapters got this wrong once;
    /// taking it as a parameter is what makes it visible at the call site.
    ///
    /// `margin` is the rail's own — see [`usable_until`].
    #[must_use]
    pub fn new(
        value: String,
        minted_at: Instant,
        lifetime: Duration,
        margin: Duration,
        fingerprint: [u8; 32],
    ) -> Self {
        Self {
            value,
            expires_at: usable_until(minted_at, lifetime, margin),
            fingerprint,
        }
    }

    /// The token as minted, whatever its age.
    ///
    /// Only for the caller that just minted it — a rail may answer with a
    /// lifetime already inside the margin, and refusing to use a bearer the
    /// rail has just said is good would be worse than not caching it.
    /// Everything reading the *cache* goes through [`CachedToken::usable`].
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The token, if it is still valid at `now` **and** was minted from these
    /// credentials.
    ///
    /// `Option<&str>` rather than a `bool` is what keeps the two questions
    /// together: a caller cannot read the value without having asked both.
    /// The fingerprint half is the load-bearing one — the port hands
    /// `&ProviderConfig` to every call, so one adapter value can legitimately
    /// serve two merchants, and a cache that answered on expiry alone would
    /// send merchant B's charges under merchant A's credentials.
    ///
    /// The clock is the caller's so that expiry can be asserted without
    /// sleeping.
    #[must_use]
    pub fn usable(&self, now: Instant, fingerprint: &[u8; 32]) -> Option<&str> {
        (self.fingerprint == *fingerprint && now < self.expires_at).then_some(self.value.as_str())
    }
}

/// Hand-written on purpose: a `Debug` that printed the bearer would leak it
/// into any `tracing` call that formats the adapter holding this.
impl fmt::Debug for CachedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedToken")
            .field("value", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_reused_only_before_expiry_and_only_for_its_own_credentials() {
        let now = Instant::now();
        let mine = fingerprint(&["merchant-a", "secret-a"]);
        let theirs = fingerprint(&["merchant-b", "secret-b"]);
        let token = CachedToken::new(
            "bearer".to_owned(),
            now,
            Duration::from_secs(3_600),
            Duration::from_secs(60),
            mine,
        );

        assert_eq!(token.usable(now, &mine), Some("bearer"));
        assert_eq!(
            token.usable(now, &theirs),
            None,
            "a second merchant's call must never reuse the first's token"
        );
        assert_eq!(
            token.usable(now + Duration::from_secs(3_540), &mine),
            None,
            "at the margin the token is spent"
        );
        assert_eq!(
            token.usable(now + Duration::from_secs(3_539), &mine),
            Some("bearer"),
            "one second earlier it is not"
        );
    }

    /// The margin is the caller's, and this type must not have an opinion:
    /// two rails passing different margins must get different answers from
    /// the same lifetime.
    #[test]
    fn the_margin_is_the_callers_and_this_type_supplies_none() {
        let now = Instant::now();
        let lifetime = Duration::from_secs(600);
        assert_eq!(
            usable_until(now, lifetime, Duration::from_secs(60)),
            now + Duration::from_secs(540)
        );
        assert_eq!(
            usable_until(now, lifetime, Duration::from_secs(120)),
            now + Duration::from_secs(480)
        );
    }

    #[test]
    fn debugging_a_cached_token_does_not_print_it() {
        let rendered = format!(
            "{:?}",
            CachedToken::new(
                "super-secret-bearer".to_owned(),
                Instant::now(),
                Duration::from_secs(3_600),
                Duration::from_secs(60),
                fingerprint(&["client", "secret"]),
            )
        );
        assert!(!rendered.contains("super-secret-bearer"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }
}
