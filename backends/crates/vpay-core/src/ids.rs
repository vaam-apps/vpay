//! The public ids vpay's objects are named by: `pi_…`, `ch_…`, `re_…`,
//! `evt_…`.
//!
//! An id is a *merchant-visible, permanent* name. It appears in URLs, in
//! logs, in support tickets, in a merchant's own database, and — because
//! `docs/api/README.md` promises Stripe's shape — in code merchants wrote
//! against Stripe. That fixes four properties, and each one is a test below:
//!
//! * **A prefix says what it names.** `pi_` on a charge id is a bug an
//!   operator can see at a glance instead of one they have to look up.
//! * **The body is `[a-z0-9]` only.** So the id survives a URL path segment,
//!   a query string, a form body, a shell argument and a filename unchanged
//!   — [`crate::ids`]'s own test proves `encodeURIComponent` (which is what
//!   both SDKs escape path segments with, `sdks/rust/src/form.rs`) is the
//!   identity on it. A `+` or a `/` from a base64 id would be re-encoded by
//!   one client and not another, and the two would then address different
//!   URLs.
//! * **It carries no information.** Not a sequence, not a timestamp, not a
//!   merchant id: an id that leaks how many payments a deployment has taken
//!   is a business fact given away to anyone holding one id, and a guessable
//!   id is an enumeration attack against a tenant-scoped API.
//! * **It fits.** 3 or 4 prefix characters plus 24 body characters is 27–28
//!   characters, comfortably inside the `CHECK (char_length(id) BETWEEN 1
//!   AND 64)` the schema puts on every id column, with room for a longer
//!   prefix later.
//!
//! # Why Crockford base32 and not hex or base62
//!
//! Hex would need 32 characters for the same entropy and reads as a hash,
//! which invites people to try to invert it. Base62 is mixed-case, and a
//! mixed-case id in a case-insensitive place (a Windows filename, an email
//! subject line someone lower-cased, a `LIKE` in a merchant's own database)
//! becomes two different ids. Crockford's alphabet is lower-cased here and
//! drops `i`, `l`, `o` and `u`, so an id read aloud or copied out of a
//! screenshot cannot become a *different valid-looking* id — which matters
//! because these end up in support tickets.
//!
//! The alphabet is Crockford's; the *encoding* deliberately is not. Crockford
//! specifies check symbols and case-insensitive decoding with `i`/`l` → `1`
//! and `o` → `0`; vpay decodes nothing (an id is an opaque key, looked up
//! whole) so none of that applies. Only the character set is borrowed.

use uuid::Uuid;

/// Crockford's base32 alphabet, lower-cased: the digits, then the letters
/// with `i`, `l`, `o` and `u` removed. Exactly 32 entries — pinned by
/// `the_alphabet_is_crockfords_and_every_five_bit_value_maps_into_it`.
const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// How many alphabet characters follow the prefix.
///
/// 24 x 5 = 120 bits, taken from the top 120 bits of a v4 UUID. Six of those
/// are the version and variant bits RFC 9562 fixes, so an id carries 114 bits
/// of randomness: a deployment would need on the order of 2^57 (≈1.4 x 10^17)
/// ids before a collision became an even bet, against a system that will
/// issue perhaps 10^9 in its life. The 8 dropped low bits are simply the
/// difference between 128 bits and a whole number of base32 characters; they
/// are dropped rather than folded in because a fold would be extra
/// arithmetic nobody could check by eye for zero practical benefit.
const BODY_CHARS: usize = 24;

/// How many bits one alphabet character carries.
const BITS_PER_CHAR: usize = 5;

/// The prefix on a PaymentIntent id.
///
/// Public because a *caller-supplied* id has to be checked against it before
/// it is used as a list cursor: an id the shape check would have caught
/// resolves to `NULL` in `vpay_db::payment_intents::list_page`'s cursor
/// subquery and comes back as an empty page, so without a boundary check a
/// typo pages a merchant into silence instead of into a `400`.
pub const PAYMENT_INTENT_PREFIX: &str = "pi_";

