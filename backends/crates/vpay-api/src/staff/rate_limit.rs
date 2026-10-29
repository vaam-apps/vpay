//! Fixed-window rate limiting for the credential endpoints, **shared by every
//! replica** ([ADR-0017](../../../../../docs/adr/0017-staff-authentication.md)
//! decision 2, amended 2026-09-10 for issue #79 item 2).
//!
//! # It was in process, and per-replica limiting is not a limit
//!
//! This module was a `Mutex<HashMap<String, Window>>` until 2026-09-10.
//! ADR-0017 said what that cost, in its own Consequences: "**The rate limit
//! is per replica.** Three replicas admit three times the attempts a single
//! one does … the first thing to revisit if a deployment runs many
//! replicas." It also named the alternative and refused it — "a shared
//! counter in Postgres … puts a write on the unauthenticated path, which is a
//! denial-of-service amplifier of a different kind".
//!
//! Both sentences were true and the amendment takes the second trade with the
//! amplifier bounded rather than accepted. The write is one statement,
//! keyed by a **digest** so an unauthenticated caller cannot choose how wide
//! a row is, and it sweeps sixteen times more elapsed rows than it adds — see
//! `vpay_db::rate_limits`, which argues each of the three in full, and
//! migration `0038`, which an operator will read first.
//!
//! What that buys is the thing a per-replica limiter cannot have: **ten
//! attempts means ten attempts**, whatever a deployment's replica count is
//! and however a load balancer spreads a burst across it.
//!
//! # Two keys on a sign-in, and why both
//!
//! Per **email**, so that guessing one person's password is bounded however
//! many addresses an attacker has. Per **address**, so that spreading the
//! same budget across a thousand addresses is bounded too. Either limit alone
//! leaves the other attack unbounded, and neither is a substitute for the
//! other.
//!
//! **Both are counted on every attempt, even when the first already
//! refuses.** Short-circuiting would let an attacker who has exhausted one
//! address's budget keep hammering a thousand others from the same host
//! without that host's counter moving. It costs one extra statement on an
//! attempt that is already being refused, which is the cheapest thing on this
//! path.
//!
//! # Fixed window, and not a lockout
//!
//! A *lockout* — "five failures and this account is frozen" — is a denial of
//! service an attacker triggers by guessing at somebody else's address, which
//! is why ADR-0017 refuses one. Making the counter durable does not change
//! that argument; if anything it sharpens it, because a durable lockout would
//! survive the restart that used to clear it.
//!
//! Fixed rather than sliding: a sliding window needs a timestamp per attempt
//! and therefore a row per attempt, which is the unbounded table the digest
//! key and the sweep exist to avoid. The cost is the classic one and it is
//! stated rather than hidden — **an attacker who straddles a window boundary
//! gets twice the limit in one instant** — and at these numbers that is
//! twenty guesses, which changes nothing about a 130-bit one-time password
//! or a chosen one behind argon2id.
//!
//! # Failing closed
//!
//! Every method here returns `Result`, and a database failure is an `Err`
//! that the handler turns into a refusal. A limiter that answered "allowed"
//! when it could not count is not a limiter: an attacker who can make one
//! statement fail — by exhausting the pool with the very attempts being
//! counted — would have removed the limit by attacking it.
//!
//! # Where the address comes from
//!
//! [`crate::staff::client_address`], which reads `X-Forwarded-For` **only**
//! from a peer named in `staff_auth.trusted_proxies` and otherwise counts the
//! transport peer. A `None` peer is counted under one shared key rather than
//! exempted — the fail-closed reading, and the right one: the alternative is
//! an unlimited bucket reachable by removing whatever supplies the address.
//! That is not hypothetical. Until the exp24 review (2026-09-07, finding F2)
//! neither `vpay-server` nor the integration harness built its service with
//! `into_make_service_with_connect_info`, so the peer was `None` on every
//! request and the whole deployment shared one bucket.
//!
//! # Which endpoints spend from which budget
//!
//! * [`crate::staff::login`] and [`crate::staff::totp_step`] — **one** shared
//!   sign-in budget, per email and per address. One budget across the two
//!   legs is ADR-0017 decision 2: `login` lower-cases the address it was
//!   given and `totp_step` uses the row's, which migration `0035` constrains
//!   to lower case, so one account is one budget across both.
//!
//!   `totp_step` did **not** spend from it until the exp28 review
//!   (2026-09-07). A second factor is six digits, three of them live at any
//!   instant given `Totp`'s one-step skew, on a path that costs no argon2id —
//!   the cheapest credential in this design to guess and the only one nothing
//!   bounded. Measured against a real stack: thirty consecutive wrong codes,
//!   thirty `401`s, no `429`.
//!
//!   `login` counts **every** attempt, before any credential work, because an
//!   attempt over budget must not cost an argon2id verification.
//!   `totp_step` counts only a **wrong** code, after the verification: the
//!   budget is shared, and behind a proxy with no allow-list the per-address
//!   half is shared by the whole deployment, so counting successful second
//!   factors would have halved how many people can sign in per window to
//!   close a hole only wrong codes exploit. There is no argon2id on that path
//!   to protect, so the count can wait until the answer is known.
//!
//! * [`crate::staff::change_password`] — its own budget, keyed by the
//!   **session** (issue #79 item 3). Since 2026-09-10 that endpoint verifies
//!   the current password, which is an argon2id verification an attacker
//!   holding a stolen session cookie can drive. Keyed by the session rather
//!   than by the email because the session token is the narrowest thing
//!   identifying that caller, and because a budget shared with sign-in would
//!   let a thief lock the owner out of their own login by guessing at the
//!   password change.

