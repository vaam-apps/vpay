//! A small Stripe-style, bracket-nested `application/x-www-form-urlencoded`
//! encoder.
//!
//! `docs/flows/merchant-auth.md`'s encoding table pins this exactly:
//! `metadata[order_id]=1234` for a nested object, `payment_method_types[0]=…`
//! for an array (indexed, "as `stripe-node`/`stripe-rust` send").
//!
//! **Byte-for-byte parity with `sdks/nodejs/src/form.ts` is the requirement
//! here, not merely "a correct form encoding".** The two SDKs implement one
//! wire contract, and a server-side signature, log line or fixture that
//! matches one SDK's body and not the other's would be a defect nobody could
//! see from either side alone. So the escaping rule is JavaScript's
//! `encodeURIComponent`, reproduced deliberately rather than approximated:
//! `A-Za-z0-9` and `-_.!~*'()` pass through, every other byte becomes
//! `%XX` (uppercase hex, UTF-8 bytes). That is *not* the WHATWG
//! `application/x-www-form-urlencoded` serializer `form_urlencoded::Serializer`
//! implements — it would emit `+` for a space and escape `!*'()` — and it is
//! not RFC 3986's unreserved set either.
//!
//! Brackets are structural: they are emitted literally around each key
//! segment, while the segment itself is escaped like any other string. So a
//! `[` *inside* a metadata key becomes `%5B` and can never be mistaken for
//! nesting. `form.ts` now carries its keys as an array of path segments and
//! assembles them the same way, so the two agree here — including on a key
//! segment holding an unbalanced `[`. (Both statements in this paragraph
//! were false in an earlier revision: `form.ts` used to re-parse its
//! assembled key with a bracket regex, which mangled such a key, and this
//! doc claimed that as the one remaining divergence long after it was
//! fixed.)
//!
//! # Numbers
//!
//! The other historic divergence — this encoder rendering any `i64` while
//! `form.ts` refuses anything that is not a *safe* integer — is closed at the
//! params layer rather than here. `crate::validate::check_amount` refuses a
//! negative or `> 2^53-1` amount before a request is built, and the only
//! other numeric field in the crate is `limit: u32`, whose whole range is
//! representable in both languages. So no params type can hand this encoder
//! a number `form.ts` would have thrown on, and the encoder itself stays
//! infallible. If a future field takes an unbounded integer, the check
//! belongs beside it in `crate::validate`, not in `From<i64> for FormValue`.

use std::fmt::Write as _;

/// A nested value shaped for bracket-form encoding.
///
/// A hand-rolled enum rather than routing typed params through
/// `serde_json::Value`: a JSON object's key order is unspecified unless the
/// whole workspace opts into `serde_json`'s `preserve_order` feature (which,
/// under Cargo's feature unification, would also change every other crate's
/// `Value` ordering) — whereas `Vec<(String, FormValue)>` preserves
/// construction order for free, which is what lets each params type pin its
/// own field order to match the docs' wire examples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormValue {
    /// An absent optional field: the key is omitted entirely rather than
    /// sent empty. `description` unset and `description=` are different
    /// requests, and only the first is what "omit for full" (`docs/flows/
    /// merchant-auth.md`'s refund table) means.
    Skip,
    Scalar(String),
    Array(Vec<FormValue>),
    Object(Vec<(String, FormValue)>),
}

impl From<&str> for FormValue {
    fn from(v: &str) -> Self {
        FormValue::Scalar(v.to_string())
    }
}

impl From<String> for FormValue {
    fn from(v: String) -> Self {
        FormValue::Scalar(v)
    }
}

impl From<i64> for FormValue {
    fn from(v: i64) -> Self {
        FormValue::Scalar(v.to_string())
    }
}

impl From<u32> for FormValue {
    fn from(v: u32) -> Self {
        FormValue::Scalar(v.to_string())
    }
}

impl From<bool> for FormValue {
    fn from(v: bool) -> Self {
        FormValue::Scalar(if v { "true" } else { "false" }.to_string())
    }
}

impl<T: Into<FormValue>> From<Option<T>> for FormValue {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(inner) => inner.into(),
            None => FormValue::Skip,
        }
    }
}