/// The prefix on a Charge id.
pub const CHARGE_PREFIX: &str = "ch_";

/// The prefix on a Refund id.
pub const REFUND_PREFIX: &str = "re_";

/// The prefix on an Event id.
pub const EVENT_PREFIX: &str = "evt_";

/// Whether `id` is shaped like an id this module would have minted under
/// `prefix`: the prefix, then exactly `BODY_CHARS` characters, every one
/// of them in the alphabet above.
///
/// A *shape* check and deliberately nothing more. It says nothing about
/// whether the object exists, belongs to the caller, or ever existed — the
/// merchant-scoped query is what answers that, and it must stay the only
/// thing that does, or this function becomes an existence oracle. What it is
/// for is the case where a malformed id would otherwise produce a *silent*
/// wrong answer rather than an error (see [`PAYMENT_INTENT_PREFIX`]).
///
/// Case-sensitive, because the ids are: the alphabet is lower-cased
/// (see this module's docs) and an uppercase copy of a real id is not that
/// id anywhere else in the system either, so accepting it here would be the
/// one place that disagreed.
#[must_use]
pub fn is_well_formed(prefix: &str, id: &str) -> bool {
    let Some(body) = id.strip_prefix(prefix) else {
        return false;
    };
    // `len()` is a byte count, and a multi-byte character would fail the
    // alphabet test below in any case — the two together mean exactly
    // `BODY_CHARS` ASCII alphabet characters.
    body.len() == BODY_CHARS && body.bytes().all(|byte| ALPHABET.contains(&byte))
}

/// Builds one id: `prefix` followed by `BODY_CHARS` alphabet characters.
///
/// Private because the prefix vocabulary is closed — the four functions below
/// are the whole of it. A `pub fn new_id(prefix: &str)` would let a call site
/// invent a fifth prefix (or misspell one of these) without review noticing,
/// and the prefix is the part an operator reads.
fn new_id(prefix: &str) -> String {
    // `new_v4` draws from the OS CSPRNG (`getrandom`), which is the property
    // that matters here: a v7 UUID would embed a timestamp, and a timestamp
    // in a public id tells a holder when the object was created and roughly
    // how busy the deployment was — see the module doc.
    let bits = Uuid::new_v4().as_u128() >> (128 - BODY_CHARS * BITS_PER_CHAR);

    let mut id = String::with_capacity(prefix.len() + BODY_CHARS);
    id.push_str(prefix);
    for position in (0..BODY_CHARS).rev() {
        let index = ((bits >> (position * BITS_PER_CHAR)) & 0x1f) as usize;
        // `index` is five bits masked, so it is 0..=31 and `ALPHABET` has 32
        // entries — the `.get()` cannot be `None`. It is written as a total
        // expression rather than an index (`clippy::indexing_slicing`) or an
        // `expect` (ADR-0007 denies panics), and the unreachable branch is
        // *proved* unreachable by the alphabet-length test below rather than
        // merely asserted here.
        let digit = ALPHABET.get(index).copied().unwrap_or(b'0');
        id.push(char::from(digit));
    }
    id
}

/// A new PaymentIntent id, `pi_…`.
#[must_use]
pub fn payment_intent_id() -> String {
    new_id(PAYMENT_INTENT_PREFIX)
}

/// A new Charge id, `ch_…`.
///
/// A charge is not a merchant-facing object in this API — one charge per
/// intent, forever, and merchants address the intent — but it is the row an
/// operator traces a rail submission through, so it gets a real id rather
/// than a bare UUID for the same readability reasons.
#[must_use]
pub fn charge_id() -> String {
    new_id(CHARGE_PREFIX)
}

/// A new Refund id, `re_…`.
#[must_use]
pub fn refund_id() -> String {
    new_id(REFUND_PREFIX)
}