use std::net::IpAddr;

use time::{Duration, OffsetDateTime};
use vpay_db::Repositories;

use crate::ApiError;
use crate::staff_auth::tokens;

/// One budget: how many attempts, over how long.
///
/// A value rather than two constants, because the numbers are configuration
/// since 2026-09-10 (`vpay_config::RateLimitPolicy`, which carries the
/// defaults and the argument for them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Attempts admitted per window. The `attempts + 1`th is refused.
    attempts: u32,
    /// How long one window lasts.
    window: Duration,
}

impl Policy {
    /// A policy from the configured pair.
    ///
    /// `i64::from` on both, so the arithmetic that builds the `Duration`
    /// cannot overflow: `vpay_config` validates each as a `u32` at least 1,
    /// and every `u32` is an `i64`.
    #[must_use]
    fn from_config(configured: vpay_config::RateLimitPolicy) -> Self {
        Self {
            attempts: configured.attempts,
            window: Duration::seconds(i64::from(configured.window_seconds)),
        }
    }
}

/// The kinds of budget this deployment counts, and the three values
/// `rate_limit_windows_scope_is_known` admits.
///
/// A closed enum mirroring a database CHECK, exactly as
/// `vpay_db::StaffStatus` mirrors `staff_members_status_is_known`: a scope
/// spelled here and not there is refused at the insert rather than written
/// and never understood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    /// One account's sign-in budget, across both legs.
    SignInEmail,
    /// One source address's sign-in budget, across every account.
    SignInAddress,
    /// One session's current-password budget.
    ChangePasswordSession,
}

impl Budget {
    /// The stored spelling.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Budget::SignInEmail => "sign_in:email",
            Budget::SignInAddress => "sign_in:address",
            Budget::ChangePasswordSession => "change_password:session",
        }
    }
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

impl Verdict {
    /// `Limited` if `attempts` is past `policy`.
    ///
    /// `>` and not `>=`: `count_attempt` returns the position of the attempt
    /// being counted, *including itself*, so a budget of ten admits the
    /// answer ten and refuses eleven. The off-by-one this spelling avoids is
    /// a limiter that admits one fewer attempt than the operator configured,
    /// which nobody would ever notice.
    fn of(attempts: i64, policy: Policy) -> Self {
        if attempts > i64::from(policy.attempts) {
            Verdict::Limited
        } else {
            Verdict::Allowed
        }
    }
}