/// Appends one already-escaped bracket segment to an already-escaped prefix.
///
/// Keys are assembled *escaped*, never escaped again afterwards: escaping a
/// finished key would turn its structural brackets into `%5B`/`%5D`, and
/// un-escaping them selectively afterwards is exactly the ambiguity the
/// module doc describes.
fn child_key(prefix: Option<&str>, segment: &str) -> String {
    let escaped = percent_encode(segment);
    match prefix {
        Some(p) => format!("{p}[{escaped}]"),
        None => escaped,
    }
}

fn flatten(prefix: Option<&str>, value: &FormValue, out: &mut Vec<(String, String)>) {
    match value {
        FormValue::Skip => {}
        FormValue::Scalar(s) => {
            if let Some(p) = prefix {
                out.push((p.to_string(), s.clone()));
            }
            // A bare scalar with no prefix has no key to encode under; not
            // reachable from any params type in this crate, so silently
            // dropping it costs nothing today and avoids inventing a key.
        }
        FormValue::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                flatten(Some(&child_key(prefix, &i.to_string())), item, out);
            }
        }
        FormValue::Object(entries) => {
            for (k, v) in entries {
                flatten(Some(&child_key(prefix, k)), v, out);
            }
        }
    }
}

/// The set JavaScript's `encodeURIComponent` leaves unescaped — see the
/// module doc for why parity with it, rather than RFC 3986 or the WHATWG
/// serializer, is the rule this encoder follows.
fn is_safe_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
        )
}

