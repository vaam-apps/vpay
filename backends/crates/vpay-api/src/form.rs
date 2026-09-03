//! The Stripe-style bracket-nested `application/x-www-form-urlencoded`
//! decoder, and the two extractors ([`VpayForm`], [`VpayQuery`]) that put it
//! in front of a handler.
//!
//! This is the *reading* half of a wire contract whose writing half ships in
//! `sdks/rust/src/form.rs` and `sdks/nodejs/src/form.ts`. It is a deliberate
//! port, not a general-purpose form parser: if this file and those disagree, a
//! merchant's request means one thing to their SDK and another to us.
//!
//! The grammar, why `serde_urlencoded` cannot be used, why `+` is a literal
//! plus, why every scalar is a `String`, and where the body bound is enforced:
//! [docs/reference/vpay-api.md § the form decoder](../../../../docs/reference/vpay-api.md#the-form-decoder-formrs).

use std::collections::BTreeMap;

use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::ApiError;

/// How deeply a key may nest: the head segment plus [`MAX_DEPTH`] - 1 bracket
/// groups.
///
/// The deepest key the contract has is
/// `payment_method_data[mtn_momo][msisdn]` — three. Eight leaves room for a
/// resource nobody has designed yet and still bounds the recursion in
/// [`insert`] to something a 64 KiB body cannot turn into a stack overflow.
const MAX_DEPTH: usize = 8;

/// The one content type `POST` bodies may carry
/// (`docs/flows/merchant-auth.md`'s header table).
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

/// A tree under construction. Not `serde_json::Value` directly because a
/// half-built node has a state `Value` cannot express — [`Node::Empty`], "a
/// key was mentioned and we do not yet know whether it names a scalar, an
/// object or an array" — and because arrays are keyed by their index while
/// being built (see [`Node::Array`]).
#[derive(Debug)]
enum Node {
    /// Mentioned, not yet resolved. Never survives [`Node::into_value`] in
    /// practice: every path a caller can take either fills it in or fails.
    Empty,
    Scalar(String),
    /// Keyed by index rather than a `Vec`, so `k[1]=b&k[0]=a` round-trips in
    /// the order the *merchant* numbered it rather than the order the pairs
    /// happened to arrive in, and so a sparse `k[7]=v` costs one entry rather
    /// than eight. Indices are compacted away on the way out — Stripe does
    /// the same — because the wire has no concept of a hole.
    Array(BTreeMap<u64, Node>),
    Object(BTreeMap<String, Node>),
}

/// One step of a key path.
#[derive(Debug, PartialEq, Eq)]
enum Segment {
    /// `metadata`, `order_id` — the parent is an object.
    Key(String),
    /// `[0]` — the parent is an array, and this is the caller's own index.
    Index(u64),
    /// `[]` — the parent is an array and the caller left the index to us.
    Append,
}

impl Node {
    fn into_value(self) -> Value {
        match self {
            // An `Empty` that reached here would be a bug in `insert`, not a
            // caller's doing. `null` rather than a panic (ADR-0007) and rather
            // than an empty string, which would look like a field the caller
            // actually sent.
            Self::Empty => Value::Null,
            Self::Scalar(s) => Value::String(s),
            Self::Array(items) => Value::Array(items.into_values().map(Self::into_value).collect()),
            Self::Object(fields) => Value::Object(
                fields
                    .into_iter()
                    .map(|(k, v)| (k, v.into_value()))
                    .collect::<Map<String, Value>>(),
            ),
        }
    }
}