/// The deployment's rate-limiting policy, and the only thing that counts an
/// attempt.
///
/// Holds no state: every counter is a row. It is `Clone`-free and shared as
/// `Arc<StaffLogin>` like everything else on that struct, and it could as
/// easily be two `Copy` policies — it is a type so that the *keys* are
/// composed in one place, which is the half of this design a caller could
/// otherwise get subtly wrong.
#[derive(Debug, Clone, Copy)]
pub struct SignInLimiter {
    sign_in: Policy,
    change_password: Policy,
}

impl SignInLimiter {
    /// A limiter over this deployment's configured policies.
    #[must_use]
    pub fn new(limits: vpay_config::RateLimits) -> Self {
        Self {
            sign_in: Policy::from_config(limits.sign_in),
            change_password: Policy::from_config(limits.change_password),
        }
    }

    /// Counts one **sign-in** attempt against both keys and says whether it
    /// may proceed.
    ///
    /// The email is lower-cased by the caller before it gets here, for the
    /// same reason the column is: two spellings of one address must not be
    /// two budgets.
    ///
    /// Both keys are always counted — see the module header for the attack
    /// that short-circuiting reopens. The two statements are sequential
    /// rather than concurrent because they are two rows in one pool and a
    /// `join!` here would hold two connections per refused attempt, which is
    /// the resource an attacker is already trying to exhaust.
    ///
    /// # Errors
    ///
    /// [`ApiError::Db`] if either count fails. The caller must **not** treat
    /// that as an allowance: a limiter that fails open has been removed by
    /// the attack it exists to bound.
    pub async fn check_sign_in(
        &self,
        repositories: &dyn Repositories,
        email: &str,
        address: Option<IpAddr>,
        now: OffsetDateTime,
    ) -> Result<Verdict, ApiError> {
        let by_email = self
            .count(repositories, Budget::SignInEmail, email, self.sign_in, now)
            .await?;
        // A caller with no resolvable address — behind a proxy that stripped
        // it, or a channel that has none — is counted under one shared key
        // rather than not counted at all. That makes the unknown-address
        // population share a budget, which is the fail-closed reading: the
        // alternative is an unlimited bucket reachable by removing whatever
        // supplies the address.
        let key = address.map_or_else(|| "unknown".to_owned(), |ip| ip.to_string());
        let by_address = self
            .count(repositories, Budget::SignInAddress, &key, self.sign_in, now)
            .await?;

        Ok(match (by_email, by_address) {
            (Verdict::Allowed, Verdict::Allowed) => Verdict::Allowed,
            _ => Verdict::Limited,
        })
    }

    /// Counts one **current-password** attempt against the session making it.
    ///
    /// `session_digest` is `staff_sessions.id` — the SHA-256 of the token,
    /// which the handler already holds. It is hashed again here rather than
    /// used raw, because every key on `rate_limit_windows` is a digest of a
    /// scoped pre-image and a single exception would be the one an operator
    /// reading the table could correlate back to a live session row.
    ///
    /// # Errors
    ///
    /// [`ApiError::Db`], which the caller must treat as a refusal.
    pub async fn check_password_change(
        &self,
        repositories: &dyn Repositories,
        session_digest: &str,
        now: OffsetDateTime,
    ) -> Result<Verdict, ApiError> {
        self.count(
            repositories,
            Budget::ChangePasswordSession,
            session_digest,
            self.change_password,
            now,
        )
        .await
    }

