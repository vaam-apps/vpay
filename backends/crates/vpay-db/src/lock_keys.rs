//! Every Postgres advisory-lock key this workspace takes, in one place.
//!
//! # Why they live together rather than beside their callers
//!
//! Postgres advisory locks share **one global namespace per database**
//! (`pg_locks.objid`): there is no schema, no owner and no type system
//! keeping two unrelated subjects apart. Two modules that each picked a
//! "distinct" constant in isolation would serialise against one another the
//! first time the two values happened to collide, and the symptom — one
//! boot step waiting on an unrelated one — is invisible in both call sites.
//! Listing them side by side is what makes "distinct" checkable by reading,
//! and [`ALL`] plus its test makes it checkable by the compiler's test
//! runner.
//!
//! # How a value is chosen
//!
//! Each is the ASCII bytes of a short mnemonic read as a big-endian integer,
//! so an operator who finds one in `pg_locks` can decode it (`printf '%x'`,
//! then `xxd -r -p`) and learn what is holding it. Seven bytes at most, which
//! keeps every value inside `bigint` and positive.
//!
//! **A value must never change.** It is not persisted anywhere, but a
//! deployment mid-rollout runs both the old and the new binary at once: if
//! one of them takes a different key for the same subject, the two stop
//! excluding each other for exactly the window the lock exists to cover.

/// `vpaykey` — taken by every writer in this crate's `signing_keys` module
/// (private; its entry point is [`crate::SigningKeys::ensure_active_signing_key`]) before
/// it reads or changes which OAuth signing key is active.
///
/// Why a lock at all, when the writes are already one transaction:
/// `ensure_active_signing_key` has to *read* which key is active and then
/// decide whether to write, and that read-then-write cannot be expressed as
/// a single compare-and-swap `UPDATE` — the operation is "retire the old row
/// **and** insert a new one", guarded on a row that may not exist at all on
/// the very first boot. Two replicas booting simultaneously with the same
/// PEM would both read "not active" and both try to insert the same `kid`;
/// one would take a duplicate-key error on what is supposed to be a no-op.
pub const SIGNING_KEY_ROTATION: i64 = 0x0076_7061_796b_6579;

/// `vpaycfg` — taken as the first statement of
/// [`crate::ConfigReconcile::reconcile`]'s transaction.
///
/// Boot step 4 runs in **both** binaries and in every replica of each, so a
/// rollout restarting four processes runs four reconciles against one
/// database within the same second. Without this lock those transactions
/// interleave, and two of them differ in two ways that matter:
///
/// * they upsert the same `providers` rows, in whatever order the YAML lists
///   them. Two configurations listing the rails in opposite orders take the
///   same row locks in opposite orders, which is a textbook deadlock — one
///   transaction is aborted by Postgres with `40P01` and the binary exits.
///   (`reconcile` also sorts its seeds, so the two guards are independent:
///   the sort removes the ordering hazard between *these* writers, the lock
///   removes it against any future writer of the same tables.)
/// * the disable pass (`UPDATE providers SET enabled = false WHERE code <>
///   ALL(...)`) decides what to disable from what it can *see*, so under
///   `READ COMMITTED` it can miss a rail another transaction is in the middle
///   of inserting, and the winner of the race decides whether a rail an
///   operator just removed is still enabled.
///
/// Distinct from [`SIGNING_KEY_ROTATION`] because the two steps are adjacent
/// in both binaries' boot sequences and sharing a key would make each wait
/// for the other's unrelated work.
pub const CONFIG_RECONCILE: i64 = 0x0076_7061_7963_6667;

/// Every key above, for the test that proves they are distinct.
///
/// A plain array rather than a derive: the property that matters is
/// "no two constants in this module are equal", and nothing in Rust
/// expresses that about `const`s. Adding a key here is the one step that
/// cannot be forgotten, because a key absent from this list is a key the
/// distinctness test does not cover — which is why the test also asserts the
/// length.
pub const ALL: [i64; 2] = [SIGNING_KEY_ROTATION, CONFIG_RECONCILE];

#[cfg(test)]
mod tests {
    use super::*;

    /// The one property this module exists to guarantee. Two subjects
    /// sharing a key would serialise unrelated work, and the symptom (a boot
    /// step blocking on another one) names neither of them.
    #[test]
    fn every_advisory_lock_key_is_distinct_and_positive() {
        let mut seen = std::collections::BTreeSet::new();
        for key in ALL {
            assert!(
                seen.insert(key),
                "advisory-lock key {key:#x} is used for two different subjects"
            );
            // Negative keys are legal in Postgres but render as a
            // two's-complement blob in `pg_locks`, which defeats the
            // decode-the-mnemonic property the module comment promises.
            assert!(key > 0, "advisory-lock key {key:#x} is not positive");
        }
        assert_eq!(seen.len(), ALL.len());
    }

    /// The mnemonics decode to the words the doc comments claim. Without
    /// this, a typo'd constant would still be distinct, still work, and
    /// still lie to the operator who decodes it out of `pg_locks`.
    #[test]
    fn each_key_decodes_to_its_documented_mnemonic() {
        for (key, mnemonic) in [
            (SIGNING_KEY_ROTATION, "vpaykey"),
            (CONFIG_RECONCILE, "vpaycfg"),
        ] {
            let bytes: Vec<u8> = key
                .to_be_bytes()
                .into_iter()
                .skip_while(|byte| *byte == 0)
                .collect();
            assert_eq!(
                String::from_utf8(bytes).expect("the mnemonic is ASCII"),
                mnemonic
            );
        }
    }
}
