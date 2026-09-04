//! The public ids vpay's objects are named by — `pi_…`, `ch_…`, `re_…`,
//! `evt_…`, `cs_…` — and the two payer credentials that ride in URLs: the
//! `client_secret` a browser presents, and the `return_token` a redirect
//! rail's bounce carries back.
//!
//! An id is a *merchant-visible, permanent* name: it carries a prefix that
//! says what it names, a body of `[a-z0-9]` only, no information about the
//! deployment that minted it, and a length every id column's
//! `CHECK (char_length(id) BETWEEN 1 AND 64)` accepts.
//!
//! Why each of those four properties, why Crockford's alphabet rather than
//! hex or base62, and where a client secret's entropy comes from:
//! [docs/reference/vpay-core.md § ids](../../../../docs/reference/vpay-core.md#ids).

use uuid::Uuid;

/// Crockford's base32 alphabet, lower-cased: the digits, then the letters
/// with `i`, `l`, `o` and `u` removed. Exactly 32 entries — pinned by
/// `the_alphabet_is_crockfords_and_every_five_bit_value_maps_into_it`.
const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// How many alphabet characters follow the prefix: 24, for 114 bits of
/// randomness out of a v4 UUID's top 120.
const BODY_CHARS: usize = 24;

/// How many bits one alphabet character carries.
const BITS_PER_CHAR: usize = 5;

/// The prefix on a PaymentIntent id.
///
/// Public because a *caller-supplied* id has to be checked against it before
/// it is used as a list cursor: an id the shape check would have caught pages
/// a merchant into silence rather than into a `400`. See
/// [`is_well_formed`].
pub const PAYMENT_INTENT_PREFIX: &str = "pi_";

/// The prefix on a Charge id.
pub const CHARGE_PREFIX: &str = "ch_";

/// The prefix on a Refund id.
pub const REFUND_PREFIX: &str = "re_";

/// The prefix on an Event id.
pub const EVENT_PREFIX: &str = "evt_";

/// The prefix on a Checkout Session id.
///
/// Stripe's own spelling for the same object, and that matters more here than
/// it does for `pi_`/`ch_`: this prefix is the leading characters of a
/// `client_secret` a merchant pastes into `initEmbeddedCheckout`, so a
/// merchant who has integrated Stripe once recognises at a glance which of
/// the two credentials on their page they are holding.
pub const CHECKOUT_SESSION_PREFIX: &str = "cs_";

/// Whether `id` is shaped like an id this module would have minted under
/// `prefix`: the prefix, then exactly 24 characters, every one of them in the
/// alphabet.
///
/// A *shape* check and deliberately nothing more — it must never become an
/// existence oracle, and it is case-sensitive because the ids are. Both
/// reasons, and the silent-wrong-answer case it exists for, are in
/// [docs/reference/vpay-core.md § ids](../../../../docs/reference/vpay-core.md#is_well_formed-is-a-shape-check-and-not-an-existence-oracle).
///
/// ```
/// use vpay_core::ids::{self, CHARGE_PREFIX, PAYMENT_INTENT_PREFIX};
///
/// let id = ids::payment_intent_id();
/// assert!(ids::is_well_formed(PAYMENT_INTENT_PREFIX, &id));
///
/// // Another resource's prefix, a truncated copy, and the letters the
/// // alphabet deliberately drops are all refused.
/// assert!(!ids::is_well_formed(CHARGE_PREFIX, &id));
/// assert!(!ids::is_well_formed(PAYMENT_INTENT_PREFIX, &id[..id.len() - 1]));
/// assert!(!ids::is_well_formed(
///     PAYMENT_INTENT_PREFIX,
///     "pi_iiiiiiiiiiiiiiiiiiiiiiii"
/// ));
/// ```
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
/// Private because the prefix vocabulary is closed — the five functions below
/// are the whole of it, and the prefix is the part an operator reads.
fn new_id(prefix: &str) -> String {
    // `new_v4` draws from the OS CSPRNG (`getrandom`). A v7 UUID would embed a
    // timestamp, and a timestamp in a public id tells a holder when the object
    // was created and roughly how busy the deployment was.
    let bits = Uuid::new_v4().as_u128() >> (128 - BODY_CHARS * BITS_PER_CHAR);

    let mut id = String::with_capacity(prefix.len() + BODY_CHARS);
    id.push_str(prefix);
    push_base32(&mut id, bits, BODY_CHARS);
    id
}