    /// Counts one attempt against one key.
    ///
    /// The key is `SHA-256("<scope>:<value>")`. **Scoped**, so that the same
    /// email cannot collide with the same string used as another budget's
    /// value, and **hashed**, for migration `0038`'s two reasons: an
    /// unauthenticated caller chooses the value, and for the commonest key it
    /// is an email address with no account behind it.
    async fn count(
        &self,
        repositories: &dyn Repositories,
        budget: Budget,
        value: &str,
        policy: Policy,
        now: OffsetDateTime,
    ) -> Result<Verdict, ApiError> {
        let scope = budget.as_wire_str();
        let id = tokens::digest(&format!("{scope}:{value}"));
        let attempts = repositories
            .count_attempt(&id, scope, policy.window, now)
            .await?;
        Ok(Verdict::of(attempts, policy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(attempts: u32, window_seconds: u32) -> Policy {
        Policy::from_config(vpay_config::RateLimitPolicy {
            attempts,
            window_seconds,
        })
    }

    /// The budget is `attempts`, and the `attempts + 1`th is refused.
    ///
    /// The whole of the verdict, and the only arithmetic in this module.
    /// `count_attempt` answers the position of the attempt it just counted,
    /// so ten is the tenth attempt and is inside a budget of ten.
    #[test]
    fn the_nth_attempt_is_inside_a_budget_of_n_and_the_next_is_not() {
        let ten = policy(10, 300);

        assert_eq!(Verdict::of(1, ten), Verdict::Allowed, "the first attempt");
        assert_eq!(Verdict::of(10, ten), Verdict::Allowed, "the tenth");
        assert_eq!(Verdict::of(11, ten), Verdict::Limited, "the eleventh");

        let five = policy(5, 300);
        assert_eq!(Verdict::of(5, five), Verdict::Allowed);
        assert_eq!(
            Verdict::of(6, five),
            Verdict::Limited,
            "a deployment that configures five gets five"
        );
    }

    /// The configured numbers reach the policy unchanged, including the
    /// window, which is the one that is converted.
    #[test]
    fn the_configured_policy_is_the_policy() {
        let limiter = SignInLimiter::new(vpay_config::RateLimits::default());

        assert_eq!(limiter.sign_in, policy(10, 300), "ADR-0017's numbers");
        assert_eq!(
            limiter.change_password,
            policy(5, 300),
            "tighter, because nothing here is a printed password somebody is copying by hand"
        );
        assert_eq!(limiter.sign_in.window, Duration::minutes(5));
    }

    /// The three scopes are exactly the three
    /// `rate_limit_windows_scope_is_known` admits, and they fit the column.
    ///
    /// The mutation this catches is renaming a variant's wire string: the
    /// insert would then be refused by the CHECK on every attempt, which
    /// fails closed but as a `500` on a login form rather than as a `429`.
    #[test]
    fn the_scopes_are_the_ones_the_database_admits() {
        let scopes = [
            Budget::SignInEmail,
            Budget::SignInAddress,
            Budget::ChangePasswordSession,
        ]
        .map(Budget::as_wire_str);

        assert_eq!(
            scopes,
            [
                "sign_in:email",
                "sign_in:address",
                "change_password:session"
            ],
            "migration 0038's rate_limit_windows_scope_is_known lists exactly these three"
        );
        for scope in scopes {
            assert!(
                !scope.is_empty() && scope.len() <= 64,
                "rate_limit_windows_scope_length is 1..=64: {scope}"
            );
        }
    }

    /// Two budgets never share a row, however the values are spelled.
    ///
    /// The scope is part of the pre-image, so one address used as an email
    /// key and as an address key is two rows. Without the scope prefix a
    /// deployment whose staff member's address happened to equal a source
    /// address string would have had one budget for both — a collision that
    /// is absurd for an email and not at all absurd once a third budget
    /// exists.
    #[test]
    fn a_scope_is_part_of_the_key() {
        let key = |budget: Budget, value: &str| {
            tokens::digest(&format!("{}:{value}", budget.as_wire_str()))
        };

        assert_ne!(
            key(Budget::SignInEmail, "ada@example.test"),
            key(Budget::SignInAddress, "ada@example.test"),
        );
        assert_ne!(
            key(Budget::SignInEmail, "ada@example.test"),
            key(Budget::ChangePasswordSession, "ada@example.test"),
        );
        assert_eq!(
            key(Budget::SignInEmail, "ada@example.test").len(),
            64,
            "rate_limit_windows_id_length is an equality CHECK on 64"
        );
    }
}