/// JavaScript's `encodeURIComponent`, byte for byte. Used for form keys and
/// values here, and for path segments in [`crate::resources`] — the Node SDK
/// uses the same function in both places, and an id escaped one way in one
/// SDK and another way in the other would address two different URLs.
pub(crate) fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if is_safe_byte(byte) {
            out.push(byte as char);
        } else {
            // `write!` to a `String` never fails. Uppercase hex, matching
            // `encodeURIComponent` (`%20`, not `%20`'s lowercase spelling).
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Flattens `value` into an `application/x-www-form-urlencoded` body (or
/// query string — the wire contract uses the same encoder for both).
pub(crate) fn encode_form(value: &FormValue) -> String {
    let mut pairs = Vec::new();
    flatten(None, value, &mut pairs);
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={}", percent_encode(&v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_scalar() {
        let form = FormValue::Object(vec![("amount".to_string(), FormValue::from(5000_i64))]);
        assert_eq!(encode_form(&form), "amount=5000");
    }

    #[test]
    fn encodes_an_indexed_array() {
        let form = FormValue::Object(vec![(
            "payment_method_types".to_string(),
            FormValue::Array(vec![
                FormValue::from("mtn_momo"),
                FormValue::from("orange_money"),
            ]),
        )]);
        assert_eq!(
            encode_form(&form),
            "payment_method_types[0]=mtn_momo&payment_method_types[1]=orange_money"
        );
    }

    #[test]
    fn encodes_a_nested_object() {
        let form = FormValue::Object(vec![(
            "metadata".to_string(),
            FormValue::Object(vec![("order_id".to_string(), FormValue::from("1234"))]),
        )]);
        assert_eq!(encode_form(&form), "metadata[order_id]=1234");
    }

    #[test]
    fn encodes_doubly_nested_objects() {
        // payment_method_data[mtn_momo][msisdn]=… from the confirm contract.
        let form = FormValue::Object(vec![(
            "payment_method_data".to_string(),
            FormValue::Object(vec![(
                "mtn_momo".to_string(),
                FormValue::Object(vec![(
                    "msisdn".to_string(),
                    FormValue::from("237670000000"),
                )]),
            )]),
        )]);
        assert_eq!(
            encode_form(&form),
            "payment_method_data[mtn_momo][msisdn]=237670000000"
        );
    }

    #[test]
    fn omits_skipped_fields_entirely_rather_than_sending_them_empty() {
        let form = FormValue::Object(vec![
            ("amount".to_string(), FormValue::from(5000_i64)),
            (
                "description".to_string(),
                FormValue::from(Option::<String>::None),
            ),
        ]);
        assert_eq!(encode_form(&form), "amount=5000");
    }

    #[test]
    fn preserves_field_order_exactly_as_constructed() {
        let form = FormValue::Object(vec![
            ("amount".to_string(), FormValue::from(5000_i64)),
            ("currency".to_string(), FormValue::from("xaf")),
            (
                "payment_method_types".to_string(),
                FormValue::Array(vec![FormValue::from("mtn_momo")]),
            ),
            (
                "metadata".to_string(),
                FormValue::Object(vec![("order_id".to_string(), FormValue::from("1234"))]),
            ),
        ]);
        assert_eq!(
            encode_form(&form),
            "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo&metadata[order_id]=1234"
        );
    }

    #[test]
    fn percent_encodes_reserved_characters_and_spaces() {
        let form = FormValue::Object(vec![(
            "description".to_string(),
            FormValue::from("a b&c=d"),
        )]);
        assert_eq!(encode_form(&form), "description=a%20b%26c%3Dd");
    }

    #[test]
    fn escapes_a_bracket_that_appears_inside_a_value() {
        // A bracket is structural only where this encoder emits one. One
        // arriving inside a caller's value must not be able to fake nesting,
        // and `encodeURIComponent` escapes it too — so `%5B`, not `[`.
        let form = FormValue::Object(vec![("k".to_string(), FormValue::from("[x]"))]);
        assert_eq!(encode_form(&form), "k=%5Bx%5D");
    }

    #[test]
    fn escapes_a_bracket_that_appears_inside_a_key_segment() {
        let form = FormValue::Object(vec![(
            "metadata".to_string(),
            FormValue::Object(vec![("a[b".to_string(), FormValue::from("v"))]),
        )]);
        assert_eq!(encode_form(&form), "metadata[a%5Bb]=v");
    }

    #[test]
    fn leaves_exactly_the_characters_encodeuricomponent_leaves() {
        // Locks the parity rule from the module doc in place: `!*'()~.-_` and
        // alphanumerics survive, a space does not. Verified against the Node
        // SDK's own output — see `node_parity` below.
        let form = FormValue::Object(vec![(
            "weird key".to_string(),
            FormValue::from("v!*()~.-_'"),
        )]);
        assert_eq!(encode_form(&form), "weird%20key=v!*()~.-_'");
    }

    /// The byte strings in this module were produced by the Node SDK, not by
    /// reading its source: `node -e 'import("./dist/form.js")...'` against
    /// `sdks/nodejs/dist`, with the same params. They are pasted here so a
    /// change to *either* encoder that breaks parity fails a test in the
    /// repository it was changed in, without needing Node installed to run
    /// this suite.
    mod node_parity {
        use super::*;

        #[test]
        fn create_payment_intent_body_matches_the_node_sdk_byte_for_byte() {
            let form = FormValue::Object(vec![
                ("amount".to_string(), FormValue::from(5000_i64)),
                ("currency".to_string(), FormValue::from("xaf")),
                (
                    "payment_method_types".to_string(),
                    FormValue::Array(vec![
                        FormValue::from("mtn_momo"),
                        FormValue::from("orange_money"),
                    ]),
                ),
                (
                    "metadata".to_string(),
                    FormValue::Object(vec![
                        ("order_id".to_string(), FormValue::from("1234")),
                        ("note".to_string(), FormValue::from("a b&c=d")),
                    ]),
                ),
                (
                    "description".to_string(),
                    FormValue::from("Order #42 (rush)"),
                ),
            ]);
            assert_eq!(
                encode_form(&form),
                "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo\
                 &payment_method_types[1]=orange_money&metadata[order_id]=1234\
                 &metadata[note]=a%20b%26c%3Dd&description=Order%20%2342%20(rush)"
            );
        }

        #[test]
        fn confirm_body_matches_the_node_sdk_byte_for_byte() {
            let form = FormValue::Object(vec![
                (
                    "payment_method_data".to_string(),
                    FormValue::Object(vec![
                        ("type".to_string(), FormValue::from("mtn_momo")),
                        (
                            "mtn_momo".to_string(),
                            FormValue::Object(vec![(
                                "msisdn".to_string(),
                                FormValue::from("237670000000"),
                            )]),
                        ),
                    ]),
                ),
                (
                    "return_url".to_string(),
                    FormValue::from("https://m.example/return?x=1"),
                ),
            ]);
            assert_eq!(
                encode_form(&form),
                "payment_method_data[type]=mtn_momo\
                 &payment_method_data[mtn_momo][msisdn]=237670000000\
                 &return_url=https%3A%2F%2Fm.example%2Freturn%3Fx%3D1"
            );
        }
    }
}
