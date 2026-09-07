//! argon2id with a deployment pepper — the first of a staff member's two
//! factors (ADR-0017 decision 1).
//!
//! # The parameters, and where they come from
//!
//! [`MEMORY_KIB`], [`ITERATIONS`] and [`PARALLELISM`] are OWASP's *minimum*
//! recommendation for Argon2id (19 MiB, t=2, p=1) — the first configuration
//! in the Password Storage Cheat Sheet's list, chosen because it is the one
//! with the smallest memory footprint that OWASP still calls adequate, and
//! this process shares its memory with a payment API.
//!
//! They are **not configurable**. A per-deployment cost is one more thing
//! that can differ between the sandbox a merchant integrates against and the
//! production they go live on (ADR-0003's rule, applied to a cost rather than
//! to a code path) — and the PHC string a hash is stored as carries its own
//! parameters, so raising these later re-verifies every existing row without
//! a migration.
//!
//! # The pepper is not a salt
//!
//! Each hash has its own random salt, inside the PHC string, in the database.
//! The pepper is one deployment-wide value that is **not** in the database
//! (`vpay_config::StaffAuth::password_pepper`, from a Secret), mixed in as
//! argon2's `secret` input. A stolen `staff_members` table is therefore not by itself
//! an offline cracking target: an attacker needs the table *and* the Secret.
//!
//! The cost is stated plainly in ADR-0017's Consequences and in migration
//! 0035's own comment: **losing the pepper invalidates every stored hash.**

use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

use super::{StaffAuthError, StaffCredentials};

/// Memory cost, in kibibytes. 19 MiB — OWASP's first listed Argon2id
/// configuration.
const MEMORY_KIB: u32 = 19_456;

/// Time cost. Two passes, with [`MEMORY_KIB`].
const ITERATIONS: u32 = 2;

/// Lanes. One, per the same OWASP row.
const PARALLELISM: u32 = 1;

/// A PHC string for a password nobody has.
///
/// [`StaffCredentials::verify_password`] runs a real verification against
/// this when no account matched, so that "no such address" and "wrong
/// password" take the same code path and cost the same time. Without it,
/// vpay's login endpoint would be an account-enumeration oracle measurable
/// with a stopwatch: an unknown address would return in microseconds and a
/// known one in the ~50 ms an argon2id verification takes.
///
/// It is a **real hash of a random value**, generated once and pasted here,
/// not a hand-written string: `PasswordHash::new` parses it, and a malformed
/// one would make the dummy path return an error instead of `false` — which
/// would restore the very difference it exists to remove. Its plaintext was
/// never recorded and does not exist anywhere.
///
/// Not peppered, and it does not need to be: nothing is ever compared
/// *successfully* against it. What matters is that verifying against it costs
/// the same as verifying against a real row, and the cost is the parameters,
/// which are the same.
const ABSENT_ACCOUNT_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1\
    $Xm9OTFJ2WHNQVFJvSVptZQ$1YOgL2LZ5uwGiIQZfNvJUgFYr3zEJTaVQ2qgBGWfL7A";