/// Decodes a bracket-nested form body (or query string) into the JSON shape a
/// handler's params type deserializes from.
///
/// Every scalar is a `Value::String` — see this module's "Everything is a
/// string".
///
/// # Errors
/// [`ApiError::InvalidParam`] for anything the grammar above does not accept:
/// a malformed percent escape, bytes that are not UTF-8 once decoded, an
/// unbalanced bracket, nesting past `MAX_DEPTH`, or a key sent twice (or
/// sent once as a scalar and once as a container). `param` names the offending
/// top-level key where there is one, and `body` where the failure is the
/// body's own shape.
/// # Examples
///
/// Brackets are nesting, and every scalar comes back a string:
///
/// ```
/// use serde_json::json;
/// use vpay_api::form::parse_form;
///
/// let decoded = parse_form(b"amount=5000&currency=xaf&metadata[order_id]=1234")
///     .expect("a well-formed body");
/// assert_eq!(
///     decoded,
///     json!({"amount": "5000", "currency": "xaf", "metadata": {"order_id": "1234"}}),
/// );
/// ```
///
/// Both array spellings the contract carries decode to the same list, and `+`
/// is a literal plus — the rule that keeps an MSISDN intact:
///
/// ```
/// use serde_json::json;
/// use vpay_api::form::parse_form;
///
/// let numbered = parse_form(b"payment_method_types[0]=mtn_momo&payment_method_types[1]=orange_money")
///     .expect("a well-formed body");
/// let appended = parse_form(b"payment_method_types[]=mtn_momo&payment_method_types[]=orange_money")
///     .expect("a well-formed body");
/// assert_eq!(numbered, appended);
/// assert_eq!(numbered, json!({"payment_method_types": ["mtn_momo", "orange_money"]}));
///
/// let instrument = parse_form(b"payment_method_data[mtn_momo][msisdn]=%2B237670000000")
///     .expect("a well-formed body");
/// assert_eq!(
///     instrument["payment_method_data"]["mtn_momo"]["msisdn"],
///     json!("+237670000000"),
/// );
/// let literal_plus = parse_form(b"description=a+b").expect("a well-formed body");
/// assert_eq!(literal_plus["description"], json!("a+b"));
/// ```
///
/// A key sent twice is refused rather than silently resolved to one of the two
/// values, and the refusal names the top-level key:
///
/// ```
/// use vpay_api::form::parse_form;
///
/// assert!(parse_form(b"amount=1&amount=2").is_err());
/// assert!(parse_form(b"metadata[a]=1&metadata[a][b]=2").is_err());
/// ```
pub fn parse_form(bytes: &[u8]) -> Result<Value, ApiError> {
    parse_pairs(bytes, "body")
}

/// [`parse_form`], with the `param` an un-attributable failure is reported
/// under. `query` for a query string, so an SDK is pointed at the URL rather
/// than at a body the request did not have.
fn parse_pairs(bytes: &[u8], fallback_param: &'static str) -> Result<Value, ApiError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ApiError::invalid_param(
            fallback_param,
            "The request was not valid UTF-8. Percent-escape any byte outside US-ASCII.",
        )
    })?;

    let mut root = Node::Object(BTreeMap::new());
    for pair in text.split('&') {
        // A trailing `&`, a leading one, or `a=1&&b=2`: nothing was sent, so
        // there is nothing to reject.
        if pair.is_empty() {
            continue;
        }
        // No `=` at all is a key with an empty value, which is what every
        // form encoder produces for one and what `sdks/rust/tests/support`'s
        // own `form_pairs` reader assumes.
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));

        let path = parse_key(raw_key).ok_or_else(|| malformed_key(raw_key, fallback_param))?;
        let value =
            percent_decode(raw_value).ok_or_else(|| malformed_value(&path, fallback_param))?;

        insert(&mut root, &path, value).map_err(|_| conflicting_key(&path, fallback_param))?;
    }
    Ok(root.into_value())
}

/// Splits a raw (still-escaped) key into its path, or `None` if it is not a
/// key this grammar accepts.
///
/// The split happens *before* any decoding — see the module doc.
fn parse_key(raw_key: &str) -> Option<Vec<Segment>> {
    let (head, mut rest) = match raw_key.find('[') {
        Some(i) => (raw_key.get(..i)?, raw_key.get(i..)?),
        None => (raw_key, ""),
    };
    // A `]` in the head, or an empty head, is not something the encoder can
    // produce: it escapes both (`%5D`, and a key is never empty).
    if head.is_empty() || head.contains(']') {
        return None;
    }

    let mut path = vec![Segment::Key(percent_decode(head)?)];
    while !rest.is_empty() {
        let after_open = rest.strip_prefix('[')?;
        let close = after_open.find(']')?;
        let raw_segment = after_open.get(..close)?;
        // A structural bracket inside a group means the caller sent an
        // unescaped `[` where the encoder would have sent `%5B`.
        if raw_segment.contains('[') {
            return None;
        }
        path.push(segment_for(raw_segment)?);
        rest = after_open.get(close + 1..)?;
        if path.len() > MAX_DEPTH {
            return None;
        }
    }
    Some(path)
}