/// Appends the low `chars * BITS_PER_CHAR` bits of `value` to `out` as base32
/// digits, most significant first.
///
/// Shared by [`new_id`] and [`secret_body`] so an id, a client secret and a
/// return token cannot disagree about which characters are legal.
fn push_base32(out: &mut String, value: u128, chars: usize) {
    for position in (0..chars).rev() {
        let index = ((value >> (position * BITS_PER_CHAR)) & 0x1f) as usize;
        // `index` is five bits masked, so it is 0..=31 and `ALPHABET` has 32
        // entries — the `.get()` cannot be `None`. Written as a total
        // expression rather than an index or an `expect` (ADR-0007), and the
        // unreachable branch is *proved* unreachable by the alphabet-length
        // test below rather than merely asserted here.
        let digit = ALPHABET.get(index).copied().unwrap_or(b'0');
        out.push(char::from(digit));
    }
}

/// A new PaymentIntent id, `pi_…`.
///
/// ```
/// use vpay_core::ids::{self, PAYMENT_INTENT_PREFIX};
///
/// let id = ids::payment_intent_id();
/// assert!(id.starts_with(PAYMENT_INTENT_PREFIX));
/// assert_eq!(id.len(), PAYMENT_INTENT_PREFIX.len() + 24);
/// assert_ne!(id, ids::payment_intent_id());
/// ```
#[must_use]
pub fn payment_intent_id() -> String {
    new_id(PAYMENT_INTENT_PREFIX)
}

/// A new Charge id, `ch_…`.
///
/// A charge is not a merchant-facing object in this API — one charge per
/// intent, forever, and merchants address the intent — but it is the row an
/// operator traces a rail submission through, so it gets a real id rather
/// than a bare UUID.
///
/// ```
/// use vpay_core::ids::{self, CHARGE_PREFIX};
///
/// assert!(ids::is_well_formed(CHARGE_PREFIX, &ids::charge_id()));
/// ```
#[must_use]
pub fn charge_id() -> String {
    new_id(CHARGE_PREFIX)
}

/// A new Refund id, `re_…`.
///
/// ```
/// use vpay_core::ids::{self, REFUND_PREFIX};
///
/// assert!(ids::is_well_formed(REFUND_PREFIX, &ids::refund_id()));
/// ```
#[must_use]
pub fn refund_id() -> String {
    new_id(REFUND_PREFIX)
}

/// A new Event id, `evt_…`.
///
/// Webhook delivery is at-least-once and merchants are told to dedupe on this
/// value (`docs/flows/webhooks.md`), so it must be unique per *event*, never
/// per delivery attempt.
///
/// ```
/// use vpay_core::ids::{self, EVENT_PREFIX};
///
/// assert!(ids::is_well_formed(EVENT_PREFIX, &ids::event_id()));
/// assert_ne!(ids::event_id(), ids::event_id());
/// ```
#[must_use]
pub fn event_id() -> String {
    new_id(EVENT_PREFIX)
}

/// A new Checkout Session id, `cs_…`.
///
/// ```
/// use vpay_core::ids::{self, CHECKOUT_SESSION_PREFIX, PAYMENT_INTENT_PREFIX};
///
/// let id = ids::checkout_session_id();
/// assert!(ids::is_well_formed(CHECKOUT_SESSION_PREFIX, &id));
/// // A session id is not an intent id, and the prefix is what says so —
/// // `POST /v1/checkout/sessions` takes both and must never confuse them.
/// assert!(!ids::is_well_formed(PAYMENT_INTENT_PREFIX, &id));
/// ```
#[must_use]
pub fn checkout_session_id() -> String {
    new_id(CHECKOUT_SESSION_PREFIX)
}