impl StaffCredentials {
    /// The argon2id hasher this deployment uses, peppered.
    ///
    /// Built per call rather than held on the struct: `Argon2<'key>` borrows
    /// its secret, so storing one would make [`StaffCredentials`]
    /// self-referential. Construction is a handful of integer moves — the
    /// cost of argon2 is in the hashing, not in the parameter block.
    fn hasher(&self) -> Result<Argon2<'_>, StaffAuthError> {
        let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, None)
            .map_err(|error| StaffAuthError::Password(error.to_string()))?;
        Argon2::new_with_secret(self.pepper(), Algorithm::Argon2id, Version::V0x13, params)
            .map_err(|error| StaffAuthError::Password(error.to_string()))
    }

    /// Hashes a password for storage, with a fresh random salt.
    ///
    /// # Errors
    ///
    /// [`StaffAuthError::Password`] if argon2 refuses the parameters or the
    /// hash. Neither carries the password: `argon2::Error` and
    /// `password_hash::Error` are both parameter-level types.
    pub fn hash_password(&self, password: &str) -> Result<String, StaffAuthError> {
        let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
        let hash = self
            .hasher()?
            .hash_password(password.as_bytes(), &salt)
            .map_err(|error| StaffAuthError::Password(error.to_string()))?;
        Ok(hash.to_string())
    }

    /// Whether `password` matches `stored`.
    ///
    /// `Ok(false)` for a wrong password **and** for a stored hash that will
    /// not parse. The second is a judgement call and it is the safe one: a
    /// row whose PHC string is corrupt is a row nobody can authenticate
    /// against, and answering `Err` would turn it into a `500` that tells an
    /// attacker they have found an account whose hash is unusual.
    ///
    /// # Errors
    ///
    /// [`StaffAuthError::Password`] only if the *parameters* are impossible,
    /// which is a deployment fault and cannot depend on the input.
    pub fn verify_password(
        &self,
        password: &str,
        stored: &str,
    ) -> Result<bool, StaffAuthError> {
        let hasher = self.hasher()?;
        let Ok(parsed) = PasswordHash::new(stored) else {
            return Ok(false);
        };
        Ok(hasher.verify_password(password.as_bytes(), &parsed).is_ok())
    }

    /// Burns the same work a real verification would, and answers `false`.
    ///
    /// The login handler calls this when no account matched, so that the two
    /// outcomes are one code path. It is a method rather than a bare `sleep`
    /// because the *cost* has to track the parameters: raising [`MEMORY_KIB`]
    /// later must slow this down by exactly as much as it slows a real
    /// verification, and only doing the same work can guarantee that.
    ///
    /// # Errors
    ///
    /// [`StaffAuthError::Password`] if the parameters are impossible — the
    /// same deployment fault [`Self::verify_password`] raises, and it must be
    /// raised here too or the two paths would differ in exactly the case an
    /// operator most needs to hear about.
    pub fn verify_absent_account(&self, password: &str) -> Result<bool, StaffAuthError> {
        let matched = self.verify_password(password, ABSENT_ACCOUNT_HASH)?;
        debug_assert!(
            !matched,
            "ABSENT_ACCOUNT_HASH must not be a hash of anything a caller can type"
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> StaffCredentials {
        // A 32-byte key of zeros: this module never touches it, and
        // `secret_box`'s own tests are where key material matters.
        StaffCredentials::new("a-test-pepper", &"A".repeat(43)).expect("43 base64url chars")
    }

    #[test]
    fn a_password_verifies_against_its_own_hash_and_nothing_else() {
        let credentials = credentials();
        let stored = credentials
            .hash_password("correct horse battery staple")
            .expect("hashing succeeds");

        assert!(
            credentials
                .verify_password("correct horse battery staple", &stored)
                .expect("verification runs")
        );
        assert!(
            !credentials
                .verify_password("correct horse battery stapl", &stored)
                .expect("verification runs"),
            "one character off is a different password"
        );
        assert!(
            !credentials
                .verify_password("", &stored)
                .expect("verification runs")
        );
    }

    /// The pepper is part of the credential, not decoration: a hash written
    /// under one pepper must not verify under another. This is the assertion
    /// that fails if `new_with_secret` is ever replaced by `Argon2::default`
    /// — which would compile, produce valid hashes, and silently remove the
    /// whole defence.
    #[test]
    fn a_hash_written_under_one_pepper_does_not_verify_under_another() {
        let one = StaffCredentials::new("pepper-one", &"A".repeat(43)).expect("key");
        let other = StaffCredentials::new("pepper-two", &"A".repeat(43)).expect("key");

        let stored = one.hash_password("hunter2").expect("hashing succeeds");

        assert!(one.verify_password("hunter2", &stored).expect("runs"));
        assert!(
            !other.verify_password("hunter2", &stored).expect("runs"),
            "the pepper is mixed in as argon2's secret input; losing it invalidates every hash"
        );
    }

    /// Two hashes of the same password differ, because each carries its own
    /// salt. Without this, a stolen table would say which staff members chose
    /// the same password.
    #[test]
    fn two_hashes_of_one_password_are_not_equal() {
        let credentials = credentials();
        let first = credentials.hash_password("hunter2").expect("runs");
        let second = credentials.hash_password("hunter2").expect("runs");
        assert_ne!(first, second);
    }

    /// The stored hash announces the parameters it was written under, which
    /// is what lets them be raised later without a migration.
    #[test]
    fn the_stored_hash_names_the_owasp_parameters() {
        let stored = credentials().hash_password("hunter2").expect("runs");
        assert!(stored.starts_with("$argon2id$v=19$"), "{stored}");
        assert!(
            stored.contains(&format!("m={MEMORY_KIB},t={ITERATIONS},p={PARALLELISM}")),
            "{stored}"
        );
    }

    /// A corrupt stored hash is `false`, never an error — see the method's
    /// doc for why the difference would be an oracle.
    #[test]
    fn an_unparseable_stored_hash_is_a_refusal_and_not_an_error() {
        let credentials = credentials();
        for stored in ["", "not-a-phc-string", "$argon2id$", "$bcrypt$whatever"] {
            assert!(
                !credentials
                    .verify_password("hunter2", stored)
                    .expect("a corrupt row must not raise"),
                "{stored}"
            );
        }
    }

    /// The dummy hash has to be a *parseable* one, or the no-such-account
    /// path would return early and restore the timing difference it exists to
    /// remove. This is the assertion that catches a typo in the constant.
    #[test]
    fn the_absent_account_hash_parses_and_never_matches() {
        PasswordHash::new(ABSENT_ACCOUNT_HASH)
            .expect("ABSENT_ACCOUNT_HASH must parse, or the no-such-account path returns early");

        let credentials = credentials();
        for attempt in ["", "hunter2", "password", "correct horse battery staple"] {
            assert!(
                !credentials
                    .verify_absent_account(attempt)
                    .expect("the dummy path runs the same work"),
                "{attempt}"
            );
        }
    }
}