/// Classifies one bracket group: empty or all-digits is an array position,
/// anything else is an object key.
fn segment_for(raw_segment: &str) -> Option<Segment> {
    if raw_segment.is_empty() {
        return Some(Segment::Append);
    }
    if raw_segment.bytes().all(|b| b.is_ascii_digit()) {
        // All digits but wider than a `u64` is refused rather than treated as
        // an object key: `k[99999999999999999999]` is a caller doing something
        // strange, and guessing which of the two they meant would be worse
        // than saying so.
        return raw_segment.parse::<u64>().ok().map(Segment::Index);
    }
    percent_decode(raw_segment).map(Segment::Key)
}

/// Places `value` at `path`, creating the containers it passes through.
///
/// `Err(())` means the path contradicts what is already there: a scalar where
/// a container is needed, a container where a scalar is, or the same scalar
/// key twice. The unit error is deliberate — the caller owns the sentence,
/// because only it knows whether this came from a body or a query string.
fn insert(node: &mut Node, path: &[Segment], value: String) -> Result<(), ()> {
    let Some((segment, rest)) = path.split_first() else {
        // `parse_key` never returns an empty path; treating it as a conflict
        // rather than asserting keeps this function total.
        return Err(());
    };

    let slot = match segment {
        Segment::Key(key) => {
            let Node::Object(fields) = node else {
                return Err(());
            };
            fields.entry(key.clone()).or_insert(Node::Empty)
        }
        Segment::Index(index) => {
            let Node::Array(items) = node else {
                return Err(());
            };
            items.entry(*index).or_insert(Node::Empty)
        }
        Segment::Append => {
            let Node::Array(items) = node else {
                return Err(());
            };
            // One past the highest index used so far, so `k[]` after `k[0]`
            // appends rather than overwriting. `saturating_add` because the
            // index is the caller's number and `u64::MAX` is a legal one to
            // have sent.
            let next = items.keys().next_back().map_or(0, |i| i.saturating_add(1));
            items.entry(next).or_insert(Node::Empty)
        }
    };

    let Some(next_segment) = rest.first() else {
        // The leaf. Anything already here — a scalar (`amount` twice) or a
        // container (`k=1` after `k[0]=2`) — is a contradiction rather than an
        // overwrite: silently keeping the last value is how a merchant's
        // duplicated field becomes a payment for the wrong amount.
        return match slot {
            Node::Empty => {
                *slot = Node::Scalar(value);
                Ok(())
            }
            _ => Err(()),
        };
    };

    if matches!(slot, Node::Empty) {
        *slot = match next_segment {
            Segment::Key(_) => Node::Object(BTreeMap::new()),
            Segment::Index(_) | Segment::Append => Node::Array(BTreeMap::new()),
        };
    }
    insert(slot, rest, value)
}