/// What joins an object id to its secret suffix: `pi_…` + this + the suffix.
///
/// Public because it is a **wire contract**: `@vpay/stripe-js` splits a
/// `clientSecret` on this exact string (`sdks/stripe-js/src/client.ts`'s
/// `SECRET_SEPARATOR`), and Stripe spells its own client secrets the same
/// way. Spelled once here so the minting side and any Rust-side parser cannot
/// drift the way two literals would.
pub const CLIENT_SECRET_INFIX: &str = "_secret_";

/// How many alphabet characters a client-secret suffix carries: 32, for 160
/// bits of which 148 are unpredictable.
const CLIENT_SECRET_SUFFIX_CHARS: usize = 32;

/// Thirty-two alphabet characters of OS-CSPRNG randomness — the body behind
/// both [`client_secret_suffix`] and [`return_token`].
///
/// Private, and the two public functions are deliberately *not* aliases of
/// each other: they are two different capabilities over two different
/// columns, and a call site that spelled `client_secret_suffix()` while
/// minting a return token would read as if the two were interchangeable —
/// which is precisely what D6 says they must not be. Sharing the body is what
/// keeps the *entropy* one decision; keeping the names apart is what keeps
/// the *authority* two.
fn secret_body() -> String {
    // Halves rather than 32 characters from one draw plus 0 from the other:
    // an even split is the shape that makes "two independent CSPRNG draws"
    // true of the whole string rather than of a prefix of it.
    const HALF: usize = CLIENT_SECRET_SUFFIX_CHARS / 2;
    let mut body = String::with_capacity(CLIENT_SECRET_SUFFIX_CHARS);
    for _ in 0..2 {
        let bits = Uuid::new_v4().as_u128() >> (128 - HALF * BITS_PER_CHAR);
        push_base32(&mut body, bits, HALF);
    }
    body
}

/// A fresh client-secret suffix — the half of a `client_secret` that is stored
/// (`payment_intents.client_secret_suffix`, migration `0026`;
/// `checkout_sessions.client_secret_suffix`, migration `0028`).
///
/// Why two UUID draws, and why 160 bits rather than an id's 120:
/// [docs/reference/vpay-core.md § client secrets](../../../../docs/reference/vpay-core.md#client-secrets).
///
/// ```
/// let suffix = vpay_core::ids::client_secret_suffix();
///
/// // Migration 0026's `client_secret_suffix_length` CHECK is `BETWEEN 32 AND
/// // 128`; this is at its lower bound. 0028's is the identical CHECK, so one
/// // generator serves both tables.
/// assert_eq!(suffix.len(), 32);
/// // Same alphabet as an id, so a `client_secret` survives a query string.
/// assert!(
///     suffix
///         .chars()
///         .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
/// );
/// ```
#[must_use]
pub fn client_secret_suffix() -> String {
    secret_body()
}

/// A fresh `checkout_sessions.return_token` (migration `0028`) — the
/// credential a redirect rail's bounce carries back in a **query string**.
///
/// # Why this is not the session's `client_secret`
///
/// D6 of `docs/plans/2026-09-04-step9-hosted-checkout.md`. Every secret on a
/// vpay-served page rides in a URL *fragment*, which never leaves the
/// browser — but a fragment does not survive a rail's redirect, so the page a
/// payer lands on after Orange's own checkout has no way to be handed one.
/// Its credential therefore has to be a query parameter, and a query
/// parameter is written to access logs, kept in browser history and sent as a
/// `Referer` by some clients.
///
/// So the value that travels there authorises strictly less: reading the
/// session and its intent *without* the intent's `client_secret`, which is
/// enough to render an outcome and forward the payer and is not enough to
/// confirm anything. Same 160 bits, same alphabet, same constant-time
/// compare, same uniform 404 — a *smaller capability*, not a weaker secret.
///
/// ```
/// use vpay_core::ids;
///
/// let token = ids::return_token();
///
/// // Migration 0028's `return_token_length` CHECK is `BETWEEN 32 AND 128`.
/// assert_eq!(token.len(), 32);
/// // URL-safe by construction: it is a query parameter on the return page.
/// assert!(
///     token
///         .chars()
///         .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
/// );
/// assert_ne!(token, ids::return_token());
/// ```
#[must_use]
pub fn return_token() -> String {
    secret_body()
}