/// A new Event id, `evt_…`.
///
/// Webhook delivery is at-least-once and merchants are told to dedupe on this
/// value (`docs/flows/webhooks.md`), so it must be unique per *event*, never
/// per delivery attempt.
#[must_use]
pub fn event_id() -> String {
    new_id(EVENT_PREFIX)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// One id generator and the prefix it owns.
    ///
    /// A named alias only so `GENERATORS` below is not a `type_complexity`
    /// denial under the workspace's `-D warnings`; the tuple is the whole
    /// meaning.
    type Generator = (fn() -> String, &'static str);

    /// Every generator, with the prefix it owns.
    const GENERATORS: [Generator; 4] = [
        (payment_intent_id as fn() -> String, PAYMENT_INTENT_PREFIX),
        (charge_id, CHARGE_PREFIX),
        (refund_id, REFUND_PREFIX),
        (event_id, EVENT_PREFIX),
    ];

    /// `sdks/rust/src/form.rs`'s `is_safe_byte`, copied verbatim rather than
    /// imported: `vpay-core` must not depend on the merchant SDK, and the
    /// point of the test below is precisely that these two files agree. A
    /// divergence here shows up as a failing assertion in whichever
    /// repository half changed.
    fn is_safe_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
    }

    /// JavaScript's `encodeURIComponent`, as both SDKs implement it.
    fn percent_encode(input: &str) -> String {
        input
            .bytes()
            .map(|b| {
                if is_safe_byte(b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    }

    #[test]
    fn the_alphabet_is_crockfords_and_every_five_bit_value_maps_into_it() {
        assert_eq!(ALPHABET.len(), 1 << BITS_PER_CHAR);
        let unique: HashSet<u8> = ALPHABET.iter().copied().collect();
        assert_eq!(unique.len(), ALPHABET.len(), "a repeated digit");
        for b in ALPHABET {
            assert!(
                b.is_ascii_lowercase() || b.is_ascii_digit(),
                "{}: outside [a-z0-9]",
                char::from(*b)
            );
        }
        // The four Crockford excludes. `i`/`l` are `1`, `o` is `0`, and `u`
        // is dropped so no id can spell an unfortunate word by accident.
        for excluded in [b'i', b'l', b'o', b'u'] {
            assert!(
                !ALPHABET.contains(&excluded),
                "{} must not be in the alphabet",
                char::from(excluded)
            );
        }
        // This is what makes `new_id`'s `.get(..).unwrap_or(b'0')` a branch
        // that cannot be taken: the mask yields 0..=31 and every one of those
        // indexes a real digit.
        for index in 0..(1usize << BITS_PER_CHAR) {
            assert!(ALPHABET.get(index).is_some(), "no digit for {index}");
        }
    }

    #[test]
    fn every_id_is_its_prefix_plus_twenty_four_alphabet_characters() {
        for (generate, prefix) in GENERATORS {
            let id = generate();
            assert!(id.starts_with(prefix), "{id} is not a {prefix}id");
            assert_eq!(
                id.len(),
                prefix.len() + BODY_CHARS,
                "{id} is the wrong length"
            );
            // ASCII throughout, so `len()` above is also the character count
            // and the `char_length` CHECK in Postgres sees the same number.
            assert!(id.is_ascii(), "{id} is not ASCII");
            assert!(
                id.len() <= 64,
                "{id} does not fit the id columns' CHECK (1..64)"
            );

            let body = id.strip_prefix(prefix).expect("just checked the prefix");
            for c in body.chars() {
                assert!(
                    c.is_ascii_lowercase() || c.is_ascii_digit(),
                    "{id}: {c:?} is outside [a-z0-9]"
                );
            }
        }
    }

    /// The property the module doc rests on: an id escaped by either SDK is
    /// the id. If a future encoding introduced `+`, `/`, `=` or an uppercase
    /// letter, this fails — and it fails *here*, rather than as two clients
    /// addressing different URLs for the same object.
    #[test]
    fn percent_encoding_an_id_is_the_identity() {
        for (generate, _) in GENERATORS {
            for _ in 0..64 {
                let id = generate();
                assert_eq!(percent_encode(&id), id, "{id} does not survive escaping");
            }
        }
        // The mirror of `is_safe_byte` is only meaningful if it actually
        // escapes something: without this, a `percent_encode` that returned
        // its input unchanged would pass the assertions above.
        assert_eq!(percent_encode("a/b+c=d"), "a%2Fb%2Bc%3Dd");
    }

    #[test]
    fn ten_thousand_ids_are_ten_thousand_distinct_ids() {
        const N: usize = 10_000;
        let ids: HashSet<String> = (0..N).map(|_| payment_intent_id()).collect();
        assert_eq!(ids.len(), N, "a collision in {N} ids");

        // And the four generators do not collide with each other either —
        // the prefix is part of the id, not decoration on a shared body.
        let mixed: HashSet<String> = GENERATORS
            .iter()
            .flat_map(|(generate, _)| (0..1_000).map(move |_| generate()))
            .collect();
        assert_eq!(mixed.len(), GENERATORS.len() * 1_000);
    }

    /// [`is_well_formed`] accepts exactly what the generators produce, and
    /// the near-misses a merchant actually sends are the ones it has to
    /// refuse: a cursor from a different resource (`ch_…` where `pi_…` was
    /// meant), a truncated copy/paste, and the four letters the alphabet
    /// deliberately drops — `i`, `l`, `o`, `u` are exactly what a human
    /// re-typing an id off a screenshot substitutes for `1`, `1`, `0` and
    /// nothing, so they are the misreads most likely to arrive.
    #[test]
    fn is_well_formed_accepts_what_the_generators_mint_and_nothing_close_to_it() {
        for (generate, prefix) in GENERATORS {
            for _ in 0..64 {
                let id = generate();
                assert!(is_well_formed(prefix, &id), "{id} is a real {prefix}id");
            }
        }

        let id = payment_intent_id();
        // A real id, checked against another resource's prefix.
        assert!(!is_well_formed(CHARGE_PREFIX, &id));
        // The prefix alone, and the body alone.
        assert!(!is_well_formed(
            PAYMENT_INTENT_PREFIX,
            PAYMENT_INTENT_PREFIX
        ));
        assert!(!is_well_formed(
            PAYMENT_INTENT_PREFIX,
            id.trim_start_matches(PAYMENT_INTENT_PREFIX)
        ));
        // One character short, and one long.
        let short: String = id.chars().take(id.len() - 1).collect();
        assert!(!is_well_formed(PAYMENT_INTENT_PREFIX, &short));
        assert!(!is_well_formed(PAYMENT_INTENT_PREFIX, &format!("{id}0")));
        // Right length, wrong alphabet.
        for excluded in ['i', 'l', 'o', 'u', 'A', '-', '/'] {
            let mut body: String = id
                .trim_start_matches(PAYMENT_INTENT_PREFIX)
                .chars()
                .skip(1)
                .collect();
            body.insert(0, excluded);
            assert!(
                !is_well_formed(
                    PAYMENT_INTENT_PREFIX,
                    &format!("{PAYMENT_INTENT_PREFIX}{body}")
                ),
                "{excluded:?} is not in the alphabet"
            );
        }
        // Multi-byte: 24 *characters*, but not 24 alphabet bytes.
        assert!(!is_well_formed(
            PAYMENT_INTENT_PREFIX,
            &format!("{PAYMENT_INTENT_PREFIX}{}", "é".repeat(BODY_CHARS))
        ));
        assert!(!is_well_formed(PAYMENT_INTENT_PREFIX, ""));
    }

    /// A weak generator that returned a constant, a counter, or the same
    /// leading bytes every time would pass "is unique" only by luck. This
    /// asserts every one of the 24 positions actually varies, which is what
    /// catches a shift/mask bug that zeroed part of the body.
    #[test]
    fn every_character_position_varies() {
        let ids: Vec<String> = (0..512).map(|_| payment_intent_id()).collect();
        for position in 0..BODY_CHARS {
            let seen: HashSet<Option<char>> = ids
                .iter()
                .map(|id| id.chars().nth(PAYMENT_INTENT_PREFIX.len() + position))
                .collect();
            assert!(
                seen.len() > 1,
                "position {position} is constant across 512 ids"
            );
        }
    }
}