/// Percent-decoding, and *only* percent-decoding: `+` is a literal — see the
/// module doc.
///
/// `None` for a truncated or non-hex escape, or bytes that are not UTF-8 once
/// decoded. Written over bytes rather than `str::char_indices` because an
/// escape sequence can be half of a multi-byte character (`%C3%A9`), which is
/// only valid UTF-8 after both halves are decoded.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        if byte == b'%' {
            let high = hex_nibble(*bytes.get(i + 1)?)?;
            let low = hex_nibble(*bytes.get(i + 2)?)?;
            out.push((high << 4) | low);
            i += 3;
        } else {
            out.push(byte);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// One hex digit's value. Both cases accepted: the SDKs emit uppercase
/// (`encodeURIComponent` does), but a hand-written curl or another client may
/// not, and rejecting `%5b` would be a difference nobody could have predicted
/// from the contract.
fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The top-level key a failure is attributed to.
///
/// The decoded head segment — `metadata`, not `metadata[a%5Bb]` — because
/// that is the field name an SDK points a merchant at, and because
/// `ApiError`'s own `param` bound rejects anything that is not shaped like one
/// (falling back to `request`), so an odd key cannot become a reflection
/// channel here.
fn top_level_param(path: &[Segment], fallback: &'static str) -> String {
    match path.first() {
        Some(Segment::Key(key)) => key.clone(),
        _ => fallback.to_owned(),
    }
}

fn malformed_key(raw_key: &str, fallback: &'static str) -> ApiError {
    // The raw key cannot be split into a path, so there is no top-level key to
    // name — except the part before the first bracket, which is still the most
    // useful thing to point at.
    let head = raw_key.split('[').next().unwrap_or(raw_key);
    let param = percent_decode(head).unwrap_or_else(|| fallback.to_owned());
    ApiError::invalid_param(
        param,
        "A parameter name was malformed. Bracket-nest a key as `parent[child]` and \
         percent-escape any bracket that is part of a name.",
    )
}

fn malformed_value(path: &[Segment], fallback: &'static str) -> ApiError {
    ApiError::invalid_param(
        top_level_param(path, fallback),
        "A parameter value was not valid percent-encoded UTF-8.",
    )
}

fn conflicting_key(path: &[Segment], fallback: &'static str) -> ApiError {
    ApiError::invalid_param(
        top_level_param(path, fallback),
        "A parameter was sent more than once, or was sent both as a value and as a nested \
         object. Send each parameter exactly once.",
    )
}

/// `T`, deserialized from a bracket-nested form body.
///
/// The `POST` counterpart of [`VpayQuery`], and the extractor every `/v1`
/// handler that takes a body uses instead of `axum::Form` — see this module's
/// doc for the two behavioural differences (`+`, and nesting) that make
/// `axum::Form` the wrong reader for this contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VpayForm<T>(pub T);

impl<T, S> FromRequest<S> for VpayForm<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        check_content_type(
            request
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
        )?;

        let bytes = axum::body::Bytes::from_request(request, state)
            .await
            .map_err(|_| {
                // A body whose `content-length` is over the 64 KiB limit
                // never reaches this arm: the `RequestBodyLimitLayer` on the
                // `/v1` nest answers 413 before a byte is read (see the
                // module doc), and every SDK and `curl` request sends that
                // header. What is left is a connection that ended mid-body —
                // and a chunked body with no declared length that runs past
                // the bound, which the layer can only stop mid-stream and
                // which therefore lands here as a 400 rather than a 413.
                // Answering 413 for it would need an `ApiError` variant
                // classified `Category::…` for 413, which
                // `vpay_core::error::Category` does not have; the refusal is
                // correct either way, only the status is less specific.
                ApiError::invalid_param(
                    "body",
                    "The request body could not be read. Send it in one piece as \
                     `application/x-www-form-urlencoded`.",
                )
            })?;

        let value = parse_form(&bytes)?;
        let parsed = serde_json::from_value(value).map_err(|_| {
            // serde's own text is dropped for the reasons `error.rs`'s
            // `extractor_rejection!` gives: it names our struct fields and
            // changes between releases. A *handler* says which field is wrong
            // and why, in vpay's vocabulary; this is only the shape gate.
            ApiError::invalid_param(
                "body",
                "The request body did not have the fields this endpoint requires. \
                 Check the field names this endpoint documents.",
            )
        })?;
        Ok(Self(parsed))
    }
}

