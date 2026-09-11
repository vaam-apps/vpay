# `vpay-api` — the form decoder (`form.rs`)

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## The form decoder (`form.rs`)

The Stripe-style bracket-nested `application/x-www-form-urlencoded` decoder, and
the two extractors (`VpayForm`, `VpayQuery`) that put it in front of a handler.

This is the _reading_ half of a wire contract whose writing half already ships in
two SDKs — `sdks/rust/src/form.rs` and `sdks/nodejs/src/form.ts`, which are
byte-for-byte identical to each other by test. It is therefore a deliberate
port, not a general-purpose form parser: every rule below is chosen because it
is what that encoder emits, and the `node_parity` test module decodes the exact
byte strings those SDKs' own parity tests pin. If this file and those disagree, a
merchant's request means one thing to their SDK and another to us.

### Why not `serde_urlencoded` (what `axum::Form` uses)

It is a _flat_ decoder: `metadata[order_id]=1234` becomes a key literally
spelled `metadata[order_id]`, and `payment_method_types[0]=a` with
`payment_method_types[1]=b` becomes two unrelated fields. Nesting is the whole
encoding here ([merchant-auth.md](../../flows/merchant-auth.md)'s table), so the
shape has to be rebuilt rather than deserialized directly.

### The grammar

A body is `pair(&pair)*`, a pair is `key=value` (a pair with no `=` is a key
with an empty value), and a key is a head segment followed by zero or more
bracket groups:

```text
amount                                   -> ["amount"]
metadata[order_id]                       -> ["metadata", "order_id"]
payment_method_data[mtn_momo][msisdn]    -> ["payment_method_data", "mtn_momo", "msisdn"]
payment_method_types[0]                  -> ["payment_method_types", 0]
payment_method_types[]                   -> ["payment_method_types", next]
```

A bracket group holding only digits, or nothing at all, makes the parent an
**array**; anything else makes it an **object**. Both array spellings are
accepted because both are in the contract: the SDKs send `[0]`, `[1]` (as
`stripe-node`/`stripe-rust` do), and `examples/merchant-curl` and Stripe's own
curl documentation use `[]`.

### Brackets are split before segments are decoded, and that ordering is load-bearing

The encoder escapes a `[` _inside_ a key segment as `%5B` precisely so it cannot
be mistaken for nesting, and its own test
`escapes_a_bracket_that_appears_inside_a_key_segment` pins `metadata[a%5Bb]=v`.
Decoding the whole key first would turn that back into `metadata[a[b]` and the
split would then invent a nesting level the merchant never asked for — so the
split runs on the raw key and each segment is percent-decoded afterwards,
yielding the key `a[b`. (Step 2's design states the two operations in the other
order; the example it states in the same sentence is what fixes the order, and it
is the example that is testable. See
`metadata_key_with_an_escaped_bracket_is_one_key`.)

### `+` is a literal plus, never a space

Both SDKs escape with JavaScript's `encodeURIComponent`, which renders a space as
`%20` and leaves `+` alone. Applying the WHATWG
`application/x-www-form-urlencoded` rule (`+` → space) would silently corrupt
exactly the field that can least afford it: an MSISDN written `+237670000000`.
`serde_urlencoded`, and therefore `axum::Form`, does apply that rule — which is
the second reason this decoder exists.

### Everything is a string

Form encoding has no types: `amount=5000` and `description=5000` arrive
identically. So `parse_form` produces `serde_json::Value::String` for every
scalar, and a `T` deserialized through `VpayForm` must take `String` fields (or
carry its own `deserialize_with`). That is not a limitation being tolerated — it
is what lets a handler answer "amount must be a positive integer of minor units"
in vpay's own words, with `param: "amount"`, instead of leaking serde's sentence
for a field the caller can see.

### Bounds

The request body limit is **64 KiB**, applied by a
`tower_http::limit::RequestBodyLimitLayer` on the `/v1` nest rather than in the
decoder: a limit enforced by an extractor has already buffered the body it is
refusing. Over that, the layer answers `413` before this code runs — see
`a_body_over_the_limit_is_refused_by_the_layer`, which mounts the same layer over
these extractors. Nesting is bounded to `MAX_DEPTH` independently, because
64 KiB of `a[a][a][a]…` is cheap to send and recursion is not.

---