/// Joins an object id and its stored suffix into the credential a payer's
/// browser presents: `pi_…_secret_…`.
///
/// **The one place the two halves are joined**, on the minting side and the
/// checking side both — `vpay_api::browser::authenticate` rebuilds the
/// expected secret with this function rather than parsing what the caller
/// sent, so "which secret did we actually compare?" has one answer.
///
/// ```
/// use vpay_core::ids::{CLIENT_SECRET_INFIX, client_secret};
///
/// let id = "pi_0123456789abcdefghjkmnpq";
/// let secret = client_secret(id, "wxyz0123456789abcdefghjkmnpqrstv");
/// assert_eq!(
///     secret,
///     "pi_0123456789abcdefghjkmnpq_secret_wxyz0123456789abcdefghjkmnpqrstv"
/// );
///
/// // What `@vpay/stripe-js` does to recover the id it builds a URL from.
/// assert_eq!(secret.split_once(CLIENT_SECRET_INFIX), Some((id, "wxyz0123456789abcdefghjkmnpqrstv")));
/// ```
#[must_use]
pub fn client_secret(id: &str, suffix: &str) -> String {
    format!("{id}{CLIENT_SECRET_INFIX}{suffix}")
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
    ///
    /// A table rather than five separate tests, so a generator added without
    /// being listed here is a generator none of the properties below hold of
    /// — the length, the alphabet, the id-column CHECK and the
    /// percent-encoding identity are claims about *every* id vpay mints.
    const GENERATORS: [Generator; 5] = [
        (payment_intent_id as fn() -> String, PAYMENT_INTENT_PREFIX),
        (charge_id, CHARGE_PREFIX),
        (refund_id, REFUND_PREFIX),
        (event_id, EVENT_PREFIX),
        (checkout_session_id, CHECKOUT_SESSION_PREFIX),
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

    /// The suffix is exactly what migration `0026`'s
    /// `client_secret_suffix_length` CHECK accepts, and exactly what the
    /// module's URL-safety property covers.
    ///
    /// The length is written as a literal rather than derived from
    /// `CLIENT_SECRET_SUFFIX_CHARS`, so shrinking that constant is a
    /// deliberate change to this test too and not something that slips
    /// through green — the constant *is* the entropy budget.
    #[test]
    fn a_client_secret_suffix_is_thirty_two_alphabet_characters() {
        for _ in 0..64 {
            let suffix = client_secret_suffix();
            assert_eq!(
                suffix.len(),
                32,
                "client_secret_suffix() must be exactly 32 bytes long"
            );
            assert!(suffix.is_ascii(), "client_secret_suffix() must be ASCII");
            for c in suffix.chars() {
                assert!(
                    c.is_ascii_lowercase() || c.is_ascii_digit(),
                    "client_secret_suffix() produced {c:?}, outside the [a-z0-9] alphabet"
                );
            }
            // The database CHECK, restated: 32 is inside `BETWEEN 32 AND 128`
            // at its lower bound, which is where an off-by-one would land.
            assert!((32..=128).contains(&suffix.chars().count()));
        }
    }

    /// The credential is guessed, or it is not: a suffix that repeated, or
    /// whose second half were a copy of its first, would be a live payment
    /// intent anyone holding one secret could reach.
    ///
    /// Every character position is checked to vary for
    /// [`every_character_position_varies`]'s reason — a shift/mask bug in the
    /// *second* draw would leave the tail constant while every uniqueness
    /// assertion still passed on the strength of the first.
    #[test]
    fn client_secret_suffixes_are_distinct_in_every_position() {
        const N: usize = 4_096;
        let suffixes: Vec<String> = (0..N).map(|_| client_secret_suffix()).collect();
        let unique: HashSet<&String> = suffixes.iter().collect();
        assert_eq!(unique.len(), N, "a collision in {N} client secret suffixes");

        for position in 0..32 {
            let seen: HashSet<Option<char>> =
                suffixes.iter().map(|s| s.chars().nth(position)).collect();
            assert!(
                seen.len() > 1,
                "position {position} is constant across {N} suffixes"
            );
        }
    }

    /// The joined credential, and the property `@vpay/stripe-js` relies on:
    /// splitting on the **first** `_secret_` recovers the id.
    ///
    /// The separator is asserted as a literal because it is a wire contract
    /// shared with a package that cannot import this constant
    /// (`sdks/stripe-js/src/client.ts`'s `SECRET_SEPARATOR`). A rename here
    /// that only touched the constant would compile, pass every other test,
    /// and make every browser call address `/v1/browser/payment_intents/` with
    /// an empty id.
    #[test]
    fn a_client_secret_is_the_id_the_separator_and_the_suffix() {
        assert_eq!(CLIENT_SECRET_INFIX, "_secret_");

        let id = payment_intent_id();
        let suffix = client_secret_suffix();
        let secret = client_secret(&id, &suffix);

        assert_eq!(secret, format!("{id}_secret_{suffix}"));
        assert!(secret.starts_with(&id));
        // What the browser package does, reproduced: everything before the
        // first separator is the id, everything after it is the suffix.
        let (recovered_id, recovered_suffix) = secret
            .split_once("_secret_")
            .expect("a client secret carries the separator");
        assert_eq!(recovered_id, id);
        assert_eq!(recovered_suffix, suffix);

        // And the whole thing survives a URL, which it must: it travels as a
        // query parameter on every poll.
        assert_eq!(percent_encode(&secret), secret);
    }

    /// A return token is 32 alphabet characters, distinct in every position,
    /// and — the property D6 rests on — **never equal to the client-secret
    /// suffix minted beside it**.
    ///
    /// The last assertion is what a shared body could quietly break: if
    /// `return_token` were ever made to *derive* from the session's secret
    /// (a hash, a truncation, a shared draw), then the value that travels in
    /// a query string would be a function of the value that authorises
    /// reading the intent's own `client_secret`, and the two capabilities
    /// would stop being separate. `secret_body` draws afresh on every call,
    /// which is what keeps them independent.
    #[test]
    fn a_return_token_is_thirty_two_characters_and_independent_of_the_secret_beside_it() {
        const N: usize = 4_096;

        for _ in 0..64 {
            let token = return_token();
            assert_eq!(
                token.len(),
                32,
                "migration 0028's return_token_length CHECK is BETWEEN 32 AND 128"
            );
            assert!(token.is_ascii(), "return_token() must be ASCII");
            for c in token.chars() {
                assert!(
                    c.is_ascii_lowercase() || c.is_ascii_digit(),
                    "return_token() produced {c:?}, outside the [a-z0-9] alphabet"
                );
            }
            // It is a *query parameter* on the return page, so it has to
            // survive one unchanged.
            assert_eq!(percent_encode(&token), token);
        }

        let tokens: Vec<String> = (0..N).map(|_| return_token()).collect();
        let unique: HashSet<&String> = tokens.iter().collect();
        assert_eq!(unique.len(), N, "a collision in {N} return tokens");
        for position in 0..32 {
            let seen: HashSet<Option<char>> =
                tokens.iter().map(|t| t.chars().nth(position)).collect();
            assert!(
                seen.len() > 1,
                "position {position} is constant across {N} return tokens"
            );
        }

        // The two credentials a session carries are two independent draws.
        // A thousand pairs is far more than enough to catch a derivation;
        // a genuine 160-bit collision here has probability ~2^-140.
        for _ in 0..1_000 {
            assert_ne!(
                return_token(),
                client_secret_suffix(),
                "a session's return_token must not be a function of its client_secret_suffix"
            );
        }
    }

    /// A session's `client_secret` is spelled with the *session's* id, and
    /// splitting it recovers that id — the property `@vpay/stripe-js`'s
    /// `retrieveCheckoutSession` relies on, exactly as it does for an
    /// intent's.
    #[test]
    fn a_checkout_session_secret_is_the_session_id_and_never_an_intent_id() {
        let id = checkout_session_id();
        let secret = client_secret(&id, &client_secret_suffix());

        assert!(secret.starts_with("cs_"), "{secret}");
        let (recovered, _suffix) = secret
            .split_once(CLIENT_SECRET_INFIX)
            .expect("a client secret carries the separator");
        assert_eq!(recovered, id);
        // The whole credential survives a URL fragment and a query string.
        assert_eq!(percent_encode(&secret), secret);
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