/// `T`, deserialized from a bracket-nested query string.
///
/// The same grammar as [`VpayForm`], because the SDKs use the same encoder for
/// both (`sdks/rust/src/form.rs`: "the wire contract uses the same encoder for
/// both"). A `GET /v1/payment_intents?limit=10&starting_after=pi_…` therefore
/// decodes through exactly the code a `POST` body does, and neither can gain a
/// quirk the other lacks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VpayQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for VpayQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or_default();
        let value = parse_pairs(query.as_bytes(), "query")?;
        let parsed = serde_json::from_value(value).map_err(|_| {
            ApiError::invalid_param(
                "query",
                "The query string did not have the parameters this endpoint requires. \
                 Check the parameter names this endpoint documents.",
            )
        })?;
        Ok(Self(parsed))
    }
}

/// Refuses a body that announces itself as something other than a form.
///
/// A missing header is **accepted**: `curl -d` sets one, both SDKs set one,
/// and a client that sets none is sending bytes this decoder can read
/// unambiguously anyway. A header naming `application/json`, though, is a
/// merchant who has read the wrong documentation — parsing their JSON as a
/// form would produce a nonsense key and an error about a field they did send,
/// which is the least useful answer available.
fn check_content_type(header: Option<&str>) -> Result<(), ApiError> {
    let Some(value) = header else {
        return Ok(());
    };
    // `application/x-www-form-urlencoded; charset=utf-8` is the same content
    // type; the parameters are not ours to police.
    let mime = value.split(';').next().unwrap_or(value).trim();
    if mime.is_empty() || mime.eq_ignore_ascii_case(FORM_CONTENT_TYPE) {
        return Ok(());
    }
    Err(ApiError::invalid_param(
        "body",
        "The request body must be sent as `application/x-www-form-urlencoded`.",
    ))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::header::CONTENT_LENGTH;
    use axum::http::{Request, StatusCode};
    use axum::routing::{get, post};
    use serde::Deserialize;
    use serde_json::json;
    use tower::ServiceExt as _;
    use tower_http::limit::RequestBodyLimitLayer;
    use vpay_core::{Category, Classify as _};

    use super::*;

    /// The limit `docs/flows/merchant-auth.md`'s contract is served under.
    ///
    /// Read from [`crate::V1_BODY_LIMIT_BYTES`] rather than restated as a
    /// second `64 * 1024`: the layer is mounted in `lib.rs` (on the `/v1`
    /// nest, outside the token check), and a copy of the number here could
    /// drift from it silently — leaving this module proving a bound nothing
    /// serves. See `a_body_over_the_limit_is_refused_by_the_layer`.
    const BODY_LIMIT_BYTES: usize = crate::V1_BODY_LIMIT_BYTES;

    fn parse(body: &str) -> Value {
        parse_form(body.as_bytes()).expect("the body parses")
    }

    fn reject(body: &str) -> ApiError {
        parse_form(body.as_bytes()).expect_err("the body must be refused")
    }

    fn param_of(error: &ApiError) -> Option<&str> {
        error.param()
    }

    #[test]
    fn a_scalar_is_a_string_because_the_wire_has_no_types() {
        assert_eq!(parse("amount=5000"), json!({ "amount": "5000" }));
        // Not `5000` the number: see the module doc. A handler validates it
        // and says so in vpay's own words.
        assert!(
            parse("amount=5000")
                .get("amount")
                .is_some_and(Value::is_string)
        );
    }

    #[test]
    fn an_empty_body_is_an_empty_object() {
        assert_eq!(parse(""), json!({}));
        assert_eq!(parse("&"), json!({}));
        assert_eq!(parse("a=1&&b=2"), json!({ "a": "1", "b": "2" }));
    }

    #[test]
    fn a_pair_with_no_equals_is_an_empty_value() {
        assert_eq!(parse("description"), json!({ "description": "" }));
        assert_eq!(parse("description="), json!({ "description": "" }));
    }

    #[test]
    fn both_array_spellings_produce_the_same_array() {
        let indexed =
            parse("payment_method_types[0]=mtn_momo&payment_method_types[1]=orange_money");
        let bare = parse("payment_method_types[]=mtn_momo&payment_method_types[]=orange_money");
        assert_eq!(
            indexed,
            json!({ "payment_method_types": ["mtn_momo", "orange_money"] })
        );
        assert_eq!(
            bare, indexed,
            "the curl spelling and the SDK spelling must mean the same thing"
        );
    }

    /// Stripe compacts a sparse array and orders by the caller's index, and
    /// the SDKs number from zero in order — so this is about robustness for a
    /// hand-written client, not about the SDKs.
    #[test]
    fn array_indices_order_the_elements_and_holes_are_compacted() {
        assert_eq!(parse("k[1]=b&k[0]=a"), json!({ "k": ["a", "b"] }));
        assert_eq!(parse("k[7]=only"), json!({ "k": ["only"] }));
    }

    #[test]
    fn nested_objects_nest() {
        assert_eq!(
            parse(
                "payment_method_data[type]=mtn_momo&payment_method_data[mtn_momo][msisdn]=237670000000"
            ),
            json!({
                "payment_method_data": {
                    "type": "mtn_momo",
                    "mtn_momo": { "msisdn": "237670000000" }
                }
            })
        );
    }

    /// The ordering rule the module doc calls load-bearing, against the exact
    /// byte string `sdks/rust/src/form.rs`'s
    /// `escapes_a_bracket_that_appears_inside_a_key_segment` emits.
    #[test]
    fn metadata_key_with_an_escaped_bracket_is_one_key() {
        assert_eq!(
            parse("metadata[a%5Bb]=v"),
            json!({ "metadata": { "a[b": "v" } })
        );
        // Decoding before splitting would have produced this instead — the
        // assertion that makes the ordering testable rather than a claim.
        assert_ne!(
            parse("metadata[a%5Bb]=v"),
            json!({ "metadata": { "a": { "b": "v" } } })
        );
    }

    #[test]
    fn a_plus_is_a_plus_and_a_space_is_percent_twenty() {
        // The MSISDN case: `serde_urlencoded` (and therefore `axum::Form`)
        // would hand a handler ` 237670000000` here.
        assert_eq!(
            parse("payment_method_data[mtn_momo][msisdn]=%2B237670000000"),
            json!({ "payment_method_data": { "mtn_momo": { "msisdn": "+237670000000" } } })
        );
        assert_eq!(parse("k=a+b"), json!({ "k": "a+b" }));
        assert_eq!(parse("k=a%20b"), json!({ "k": "a b" }));
    }

    #[test]
    fn a_multibyte_value_survives_being_split_across_escapes() {
        assert_eq!(
            parse("description=caf%C3%A9"),
            json!({ "description": "café" })
        );
    }

    #[test]
    fn lowercase_escapes_decode_too() {
        assert_eq!(
            parse("metadata[a%5bb]=v"),
            json!({ "metadata": { "a[b": "v" } })
        );
    }

    // --- what is refused ---

    #[test]
    fn a_duplicated_scalar_key_is_refused_rather_than_last_wins() {
        let error = reject("amount=5000&amount=1");
        assert_eq!(param_of(&error), Some("amount"));
        assert_eq!(error.category(), Category::InvalidRequest);
        // Nested, too: the conflict is per key path, not only at the root.
        assert_eq!(
            param_of(&reject("metadata[order_id]=1&metadata[order_id]=2")),
            Some("metadata")
        );
    }

    #[test]
    fn a_key_used_as_both_a_scalar_and_a_container_is_refused() {
        assert_eq!(param_of(&reject("k=1&k[0]=2")), Some("k"));
        assert_eq!(param_of(&reject("k[0]=2&k=1")), Some("k"));
        assert_eq!(param_of(&reject("k[a]=1&k[0]=2")), Some("k"));
        assert_eq!(param_of(&reject("k[0]=1&k[a]=2")), Some("k"));
    }

    #[test]
    fn a_malformed_key_or_escape_is_refused() {
        for body in [
            "k[a=v",     // unbalanced
            "k]=v",      // a `]` the encoder would have escaped
            "=v",        // no key at all
            "k[a[b]]=v", // an unescaped `[` inside a group
            "k%=v",      // truncated escape in a key
            "k%zz=v",    // non-hex escape in a key
        ] {
            let error = parse_form(body.as_bytes())
                .err()
                .unwrap_or_else(|| panic!("{body} must be refused"));
            assert_eq!(error.category(), Category::InvalidRequest, "{body}");
        }
        assert_eq!(param_of(&reject("k=%zz")), Some("k"));
        assert_eq!(param_of(&reject("k=%2")), Some("k"));
    }

    #[test]
    fn nesting_past_the_bound_is_refused() {
        let ok = format!("a{}=v", "[b]".repeat(MAX_DEPTH - 1));
        parse_form(ok.as_bytes()).expect("exactly MAX_DEPTH segments is allowed");
        let too_deep = format!("a{}=v", "[b]".repeat(MAX_DEPTH));
        parse_form(too_deep.as_bytes()).expect_err("one segment too many is refused");
    }

    #[test]
    fn an_array_index_wider_than_a_u64_is_refused() {
        parse_form(b"k[99999999999999999999999]=v").expect_err("refused, not silently an object");
    }

    #[test]
    fn a_body_that_is_not_utf8_is_refused() {
        let error = parse_form(&[0xff, b'=', b'1']).expect_err("refused");
        assert_eq!(param_of(&error), Some("body"));
    }

    // --- the node_parity bodies, decoded ---

    /// The two byte strings `sdks/rust/src/form.rs`'s `node_parity` module
    /// pins as the Node SDK's own output, read back. Pasted verbatim rather
    /// than rebuilt from a params type: the point is that *these exact bytes*,
    /// which a shipping SDK really sends, mean what the SDK meant by them.
    mod node_parity {
        use super::*;

        pub(super) const CREATE_BODY: &str = "amount=5000&currency=xaf&payment_method_types[0]=mtn_momo\
             &payment_method_types[1]=orange_money&metadata[order_id]=1234\
             &metadata[note]=a%20b%26c%3Dd&description=Order%20%2342%20(rush)";

        pub(super) const CONFIRM_BODY: &str = "payment_method_data[type]=mtn_momo\
             &payment_method_data[mtn_momo][msisdn]=237670000000\
             &return_url=https%3A%2F%2Fm.example%2Freturn%3Fx%3D1";

        #[test]
        fn create_payment_intent_body_decodes_to_the_params_the_sdk_encoded() {
            assert_eq!(
                parse(CREATE_BODY),
                json!({
                    "amount": "5000",
                    "currency": "xaf",
                    "payment_method_types": ["mtn_momo", "orange_money"],
                    "metadata": { "order_id": "1234", "note": "a b&c=d" },
                    "description": "Order #42 (rush)"
                })
            );
        }

        #[test]
        fn confirm_body_decodes_to_the_params_the_sdk_encoded() {
            assert_eq!(
                parse(CONFIRM_BODY),
                json!({
                    "payment_method_data": {
                        "type": "mtn_momo",
                        "mtn_momo": { "msisdn": "237670000000" }
                    },
                    "return_url": "https://m.example/return?x=1"
                })
            );
        }

        /// The `&`, `=` and `#` inside those values are the reason the split
        /// on `&` and `=` happens before decoding. If the order were reversed,
        /// `metadata[note]=a%20b%26c%3Dd` would become three pairs.
        #[test]
        fn a_value_containing_an_ampersand_is_one_value() {
            assert_eq!(
                parse(CREATE_BODY)
                    .get("metadata")
                    .and_then(|m| m.get("note"))
                    .and_then(Value::as_str),
                Some("a b&c=d")
            );
        }
    }

    // --- the extractors, over a real router ---

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct CreateParams {
        amount: String,
        currency: String,
        #[serde(default)]
        payment_method_types: Vec<String>,
        #[serde(default)]
        metadata: BTreeMap<String, String>,
        description: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct ListParams {
        limit: Option<String>,
        starting_after: Option<String>,
    }

    async fn create(VpayForm(params): VpayForm<CreateParams>) -> String {
        format!("{params:?}")
    }

    async fn list(VpayQuery(params): VpayQuery<ListParams>) -> String {
        format!("{:?}/{:?}", params.limit, params.starting_after)
    }

    fn app() -> Router {
        Router::new()
            .route("/create", post(create))
            .route("/list", get(list))
            .layer(RequestBodyLimitLayer::new(BODY_LIMIT_BYTES))
    }

    async fn send(request: Request<Body>) -> (StatusCode, String) {
        let response = app()
            .oneshot(request)
            .await
            .expect("the router does not fail to serve");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is readable");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A `POST /create` carrying `body`, shaped like what a real client
    /// sends.
    ///
    /// `content-length` is set explicitly because `Request::builder` does
    /// not: hyper puts it on every non-streamed request, and
    /// `RequestBodyLimitLayer` uses it to refuse an oversized body *before*
    /// reading a byte of it. Without the header the layer can only enforce
    /// the bound while streaming, the read fails inside the extractor
    /// instead, and `a_body_over_the_limit_is_refused_by_the_layer` would be
    /// measuring the extractor's transport-failure arm rather than the
    /// layer — see that test.
    fn form_request(body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/create")
            .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
            .header(CONTENT_LENGTH, body.len())
            .body(Body::from(body.to_owned()))
            .expect("a valid request")
    }

    #[tokio::test]
    async fn a_handler_receives_the_decoded_params() {
        let (status, body) = send(form_request(node_parity::CREATE_BODY)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("\"5000\""), "{body}");
        assert!(body.contains("orange_money"), "{body}");
        assert!(body.contains("Order #42 (rush)"), "{body}");
    }

    #[tokio::test]
    async fn a_query_string_decodes_through_the_same_grammar() {
        let request = Request::builder()
            .uri("/list?limit=10&starting_after=pi_0000000000000000000000")
            .body(Body::empty())
            .expect("a valid request");
        let (status, body) = send(request).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("pi_0000000000000000000000"), "{body}");
    }

    #[tokio::test]
    async fn a_rejection_is_the_stripe_envelope_and_not_axums_plain_text() {
        let (status, body) = send(form_request("amount=1&amount=2&currency=xaf")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let envelope: Value = serde_json::from_str(&body).expect("a JSON envelope");
        assert_eq!(
            envelope
                .get("error")
                .and_then(|e| e.get("type"))
                .and_then(Value::as_str),
            Some("invalid_request_error")
        );
        assert_eq!(
            envelope
                .get("error")
                .and_then(|e| e.get("param"))
                .and_then(Value::as_str),
            Some("amount")
        );
    }

    #[tokio::test]
    async fn a_json_body_is_told_to_send_a_form() {
        let request = Request::builder()
            .method("POST")
            .uri("/create")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"amount":"5000"}"#))
            .expect("a valid request");
        let (status, body) = send(request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("x-www-form-urlencoded"), "{body}");
    }

    /// The 64 KiB bound the module doc documents, proved over the same layer
    /// the `/v1` nest mounts. A body one byte over is refused **before** the
    /// extractor runs — note the response is a bare 413 from tower-http and
    /// not this crate's envelope, which is a known and deliberate wart: the
    /// alternative is buffering the megabyte in order to answer prettily.
    #[tokio::test]
    async fn a_body_over_the_limit_is_refused_by_the_layer() {
        // The two required fields are part of the body on purpose: without
        // them the handler answers 400 for a *missing field* and the
        // assertion below would pass or fail for a reason that has nothing
        // to do with the size bound this test exists to prove.
        let under = format!(
            "amount=5000&currency=xaf&description={}",
            "x".repeat(BODY_LIMIT_BYTES - 100)
        );
        let (status, body) = send(form_request(&under)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "just under the limit is fine: {body}"
        );

        let over = format!(
            "amount=5000&currency=xaf&description={}",
            "x".repeat(BODY_LIMIT_BYTES)
        );
        let (status, _) = send(form_request(&over)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn a_missing_required_field_is_a_400_naming_the_body() {
        let (status, body) = send(form_request("currency=xaf")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("\"param\":\"body\""), "{body}");
        assert!(
            !body.to_ascii_lowercase().contains("missing field"),
            "serde's own text must not be echoed: {body}"
        );
    }
}
