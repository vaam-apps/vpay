import type { JsonValue } from "./runtime.js";
import DecimalJs from "decimal.js";

// cratestack#498: every `Decimal`-typed schema field is carried as a real
// arbitrary-precision value, not a wire-format-dependent opaque `string`.
// #495/#496 made `decimal-bigdecimal` a real server-side backend for
// values beyond `rust_decimal`'s ~28-29 significant-digit cap, but
// `rust_decimal`'s `Display` never emits scientific notation while
// `bigdecimal`'s does past a magnitude threshold (`"0.0000001"` vs.
// `"1E-7"` for the identical value) — a client that just typed the field
// `string` saw a backend-dependent format and had no way to do
// arithmetic/comparison on it at all. `decimal.js` parses both notations
// into the identical value (`new Decimal("1E-7").equals(new
// Decimal("0.0000001"))` is `true`), closing that gap.
//
// Cloned with an effectively-unbounded exponential-notation threshold
// (`toExpNeg`/`toExpPos`) rather than used directly: decimal.js's own
// default `.toString()`/`.toJSON()` switches to scientific notation past
// ±7/21 orders of magnitude, which would make even an *ordinary* value
// like `0.00000001` re-encode as `"1e-8"`. Every outbound encode path
// goes through one of these two: `JSON.stringify` (`jsonRpcCodec`, REST
// bodies) or `encodeWireFields`'s own `.toString()` call (the native
// `@cratestack/cbor` codec, see that function's doc comment below) — so
// this one unbounded threshold governs all of them uniformly. Forcing
// plain positional notation matches `rust_decimal`'s own `Display`
// exactly, so a re-encoded value is always accepted back by a server on
// *either* backend without needing to know whether it parses scientific
// notation.
export const Decimal = DecimalJs.clone({ toExpNeg: -1e9, toExpPos: 1e9 });
export type Decimal = DecimalJs;

/** One model/`type`'s own decode-time shape: `decimalKeys` /
 *  `bigintKeys` / `bytesKeys` / `bytesListKeys` are its *direct* field wire
 *  names whose
 *  decoded values need converting out of their wire form, and `nested`
 *  maps a field name to the shape name (a {@link wireShapes} key) that
 *  field's own value should be revived against, for any field whose type
 *  is itself another declared model/`type`. See {@link wireShapes}'s doc
 *  comment for why this is keyed by structural path (via `nested`) rather
 *  than a single flat, schema-wide field-name set.
 *
 *  `Bytes` keys are split by arity because their wire forms are not
 *  self-identifying at every value: a populated `Bytes` is `number[]` and
 *  a populated `Bytes[]` is `number[][]`, but `[]` is both — an empty
 *  `Uint8Array` and an empty list of them are the same three characters
 *  on the wire. The schema knows which; the runtime cannot.
 *
 *  `bigintKeys` is one list for every arity: a `BigInt` leaf is a string
 *  and a `BigInt[]` is an array of strings, which {@link toBigInt} tells
 *  apart structurally. An empty `[]` needs no conversion either way.
 *
 *  `decimalKeys` likewise covers every arity, scalar and list: a `Decimal`
 *  leaf is a string and a `Decimal[]` is an array of strings, which
 *  {@link toDecimal} maps element by element. */
export interface WireShape {
  readonly decimalKeys: readonly string[];
  readonly bigintKeys: readonly string[];
  readonly bytesKeys: readonly string[];
  readonly bytesListKeys: readonly string[];
  readonly nested: Readonly<Record<string, string>>;
}

/** One entry per model/`type` this schema declares (`crate::wire_shapes::
 *  build_wire_shapes`) — `reviveWireFields`'s registry.
 *
 *  cratestack#499 review: an earlier version of this revival scheme kept a
 *  single flat `Set<string>` of every `Decimal` field name reachable from
 *  a response's root type (its own fields *and* every relation's/`type`'s
 *  fields, unioned together) and matched it against a decoded response's
 *  keys at *any* nesting depth. That's provably unsound the moment two
 *  *different* reachable types can each contribute a field name to the
 *  same flat set: a non-`Decimal` field in one type that happens to share
 *  a name with a `Decimal` field in another reachable type gets wrongly
 *  converted. Confirmed empirically (not just theorized) with an
 *  `Order.total: Decimal` + related `Account.total: String` schema,
 *  `include`-ing the relation: a real (non-numeric) account reference
 *  threw `[DecimalError] Invalid argument]` decoding a perfectly valid
 *  response, and a numeric-looking one (`"00123"`) was silently corrupted
 *  into `Decimal("123")`, losing its leading zeros.
 *
 *  This registry fixes that by keeping every type's `Decimal` field names
 *  in *that type's own* `WireShape` only, never merged with another
 *  type's. `reviveShaped` (below) looks up a nested field's *own* shape
 *  via `nested` rather than testing the parent's key set against it, so
 *  `Account.total` is only ever checked against *Account's* shape (which,
 *  correctly, has no `total` key) — not `Order`'s. */
export const wireShapes: Readonly<Record<string, WireShape>> = {
  Currency: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'payment_intents': 'PaymentIntent', 'charges': 'Charge', 'refunds': 'Refund', 'ledger_entries': 'LedgerEntry' } },
  Provider: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'charges': 'Charge' } },
  PaymentIntent: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'currency': 'Currency', 'customer': 'Customer', 'charges': 'Charge', 'refunds': 'Refund', 'checkout_sessions': 'CheckoutSession' } },
  Charge: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'payment_intent': 'PaymentIntent', 'provider': 'Provider', 'currency': 'Currency', 'refunds': 'Refund', 'ledger_transactions': 'LedgerTransaction' } },
  Refund: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'payment_intent': 'PaymentIntent', 'charge': 'Charge', 'currency': 'Currency' } },
  CheckoutSession: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'payment_intent': 'PaymentIntent', 'customer': 'Customer' } },
  LedgerTransaction: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'charge': 'Charge', 'entries': 'LedgerEntry' } },
  LedgerEntry: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'transaction': 'LedgerTransaction', 'currency': 'Currency' } },
  DisabledClient: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  Event: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  WebhookDelivery: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  Customer: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  StaffMember: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  Credential: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  Invoice: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  InvoiceItem: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  ManualPayment: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  RateLimitWindow: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  EnvelopeNonce: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  ConsoleKey: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  PaymentIntentSummary: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  PaymentIntentListFilter: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  RefundSummary: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  RefundListFilter: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  WebhookDeliverySummary: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  WebhookDeliveryListFilter: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  CustomerSummary: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  CustomerListFilter: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  CheckoutSessionSummary: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  CheckoutSessionListFilter: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  MetadataEntry: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  NamedValue: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  McpRedirect: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  McpNextAction: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'redirect_to_url': 'McpRedirect' } },
  McpLastPaymentError: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  McpPaymentMethodDetails: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  McpPaymentIntent: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'payment_method_details': 'McpPaymentMethodDetails', 'next_action': 'McpNextAction', 'last_payment_error': 'McpLastPaymentError', 'metadata': 'MetadataEntry' } },
  McpRefund: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'metadata': 'MetadataEntry' } },
  OpenPaymentIntentInput: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'metadata': 'MetadataEntry' } },
  ConfirmPaymentIntentInput: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'payer_fields': 'NamedValue' } },
  CancelInput: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: {  } },
  IssueRefundInput: { decimalKeys: [], bigintKeys: [], bytesKeys: [], bytesListKeys: [], nested: { 'destination_fields': 'NamedValue', 'metadata': 'MetadataEntry' } },
};

/** Decodes `value` against the named entry in {@link wireShapes},
 *  replacing every string (or, for a `Decimal[]` field, every element of
 *  an array of strings) at a key that shape's own `decimalKeys` names
 *  with a real {@link Decimal}, every canonical decimal string (or array
 *  of them) at a `bigintKeys` name with a `bigint`, and every wire integer
 *  array at a `bytesKeys`/`bytesListKeys` name with a `Uint8Array`, and
 *  recursing into any key `nested` names using *that* field's own shape.
 *
 *  A `bigintKeys` key holding anything but a canonical decimal string (or
 *  an array of them, or `null`) **throws** a `TypeError` naming the type
 *  and field. See {@link toBigInt}.
 *
 *  A `shapeName` not found in the registry (a plain scalar or enum
 *  return, e.g. `echoName(): string`) is a documented no-op fast path, so
 *  every model/procedure call site calls this unconditionally rather than
 *  the generator branching per call site. */
export function reviveWireFields(value: unknown, shapeName: string): unknown {
  const shape = wireShapes[shapeName];
  if (!shape) {
    return value;
  }
  return reviveShaped(value, shape, shapeName);
}

/** {@link reviveWireFields} for a `Page<T>` envelope: applies `T`'s own
 *  shape to `.items` (a `Page` envelope's own keys — `items`/`totalCount`/
 *  `pageInfo` — are never themselves `T`'s fields, so `T`'s shape can't be
 *  applied to the envelope directly the way a plain `T`/`T[]` response
 *  can), leaving `totalCount`/`pageInfo` untouched. */
export function revivePagedWireFields(value: unknown, shapeName: string): unknown {
  const shape = wireShapes[shapeName];
  if (!shape || value === null || typeof value !== "object" || !("items" in value)) {
    return value;
  }
  const page = value as { items: unknown };
  return { ...page, items: reviveShaped(page.items, shape, shapeName) };
}

function reviveShaped(value: unknown, shape: WireShape, shapeName: string): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => reviveShaped(item, shape, shapeName));
  }
  if (value !== null && typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      const nestedShapeName = shape.nested[key];
      if (nestedShapeName !== undefined) {
        const nestedShape = wireShapes[nestedShapeName];
        result[key] = nestedShape ? reviveShaped(entry, nestedShape, nestedShapeName) : entry;
      } else if (shape.decimalKeys.includes(key)) {
        result[key] = toDecimal(entry);
      } else if (shape.bigintKeys.includes(key)) {
        result[key] = toBigInt(entry, `${shapeName}.${key}`);
      } else if (shape.bytesKeys.includes(key)) {
        result[key] = toBytes(entry);
      } else if (shape.bytesListKeys.includes(key)) {
        result[key] = Array.isArray(entry) ? entry.map(toBytes) : entry;
      } else {
        result[key] = entry;
      }
    }
    return result;
  }
  return value;
}

/** One `Decimal` leaf (or an array of them), wire form -> {@link Decimal}.
 *
 *  The same conversion as {@link reviveWireScalar}'s `"decimal"` arm, for a
 *  field rather than a bare procedure return (cratestack#1155). A `Decimal[]`
 *  field shares the one `decimalKeys` list with a scalar `Decimal`, so an
 *  array is revived item by item here, as `reviveWireScalar` already does
 *  for a bare `Decimal[]` return. A `null` element, a `null` array and a
 *  non-string value pass through untouched, as they did before. */
function toDecimal(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(toDecimal);
  }
  return typeof value === "string" ? new Decimal(value) : value;
}

/** The canonical `BigInt` wire grammar (ADR 0019 D2): `0`, or an optional
 *  `-`, a non-zero digit and at most 18 more digits. No `+`, no leading
 *  zeros, no `-0`, no whitespace. The server writes only this form, so a
 *  string outside it is not something to normalise but something to refuse. */
const CANONICAL_BIGINT = /^(0|-?[1-9][0-9]{0,18})$/;
const I64_MIN = -(2n ** 63n);
const I64_MAX = 2n ** 63n - 1n;

/** One `BigInt` leaf (or an array of them), wire form -> `bigint`.
 *
 *  `null` passes through: a nullable `BigInt` column is `null` on the wire
 *  and `null` in the generated `bigint | null` type. An array is revived
 *  item by item, which is how a `BigInt[]` field decodes.
 *
 *  **Anything else throws**, unlike {@link toBytes}, which hands back what
 *  it was given. A `number` (or a `bigint`, which `@cratestack/cbor-node`
 *  returns for a large CBOR integer) at a `BigInt` key means the server
 *  predates the cutover, and a JSON number above 2^53 may already have been
 *  rounded on its way here. Reading it as the integer it appears to be would
 *  be a silent wrong answer, and passing it through would put a `number`
 *  under a `bigint` type. A string that is not the canonical form, or is
 *  outside `i64`, is refused for the same reason.
 *
 *  `where` is the `Type.field` the value was found at, for the message. */
function toBigInt(value: unknown, where: string): unknown {
  if (value === null || value === undefined) {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => toBigInt(item, where));
  }
  if (typeof value === "string" && CANONICAL_BIGINT.test(value)) {
    const parsed = BigInt(value);
    if (parsed >= I64_MIN && parsed <= I64_MAX) {
      return parsed;
    }
  }
  const found = typeof value === "object" ? "an object" : `${typeof value} ${String(value)}`;
  throw new TypeError(
    `BigInt field ${where}: expected a canonical decimal string, got ${found}`,
  );
}

/** One `Bytes` leaf, wire form -> `Uint8Array`.
 *
 *  `null` passes through untouched: a nullable `Bytes` column is `null`
 *  on the wire, and turning that into an empty `Uint8Array` would erase
 *  the difference between "no value" and "zero bytes" — the same
 *  distinction the generated `Uint8Array | null` field type keeps.
 *
 *  Anything that isn't an array is also passed through rather than
 *  coerced. The shape registry says this key *should* be `Bytes`, but a
 *  response that disagrees (a hand-rolled server, a proxy rewriting
 *  bodies, a schema/deployment skew) is better surfaced to the caller as
 *  the value it actually was than silently turned into empty bytes. */
function toBytes(value: unknown): unknown {
  return Array.isArray(value) ? Uint8Array.from(value as number[]) : value;
}

/** Counterpart to {@link reviveWireFields} for a procedure whose return
 *  type is a bare revivable scalar — `Decimal`, `BigInt` or `Bytes`, at any
 *  arity —
 *  rather than an object with such fields. {@link reviveWireFields} only
 *  walks object/array *containers* looking for shaped properties, so a
 *  raw top-level decoded value needs this simpler counterpart instead
 *  (cratestack#498 F2: a procedure like `quote(): Decimal` was previously
 *  declared `Decimal` but decoded as an untouched `string`).
 *
 *  `kind` comes from `crate::wire_shapes::ScalarRevival`, rendered into
 *  the call site by the generator, because the wire form alone cannot say
 *  which conversion is wanted: `Bytes` and `Bytes[]` are `number[]` and
 *  `number[][]` when populated but both `[]` when empty. An unrecognised
 *  `kind` is a no-op rather than a throw — a generated client should never
 *  produce one, and degrading to the raw decoded value beats failing a
 *  response that is otherwise perfectly readable.
 *
 *  See `procedure_views.rs::ProcedureView::revival_kind` for which
 *  generated call sites use this vs. {@link reviveWireFields}. */
export function reviveWireScalar(value: unknown, kind: string): unknown {
  if (kind === "bytes") {
    // A bare `Bytes`/`Bytes?` return: one wire integer array (or `null`).
    return toBytes(value);
  }
  if (kind === "bytesList") {
    // A bare `Bytes[]` return: an array *of* wire integer arrays. Handled
    // here rather than by recursing through the `"bytes"` branch, which
    // would see the outer array and convert it whole.
    return Array.isArray(value) ? value.map(toBytes) : value;
  }
  if (kind === "bigint") {
    // A bare `BigInt`, `BigInt?` or `BigInt[]` return. Strict, like the
    // field-level key: see {@link toBigInt}.
    return toBigInt(value, "procedure return");
  }
  if (kind !== "decimal") {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => reviveWireScalar(item, kind));
  }
  return typeof value === "string" ? new Decimal(value) : value;
}

/** Encode-side counterpart to {@link reviveWireFields}/
 *  {@link reviveWireScalar} (cratestack#746 follow-up): every RPC
 *  request body — `create`/`update` inputs, procedure arguments (plain,
 *  nested, or array-valued), a `FindMany<Model>` argument's
 *  `<Model>Where`/`DecimalFilter` operators, and each frame's own `input`
 *  inside a `batch()` payload — can carry a real {@link Decimal} instance
 *  wherever the schema declares a `Decimal`-typed field, and every one of
 *  those needs to reach the wire as a plain string before it reaches a
 *  codec's `encode()`.
 *
 *  `jsonRpcCodec`'s `JSON.stringify` already handles this for free via
 *  `Decimal.prototype.toJSON` (an alias for `.toString()`, see the
 *  `Decimal` export's own doc comment above) — but `@cratestack/cbor`
 *  (the default codec since #746) walks a value's *own enumerable
 *  properties* on its way to a `serde_json::Value` and never calls
 *  `toJSON`. `decimal.js`'s `clone()` (used to build this file's
 *  `Decimal` export) assigns `constructor` as an own enumerable property
 *  pointing at a function, and a function has no `serde_json::Value`
 *  representation — so encoding any request body containing a real
 *  `Decimal` instance under the native codec throws `JS functions cannot
 *  be represented as a serde_json::Value` before a request is ever sent.
 *  Confirmed against the actually-published `@cratestack/cbor`, not
 *  theorized.
 *
 *  A `bigint` (a `BigInt` field, ADR 0019) is converted here too, to its
 *  canonical decimal string, for BOTH codecs: that string is the one wire
 *  form on JSON and CBOR alike. Unlike a `Decimal`, a `bigint` is not merely
 *  awkward for the native codec but silently wrong: it would be encoded as a
 *  CBOR integer, which the server refuses for a `BigInt` field. Also unlike
 *  a `Decimal`, `JSON.stringify` has no `toJSON` to fall back on and throws,
 *  so the JSON codec needs {@link encodeBinaryAsJson}'s own `bigint` arm for
 *  the callers that never reach this function.
 *
 *  Deliberately NOT built on {@link wireShapes}/{@link reviveShaped}'s
 *  by-field-name registry, unlike the decode-side functions above: this
 *  direction already has a real, unambiguous {@link Decimal} instance in
 *  hand at every leaf, so `value instanceof Decimal` identifies exactly
 *  what needs converting with no risk of the field-name collision hazard
 *  {@link wireShapes}'s own doc comment (`crate::decimal`, cratestack#499)
 *  exists to avoid — and it needs no per-shape registry entry for
 *  `<Model>Where`/procedure-argument wrapper interfaces, neither of which
 *  {@link wireShapes} describes at all (it is built only for
 *  models/`type`s, which is all the *decode* side ever needs — see that
 *  module's doc comment). A plain recursive walk that only special-cases
 *  the one JS runtime type that can't survive the native codec covers
 *  every shape above uniformly.
 *
 *  Called unconditionally from `rpc-runtime.ts.j2`'s `terminalLink`/
 *  `rpc-stream-terminal.ts.j2`'s `terminalStreamLink` — the two places
 *  every unary/batch/stream request body reaches a codec's `encode()` —
 *  rather than gated on `native_cbor`: converting a `Decimal` to its
 *  `.toString()` form before `JSON.stringify` produces the byte-identical
 *  request body `jsonRpcCodec` already sends today (the same
 *  `.toString()` `Decimal.prototype.toJSON` calls), so applying it
 *  unconditionally is a true no-op for the JSON path and avoids a fourth
 *  `{% if native_cbor %}` branch in these already-branchy
 *  templates. */
/** Rewrites every `Uint8Array` in `value` into the plain integer array a
 *  `Bytes` field travels as on the wire, every `bigint` into its canonical
 *  decimal string, and every {@link Decimal} into its string, leaving
 *  everything else alone.
 *
 *  **JSON encode paths only** — {@link jsonRpcCodec}, the REST runtime's
 *  request body, and the REST runtime's JSON-encoded query values
 *  (`computedParams`). The native `@cratestack/cbor` codec must keep
 *  receiving the real `Uint8Array`, because it encodes one as a CBOR byte
 *  string (RFC 8949 major type 2, cratestack#783), which is both correct
 *  and about half the bytes of the integer array. Applying this
 *  unconditionally would throw that away.
 *
 *  The `bigint` and `Decimal` arms are the reason this walk, and not
 *  `JSON.stringify` alone, sits in front of every JSON encode.
 *  `JSON.stringify` throws on a `bigint` ("Do not know how to serialize a
 *  BigInt"), and a `Decimal` only survives because of its `toJSON`, which
 *  this walk would otherwise bypass: it rebuilds every object through
 *  `Object.entries`, turning a `Decimal` into its internal `{s, e, d}`
 *  fields. Converting both here means no JSON path can reach
 *  `JSON.stringify` holding either, whichever caller got there first
 *  (`@cratestack/link-batch` hands a codec raw inputs that
 *  {@link encodeWireFields} never saw).
 *
 *  `JSON.stringify` cannot do this itself: a `Uint8Array` has no `toJSON`,
 *  so it serializes as an index-keyed *object* (`{"0":1,"1":2}`) that no
 *  server-side `Vec<u8>` can decode — the same defect cratestack#783 fixed
 *  on the CBOR side, in a different disguise.
 *
 *  A `JSON.stringify` replacer would be cheaper than this pre-walk but is
 *  not sufficient: Node's `Buffer` is a `Uint8Array` subclass (so it is
 *  assignable to a generated `Bytes` field) *and* defines its own
 *  `toJSON`, which `JSON.stringify` applies **before** the replacer ever
 *  sees the value — yielding `{"type":"Buffer","data":[...]}`. Walking the
 *  value first sidesteps `toJSON` entirely, so a `Buffer` and a plain
 *  `Uint8Array` encode identically. Measured, not assumed. */
export function encodeBinaryAsJson(value: unknown): unknown {
  if (typeof value === "bigint" || value instanceof Decimal) {
    return value.toString();
  }
  if (value instanceof Uint8Array) {
    return Array.from(value);
  }
  if (Array.isArray(value)) {
    return value.map((item) => encodeBinaryAsJson(item));
  }
  // `instanceof Uint8Array` above already caught every typed array this
  // client's own types can produce; anything else object-shaped is walked
  // for nested binary. (`Decimal` and `bigint` were handled above, before
  // this walk can rebuild them.)
  if (value !== null && typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      result[key] = encodeBinaryAsJson(entry);
    }
    return result;
  }
  return value;
}

export function encodeWireFields(value: unknown): unknown {
  if (value instanceof Decimal) {
    return value.toString();
  }
  // A `BigInt` field travels as its canonical decimal string on every codec
  // (ADR 0019 D2), so unlike `Uint8Array` below there is no codec that wants
  // the real value. Required, not cosmetic: `JSON.stringify` throws on a
  // `bigint`, and the native codec would silently encode one as a CBOR
  // *integer*, which a server refuses (a `BigInt` is a text string there).
  // `bigint.toString()` is plain decimal (`i64`'s `Display`), so the output
  // is always the canonical form.
  if (typeof value === "bigint") {
    return value.toString();
  }
  // Pass a `Bytes` value through UNTOUCHED (cratestack#820). Load-bearing
  // and easy to delete by accident, because it looks like a no-op.
  //
  // A `Uint8Array` is not `Array.isArray`, so without this it falls into
  // the generic object branch below and is rebuilt through
  // `Object.entries` into `{"0":1,"1":2,"2":3}`. That happens before
  // EITHER codec runs — `terminalLink` calls this function
  // unconditionally — so it broke both of them at once: the native codec
  // emitted a CBOR map where a byte string belongs, and on the JSON path
  // `encodeBinaryAsJson` never saw a real `Uint8Array` to convert, so its
  // `Array.from` never fired either. Measured on a real generated client,
  // not theorized.
  //
  // Returning it as-is is correct for both paths rather than a
  // native-only carve-out: the native codec wants the real typed array
  // (RFC 8949 major type 2, cratestack#783), and the JSON path wants
  // `encodeBinaryAsJson` to receive one so it can produce the integer
  // array — which is exactly what this restores.
  if (value instanceof Uint8Array) {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => encodeWireFields(item));
  }
  if (value !== null && typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      result[key] = encodeWireFields(entry);
    }
    return result;
  }
  return value;
}

// Mirrors cratestack-core::page::{Page, PageInfo} exactly — this is
// the literal wire shape every `@@paged` list route serializes with
// `#[serde(rename_all = "camelCase")]`, not an independently designed
// client-side type. Keep field names and optionality in lockstep with
// that struct; do not add/rename fields here without changing it
// there first.
export interface PageInfo {
  limit: number | null;
  offset: number | null;
  hasNextPage: boolean;
  hasPreviousPage: boolean;
}

export interface Page<T> {
  items: T[];
  totalCount: number | null;
  pageInfo: PageInfo;
}

// Mirrors cratestack-core::page::PageInput exactly — the request-side
// counterpart to Page/PageInfo above, currently usable only as a
// procedure argument type. Keep field names and optionality in lockstep
// with that struct.
export interface PageInput {
  limit: number | null;
  offset: number | null;
}

// Shared building blocks for every `<Model>Where`/`<Model>FindMany`
// pair below (search-with-filters for procedures — mirrors
// cratestack-core::find_many::FieldFilterInput and
// cratestack-macros's per-model `<Model>Where`/`<Model>SortField`/
// `<Model>OrderByClause`/`<Model>FindManyInput` exactly). Usable only
// as a procedure argument type.
export interface EqualityFilter<V> {
  eq?: V;
  ne?: V;
  in?: V[];
  isNull?: boolean;
}

export interface ComparableFilter<V> extends EqualityFilter<V> {
  lt?: V;
  lte?: V;
  gt?: V;
  gte?: V;
}

export interface StringFilter extends ComparableFilter<string> {
  contains?: string;
  startsWith?: string;
}

export type NumberFilter = ComparableFilter<number>;
// ADR 0019: `bigint`, the exact `i64` counterpart to `NumberFilter`. Like
// `DecimalFilter` it only travels outbound, as part of a `<Model>Where`/
// `FindMany` procedure argument, and `encodeWireFields` turns each operand
// into the canonical decimal string the server reads.
export type BigIntFilter = ComparableFilter<bigint>;
export type BooleanFilter = EqualityFilter<boolean>;
export type UuidFilter = ComparableFilter<string>;
export type DateTimeFilter = ComparableFilter<string>;
// cratestack#498: `Decimal`, not `string` — see this file's own
// `Decimal`/`reviveWireFields` doc comments above for why. Unlike a
// response's own `Decimal` fields, a `DecimalFilter` only ever travels
// *outbound* as part of a `<Model>Where`/`FindMany` procedure argument
// (`find_many_views.rs`'s own doc comment — "usable only as a procedure
// argument type"), so it needs no `reviveWireFields` counterpart on
// decode. It DOES need `encodeWireFields` on encode, same as every
// other `Decimal`-carrying request body (see that function's own doc
// comment for why `Decimal.prototype.toJSON`/`JSON.stringify` alone isn't
// enough once `@cratestack/cbor` is the codec, cratestack#746 follow-up)
// — `rpc-runtime.ts.j2`'s `terminalLink` applies it uniformly to every
// outbound RPC request body, so a `FindMany<Model>` argument's `where`
// filter needs no separate glue at this call site either.
export type DecimalFilter = ComparableFilter<Decimal>;

export type SortDirection = "asc" | "desc";

export type ProviderFlow = 'push' | 'redirect';
export const ProviderFlowValues = [
  "push",
  "redirect",
] as const satisfies readonly ProviderFlow[];

export type IntentStatus = 'requires_payment_method' | 'requires_action' | 'processing' | 'succeeded' | 'canceled';
export const IntentStatusValues = [
  "requires_payment_method",
  "requires_action",
  "processing",
  "succeeded",
  "canceled",
] as const satisfies readonly IntentStatus[];

export type ChargeState = 'submitting' | 'submitted' | 'pending' | 'unresolved' | 'succeeded' | 'failed';
export const ChargeStateValues = [
  "submitting",
  "submitted",
  "pending",
  "unresolved",
  "succeeded",
  "failed",
] as const satisfies readonly ChargeState[];

export type FailureCode = 'insufficient_funds' | 'payer_timeout' | 'payer_declined' | 'invalid_payer' | 'payer_limit_reached' | 'payer_account_blocked' | 'invalid_payee' | 'payee_account_blocked' | 'provider_account_blocked' | 'provider_unavailable' | 'provider_error';
export const FailureCodeValues = [
  "insufficient_funds",
  "payer_timeout",
  "payer_declined",
  "invalid_payer",
  "payer_limit_reached",
  "payer_account_blocked",
  "invalid_payee",
  "payee_account_blocked",
  "provider_account_blocked",
  "provider_unavailable",
  "provider_error",
] as const satisfies readonly FailureCode[];

export type RefundStatus = 'pending' | 'succeeded' | 'failed' | 'canceled';
export const RefundStatusValues = [
  "pending",
  "succeeded",
  "failed",
  "canceled",
] as const satisfies readonly RefundStatus[];

export type AccountKind = 'merchant_payable' | 'payer_clearing' | 'platform_fee_revenue';
export const AccountKindValues = [
  "merchant_payable",
  "payer_clearing",
  "platform_fee_revenue",
] as const satisfies readonly AccountKind[];

export type Direction = 'debit' | 'credit';
export const DirectionValues = [
  "debit",
  "credit",
] as const satisfies readonly Direction[];

export type InvoiceStatus = 'draft' | 'open' | 'paid' | 'void' | 'uncollectible';
export const InvoiceStatusValues = [
  "draft",
  "open",
  "paid",
  "void",
  "uncollectible",
] as const satisfies readonly InvoiceStatus[];

export type ManualPaymentMethod = 'cash' | 'cheque' | 'bank_transfer' | 'other';
export const ManualPaymentMethodValues = [
  "cash",
  "cheque",
  "bank_transfer",
  "other",
] as const satisfies readonly ManualPaymentMethod[];

export type CurrencySortField = 'code' | 'exponent';
export const CurrencySortFieldValues = [
  "code",
  "exponent",
] as const satisfies readonly CurrencySortField[];

export type ProviderSortField = 'code' | 'display_name' | 'flow' | 'supports_refunds' | 'supports_partial_refunds' | 'delivers_callbacks' | 'requires_ip_allowlist' | 'enabled';
export const ProviderSortFieldValues = [
  "code",
  "display_name",
  "flow",
  "supports_refunds",
  "supports_partial_refunds",
  "delivers_callbacks",
  "requires_ip_allowlist",
  "enabled",
] as const satisfies readonly ProviderSortField[];

export type PaymentIntentSortField = 'id' | 'seq' | 'merchant_id' | 'livemode' | 'amount' | 'amount_received' | 'amount_refunded' | 'amount_refund_pending' | 'currency_code' | 'status' | 'last_payment_error_code' | 'last_payment_error_message' | 'description' | 'customer_id' | 'client_secret_suffix' | 'created_at' | 'updated_at';
export const PaymentIntentSortFieldValues = [
  "id",
  "seq",
  "merchant_id",
  "livemode",
  "amount",
  "amount_received",
  "amount_refunded",
  "amount_refund_pending",
  "currency_code",
  "status",
  "last_payment_error_code",
  "last_payment_error_message",
  "description",
  "customer_id",
  "client_secret_suffix",
  "created_at",
  "updated_at",
] as const satisfies readonly PaymentIntentSortField[];

export type ChargeSortField = 'id' | 'payment_intent_id' | 'provider_code' | 'provider_reference_id' | 'provider_txn_id' | 'redirect_url' | 'return_url' | 'state' | 'amount' | 'currency_code' | 'payer_ref' | 'payer_ref_masked' | 'failure_code' | 'failure_raw' | 'created_at' | 'updated_at';
export const ChargeSortFieldValues = [
  "id",
  "payment_intent_id",
  "provider_code",
  "provider_reference_id",
  "provider_txn_id",
  "redirect_url",
  "return_url",
  "state",
  "amount",
  "currency_code",
  "payer_ref",
  "payer_ref_masked",
  "failure_code",
  "failure_raw",
  "created_at",
  "updated_at",
] as const satisfies readonly ChargeSortField[];

export type RefundSortField = 'id' | 'payment_intent_id' | 'charge_id' | 'amount' | 'currency_code' | 'status' | 'reason' | 'failure_code' | 'failure_raw' | 'provider_reference_id' | 'fee' | 'created_at' | 'updated_at';
export const RefundSortFieldValues = [
  "id",
  "payment_intent_id",
  "charge_id",
  "amount",
  "currency_code",
  "status",
  "reason",
  "failure_code",
  "failure_raw",
  "provider_reference_id",
  "fee",
  "created_at",
  "updated_at",
] as const satisfies readonly RefundSortField[];

export type CheckoutSessionSortField = 'id' | 'seq' | 'merchant_id' | 'payment_intent_id' | 'livemode' | 'ui_mode' | 'status' | 'payment_status' | 'success_url' | 'cancel_url' | 'return_url' | 'customer_id' | 'publishable_key' | 'client_secret_suffix' | 'return_token' | 'expires_at' | 'created_at' | 'updated_at';
export const CheckoutSessionSortFieldValues = [
  "id",
  "seq",
  "merchant_id",
  "payment_intent_id",
  "livemode",
  "ui_mode",
  "status",
  "payment_status",
  "success_url",
  "cancel_url",
  "return_url",
  "customer_id",
  "publishable_key",
  "client_secret_suffix",
  "return_token",
  "expires_at",
  "created_at",
  "updated_at",
] as const satisfies readonly CheckoutSessionSortField[];

export type LedgerTransactionSortField = 'id' | 'charge_id' | 'created_at';
export const LedgerTransactionSortFieldValues = [
  "id",
  "charge_id",
  "created_at",
] as const satisfies readonly LedgerTransactionSortField[];

export type LedgerEntrySortField = 'id' | 'transaction_id' | 'account' | 'direction' | 'amount' | 'currency_code' | 'merchant_id';
export const LedgerEntrySortFieldValues = [
  "id",
  "transaction_id",
  "account",
  "direction",
  "amount",
  "currency_code",
  "merchant_id",
] as const satisfies readonly LedgerEntrySortField[];

export type DisabledClientSortField = 'client_id' | 'disabled_at' | 'reason';
export const DisabledClientSortFieldValues = [
  "client_id",
  "disabled_at",
  "reason",
] as const satisfies readonly DisabledClientSortField[];

export type EventSortField = 'id' | 'seq' | 'merchant_id' | 'livemode' | 'type' | 'object_id' | 'fanout_state' | 'created_at';
export const EventSortFieldValues = [
  "id",
  "seq",
  "merchant_id",
  "livemode",
  "type",
  "object_id",
  "fanout_state",
  "created_at",
] as const satisfies readonly EventSortField[];

export type WebhookDeliverySortField = 'id' | 'event_id' | 'endpoint_id' | 'url' | 'state' | 'response_excerpt' | 'payload_sha256' | 'created_at' | 'sent_at' | 'responded_at' | 'next_attempt_at';
export const WebhookDeliverySortFieldValues = [
  "id",
  "event_id",
  "endpoint_id",
  "url",
  "state",
  "response_excerpt",
  "payload_sha256",
  "created_at",
  "sent_at",
  "responded_at",
  "next_attempt_at",
] as const satisfies readonly WebhookDeliverySortField[];

export type CustomerSortField = 'id' | 'seq' | 'merchant_id' | 'livemode' | 'name' | 'email' | 'phone' | 'address_line1' | 'address_line2' | 'address_city' | 'address_state' | 'address_postal_code' | 'address_country' | 'address_latitude_microdeg' | 'address_longitude_microdeg' | 'last_used_at' | 'anonymized_at' | 'created_at' | 'updated_at';
export const CustomerSortFieldValues = [
  "id",
  "seq",
  "merchant_id",
  "livemode",
  "name",
  "email",
  "phone",
  "address_line1",
  "address_line2",
  "address_city",
  "address_state",
  "address_postal_code",
  "address_country",
  "address_latitude_microdeg",
  "address_longitude_microdeg",
  "last_used_at",
  "anonymized_at",
  "created_at",
  "updated_at",
] as const satisfies readonly CustomerSortField[];

export type StaffMemberSortField = 'id' | 'merchant_id' | 'email' | 'display_name' | 'status' | 'is_admin' | 'created_at' | 'updated_at' | 'last_sign_in_at';
export const StaffMemberSortFieldValues = [
  "id",
  "merchant_id",
  "email",
  "display_name",
  "status",
  "is_admin",
  "created_at",
  "updated_at",
  "last_sign_in_at",
] as const satisfies readonly StaffMemberSortField[];

export type CredentialSortField = 'id' | 'staff_member_id' | 'kind' | 'material' | 'issuer' | 'subject' | 'counter' | 'must_change' | 'expires_at' | 'created_at' | 'updated_at';
export const CredentialSortFieldValues = [
  "id",
  "staff_member_id",
  "kind",
  "material",
  "issuer",
  "subject",
  "counter",
  "must_change",
  "expires_at",
  "created_at",
  "updated_at",
] as const satisfies readonly CredentialSortField[];

export type InvoiceSortField = 'id' | 'seq' | 'merchant_id' | 'livemode' | 'customer_id' | 'currency_code' | 'status' | 'number' | 'amount_due' | 'amount_paid' | 'amount_remaining' | 'amount_refunded' | 'paid_out_of_band' | 'due_date' | 'description' | 'payment_intent_id' | 'finalized_at' | 'paid_at' | 'voided_at' | 'marked_uncollectible_at' | 'created_at' | 'updated_at';
export const InvoiceSortFieldValues = [
  "id",
  "seq",
  "merchant_id",
  "livemode",
  "customer_id",
  "currency_code",
  "status",
  "number",
  "amount_due",
  "amount_paid",
  "amount_remaining",
  "amount_refunded",
  "paid_out_of_band",
  "due_date",
  "description",
  "payment_intent_id",
  "finalized_at",
  "paid_at",
  "voided_at",
  "marked_uncollectible_at",
  "created_at",
  "updated_at",
] as const satisfies readonly InvoiceSortField[];

export type InvoiceItemSortField = 'id' | 'seq' | 'invoice_id' | 'merchant_id' | 'livemode' | 'description' | 'quantity' | 'unit_amount' | 'amount' | 'currency_code' | 'created_at' | 'updated_at';
export const InvoiceItemSortFieldValues = [
  "id",
  "seq",
  "invoice_id",
  "merchant_id",
  "livemode",
  "description",
  "quantity",
  "unit_amount",
  "amount",
  "currency_code",
  "created_at",
  "updated_at",
] as const satisfies readonly InvoiceItemSortField[];

export type ManualPaymentSortField = 'id' | 'merchant_id' | 'livemode' | 'invoice_id' | 'method' | 'reference' | 'received_at' | 'amount' | 'currency_code' | 'created_at' | 'paid_out_of_band';
export const ManualPaymentSortFieldValues = [
  "id",
  "merchant_id",
  "livemode",
  "invoice_id",
  "method",
  "reference",
  "received_at",
  "amount",
  "currency_code",
  "created_at",
  "paid_out_of_band",
] as const satisfies readonly ManualPaymentSortField[];

export type RateLimitWindowSortField = 'id' | 'scope' | 'window_started_at' | 'attempts' | 'updated_at';
export const RateLimitWindowSortFieldValues = [
  "id",
  "scope",
  "window_started_at",
  "attempts",
  "updated_at",
] as const satisfies readonly RateLimitWindowSortField[];

export type EnvelopeNonceSortField = 'id' | 'expires_at';
export const EnvelopeNonceSortFieldValues = [
  "id",
  "expires_at",
] as const satisfies readonly EnvelopeNonceSortField[];

export type ConsoleKeySortField = 'id' | 'staff_member_id' | 'alg' | 'public_key' | 'thumbprint' | 'label' | 'paired_at' | 'expires_at' | 'revoked_at';
export const ConsoleKeySortFieldValues = [
  "id",
  "staff_member_id",
  "alg",
  "public_key",
  "thumbprint",
  "label",
  "paired_at",
  "expires_at",
  "revoked_at",
] as const satisfies readonly ConsoleKeySortField[];

export interface PaymentIntentSummary {
  id: string;
  merchant_id: string;
  livemode: boolean;
  amount: number;
  amount_received: number;
  amount_refunded: number;
  amount_refund_pending: number;
  currency_code: string;
  status: IntentStatus;
  last_payment_error_code?: FailureCode | null;
  description?: string | null;
  customer_id?: string | null;
  created_at: string;
  updated_at: string;
}

export interface PaymentIntentListFilter {
  status?: string | null;
  created_gte?: string | null;
  created_lte?: string | null;
}

export interface RefundSummary {
  id: string;
  payment_intent_id: string;
  charge_id?: string | null;
  amount: number;
  currency_code: string;
  status: RefundStatus;
  reason?: string | null;
  failure_code?: FailureCode | null;
  provider_reference_id?: string | null;
  fee?: number | null;
  created_at: string;
  updated_at: string;
}

export interface RefundListFilter {
  status?: string | null;
  created_gte?: string | null;
  created_lte?: string | null;
}

export interface WebhookDeliverySummary {
  id: string;
  merchant_id: string;
  event_id: string;
  event_type: string;
  endpoint_id: string;
  url: string;
  state: string;
  created_at: string;
  sent_at?: string | null;
  responded_at?: string | null;
  next_attempt_at?: string | null;
}

export interface WebhookDeliveryListFilter {
  state?: string | null;
  event_type?: string | null;
  created_gte?: string | null;
  created_lte?: string | null;
}

export interface CustomerSummary {
  id: string;
  merchant_id: string;
  livemode: boolean;
  name?: string | null;
  email?: string | null;
  phone?: string | null;
  last_used_at: string;
  created_at: string;
  updated_at: string;
}

export interface CustomerListFilter {
  created_gte?: string | null;
  created_lte?: string | null;
  email?: string | null;
  phone?: string | null;
}

export interface CheckoutSessionSummary {
  id: string;
  merchant_id: string;
  payment_intent_id: string;
  livemode: boolean;
  ui_mode: string;
  status: string;
  payment_status: string;
  customer_id?: string | null;
  expires_at: string;
  created_at: string;
  updated_at: string;
}

export interface CheckoutSessionListFilter {
  status?: string | null;
  payment_status?: string | null;
  created_gte?: string | null;
  created_lte?: string | null;
}

export interface MetadataEntry {
  key: string;
  value: string;
}

export interface NamedValue {
  name: string;
  value: string;
}

export interface McpRedirect {
  url: string;
  return_url?: string | null;
}

export interface McpNextAction {
  kind: string;
  redirect_to_url?: McpRedirect | null;
}

export interface McpLastPaymentError {
  code: string;
  message: string;
}

export interface McpPaymentMethodDetails {
  method_type?: string | null;
  rail: string;
}

export interface McpPaymentIntent {
  id: string;
  amount: number;
  currency: string;
  status: string;
  payment_method_types: string[];
  payment_method_details?: McpPaymentMethodDetails | null;
  next_action?: McpNextAction | null;
  last_payment_error?: McpLastPaymentError | null;
  metadata: MetadataEntry[];
  description?: string | null;
  customer?: string | null;
  created: number;
  livemode: boolean;
}

export interface McpRefund {
  id: string;
  amount: number;
  currency: string;
  payment_intent: string;
  status: string;
  reason?: string | null;
  fee?: number | null;
  metadata: MetadataEntry[];
  created: number;
}

export interface OpenPaymentIntentInput {
  idempotency_key: string;
  amount: number;
  currency: string;
  payment_method_types: string[];
  description?: string | null;
  customer?: string | null;
  metadata: MetadataEntry[];
}

export interface ConfirmPaymentIntentInput {
  idempotency_key: string;
  id: string;
  payment_method_type: string;
  payer_fields: NamedValue[];
  return_url?: string | null;
}

export interface CancelInput {
  idempotency_key: string;
  id: string;
}

export interface IssueRefundInput {
  idempotency_key: string;
  payment_intent: string;
  amount?: number | null;
  reason?: string | null;
  destination_type?: string | null;
  destination_fields: NamedValue[];
  metadata: MetadataEntry[];
}

export interface Currency {
  code?: string;
  exponent?: number;
  payment_intents?: PaymentIntent[];
  charges?: Charge[];
  refunds?: Refund[];
  ledger_entries?: LedgerEntry[];
}

export interface CreateCurrencyInput {
  code: string;
  exponent: number;
}

export interface UpdateCurrencyInput {
  exponent?: number;
}

export interface CurrencyWhere {
  code?: StringFilter;
  exponent?: NumberFilter;
}

export interface CurrencyOrderByClause {
  field: CurrencySortField;
  direction: SortDirection;
}

export interface CurrencyFindMany {
  where?: CurrencyWhere;
  orderBy?: CurrencyOrderByClause[];
}

export interface Provider {
  code?: string;
  display_name?: string;
  flow?: ProviderFlow;
  supports_refunds?: boolean;
  supports_partial_refunds?: boolean;
  delivers_callbacks?: boolean;
  requires_ip_allowlist?: boolean;
  enabled?: boolean;
  charges?: Charge[];
}

export interface CreateProviderInput {
  code: string;
  display_name: string;
  flow: ProviderFlow;
  supports_refunds: boolean;
  supports_partial_refunds: boolean;
  delivers_callbacks: boolean;
  requires_ip_allowlist: boolean;
  enabled: boolean;
}

export interface UpdateProviderInput {
  display_name?: string;
  flow?: ProviderFlow;
  supports_refunds?: boolean;
  supports_partial_refunds?: boolean;
  delivers_callbacks?: boolean;
  requires_ip_allowlist?: boolean;
  enabled?: boolean;
}

export interface ProviderWhere {
  code?: StringFilter;
  display_name?: StringFilter;
  flow?: EqualityFilter<ProviderFlow>;
  supports_refunds?: BooleanFilter;
  supports_partial_refunds?: BooleanFilter;
  delivers_callbacks?: BooleanFilter;
  requires_ip_allowlist?: BooleanFilter;
  enabled?: BooleanFilter;
}

export interface ProviderOrderByClause {
  field: ProviderSortField;
  direction: SortDirection;
}

export interface ProviderFindMany {
  where?: ProviderWhere;
  orderBy?: ProviderOrderByClause[];
}

export interface PaymentIntent {
  id?: string;
  seq?: number;
  merchant_id?: string;
  livemode?: boolean;
  amount?: number;
  amount_received?: number;
  amount_refunded?: number;
  amount_refund_pending?: number;
  currency_code?: string;
  currency?: Currency;
  status?: IntentStatus;
  last_payment_error_code?: FailureCode | null;
  last_payment_error_message?: string | null;
  description?: string | null;
  customer_id?: string | null;
  customer?: Customer | null;
  client_secret_suffix?: string;
  created_at?: string;
  updated_at?: string;
  charges?: Charge[];
  refunds?: Refund[];
  checkout_sessions?: CheckoutSession[];
}

export interface UpdatePaymentIntentInput {
  seq?: number;
  merchant_id?: string;
  livemode?: boolean;
  amount?: number;
  amount_received?: number;
  amount_refunded?: number;
  amount_refund_pending?: number;
  currency_code?: string;
  status?: IntentStatus;
  last_payment_error_code?: FailureCode | null;
  last_payment_error_message?: string | null;
  description?: string | null;
  customer_id?: string | null;
  client_secret_suffix?: string;
  created_at?: string;
  updated_at?: string;
}

export interface PaymentIntentWhere {
  id?: StringFilter;
  seq?: NumberFilter;
  merchant_id?: StringFilter;
  livemode?: BooleanFilter;
  amount?: NumberFilter;
  amount_received?: NumberFilter;
  amount_refunded?: NumberFilter;
  amount_refund_pending?: NumberFilter;
  currency_code?: StringFilter;
  status?: EqualityFilter<IntentStatus>;
  last_payment_error_code?: EqualityFilter<FailureCode>;
  last_payment_error_message?: StringFilter;
  description?: StringFilter;
  customer_id?: StringFilter;
  client_secret_suffix?: StringFilter;
  created_at?: DateTimeFilter;
  updated_at?: DateTimeFilter;
}

export interface PaymentIntentOrderByClause {
  field: PaymentIntentSortField;
  direction: SortDirection;
}

export interface PaymentIntentFindMany {
  where?: PaymentIntentWhere;
  orderBy?: PaymentIntentOrderByClause[];
}

export interface Charge {
  id?: string;
  payment_intent_id?: string;
  payment_intent?: PaymentIntent;
  provider_code?: string;
  provider?: Provider;
  provider_reference_id?: string;
  provider_txn_id?: string | null;
  redirect_url?: string | null;
  return_url?: string | null;
  state?: ChargeState;
  amount?: number;
  currency_code?: string;
  currency?: Currency;
  payer_ref?: string | null;
  payer_ref_masked?: string | null;
  failure_code?: FailureCode | null;
  failure_raw?: string | null;
  created_at?: string;
  updated_at?: string;
  refunds?: Refund[];
  ledger_transactions?: LedgerTransaction[];
}

export interface UpdateChargeInput {
  payment_intent_id?: string;
  provider_code?: string;
  provider_reference_id?: string;
  provider_txn_id?: string | null;
  redirect_url?: string | null;
  return_url?: string | null;
  state?: ChargeState;
  amount?: number;
  currency_code?: string;
  payer_ref?: string | null;
  payer_ref_masked?: string | null;
  failure_code?: FailureCode | null;
  failure_raw?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface ChargeWhere {
  id?: StringFilter;
  payment_intent_id?: StringFilter;
  provider_code?: StringFilter;
  provider_reference_id?: UuidFilter;
  provider_txn_id?: StringFilter;
  redirect_url?: StringFilter;
  return_url?: StringFilter;
  state?: EqualityFilter<ChargeState>;
  amount?: NumberFilter;
  currency_code?: StringFilter;
  payer_ref?: StringFilter;
  payer_ref_masked?: StringFilter;
  failure_code?: EqualityFilter<FailureCode>;
  failure_raw?: StringFilter;
  created_at?: DateTimeFilter;
  updated_at?: DateTimeFilter;
}

export interface ChargeOrderByClause {
  field: ChargeSortField;
  direction: SortDirection;
}

export interface ChargeFindMany {
  where?: ChargeWhere;
  orderBy?: ChargeOrderByClause[];
}

export interface Refund {
  id?: string;
  payment_intent_id?: string;
  payment_intent?: PaymentIntent;
  charge_id?: string | null;
  charge?: Charge | null;
  amount?: number;
  currency_code?: string;
  currency?: Currency;
  status?: RefundStatus;
  reason?: string | null;
  failure_code?: FailureCode | null;
  failure_raw?: string | null;
  provider_reference_id?: string | null;
  fee?: number | null;
  created_at?: string;
  updated_at?: string;
}

export interface UpdateRefundInput {
  payment_intent_id?: string;
  charge_id?: string | null;
  amount?: number;
  currency_code?: string;
  status?: RefundStatus;
  reason?: string | null;
  failure_code?: FailureCode | null;
  failure_raw?: string | null;
  provider_reference_id?: string | null;
  fee?: number | null;
  created_at?: string;
  updated_at?: string;
}

export interface RefundWhere {
  id?: StringFilter;
  payment_intent_id?: StringFilter;
  charge_id?: StringFilter;
  amount?: NumberFilter;
  currency_code?: StringFilter;
  status?: EqualityFilter<RefundStatus>;
  reason?: StringFilter;
  failure_code?: EqualityFilter<FailureCode>;
  failure_raw?: StringFilter;
  provider_reference_id?: UuidFilter;
  fee?: NumberFilter;
  created_at?: DateTimeFilter;
  updated_at?: DateTimeFilter;
}

export interface RefundOrderByClause {
  field: RefundSortField;
  direction: SortDirection;
}

export interface RefundFindMany {
  where?: RefundWhere;
  orderBy?: RefundOrderByClause[];
}

export interface CheckoutSession {
  id?: string;
  seq?: number;
  merchant_id?: string;
  payment_intent_id?: string;
  payment_intent?: PaymentIntent;
  livemode?: boolean;
  ui_mode?: string;
  status?: string;
  payment_status?: string;
  success_url?: string | null;
  cancel_url?: string | null;
  return_url?: string | null;
  customer_id?: string | null;
  customer?: Customer | null;
  publishable_key?: string;
  client_secret_suffix?: string;
  return_token?: string;
  expires_at?: string;
  created_at?: string;
  updated_at?: string;
}

export interface UpdateCheckoutSessionInput {
  seq?: number;
  merchant_id?: string;
  payment_intent_id?: string;
  livemode?: boolean;
  ui_mode?: string;
  status?: string;
  payment_status?: string;
  success_url?: string | null;
  cancel_url?: string | null;
  return_url?: string | null;
  customer_id?: string | null;
  publishable_key?: string;
  client_secret_suffix?: string;
  return_token?: string;
  expires_at?: string;
  created_at?: string;
  updated_at?: string;
}

export interface CheckoutSessionWhere {
  id?: StringFilter;
  seq?: NumberFilter;
  merchant_id?: StringFilter;
  payment_intent_id?: StringFilter;
  livemode?: BooleanFilter;
  ui_mode?: StringFilter;
  status?: StringFilter;
  payment_status?: StringFilter;
  success_url?: StringFilter;
  cancel_url?: StringFilter;
  return_url?: StringFilter;
  customer_id?: StringFilter;
  publishable_key?: StringFilter;
  client_secret_suffix?: StringFilter;
  return_token?: StringFilter;
  expires_at?: DateTimeFilter;
  created_at?: DateTimeFilter;
  updated_at?: DateTimeFilter;
}

export interface CheckoutSessionOrderByClause {
  field: CheckoutSessionSortField;
  direction: SortDirection;
}

export interface CheckoutSessionFindMany {
  where?: CheckoutSessionWhere;
  orderBy?: CheckoutSessionOrderByClause[];
}

export interface LedgerTransaction {
  id?: string;
  charge_id?: string;
  charge?: Charge;
  created_at?: string;
  entries?: LedgerEntry[];
}

export interface UpdateLedgerTransactionInput {
  charge_id?: string;
  created_at?: string;
}

export interface LedgerTransactionWhere {
  id?: StringFilter;
  charge_id?: StringFilter;
  created_at?: DateTimeFilter;
}

export interface LedgerTransactionOrderByClause {
  field: LedgerTransactionSortField;
  direction: SortDirection;
}

export interface LedgerTransactionFindMany {
  where?: LedgerTransactionWhere;
  orderBy?: LedgerTransactionOrderByClause[];
}

export interface LedgerEntry {
  id?: string;
  transaction_id?: string;
  transaction?: LedgerTransaction;
  account?: AccountKind;
  direction?: Direction;
  amount?: number;
  currency_code?: string;
  currency?: Currency;
  merchant_id?: string | null;
}

export interface UpdateLedgerEntryInput {
  transaction_id?: string;
  account?: AccountKind;
  direction?: Direction;
  amount?: number;
  currency_code?: string;
  merchant_id?: string | null;
}

export interface LedgerEntryWhere {
  id?: StringFilter;
  transaction_id?: StringFilter;
  account?: EqualityFilter<AccountKind>;
  direction?: EqualityFilter<Direction>;
  amount?: NumberFilter;
  currency_code?: StringFilter;
  merchant_id?: StringFilter;
}

export interface LedgerEntryOrderByClause {
  field: LedgerEntrySortField;
  direction: SortDirection;
}

export interface LedgerEntryFindMany {
  where?: LedgerEntryWhere;
  orderBy?: LedgerEntryOrderByClause[];
}

export interface DisabledClient {
  client_id?: string;
  disabled_at?: string;
  reason?: string | null;
}

export interface CreateDisabledClientInput {
  client_id: string;
  reason?: string | null;
}

export interface UpdateDisabledClientInput {
  disabled_at?: string;
  reason?: string | null;
}

export interface DisabledClientWhere {
  client_id?: StringFilter;
  disabled_at?: DateTimeFilter;
  reason?: StringFilter;
}

export interface DisabledClientOrderByClause {
  field: DisabledClientSortField;
  direction: SortDirection;
}

export interface DisabledClientFindMany {
  where?: DisabledClientWhere;
  orderBy?: DisabledClientOrderByClause[];
}

export interface Event {
  id?: string;
  seq?: number;
  merchant_id?: string;
  livemode?: boolean;
  type?: string;
  object_id?: string;
  fanout_state?: string;
  created_at?: string;
}

export interface UpdateEventInput {
  seq?: number;
  merchant_id?: string;
  livemode?: boolean;
  type?: string;
  object_id?: string;
  fanout_state?: string;
  created_at?: string;
}

export interface EventWhere {
  id?: StringFilter;
  seq?: NumberFilter;
  merchant_id?: StringFilter;
  livemode?: BooleanFilter;
  type?: StringFilter;
  object_id?: StringFilter;
  fanout_state?: StringFilter;
  created_at?: DateTimeFilter;
}

export interface EventOrderByClause {
  field: EventSortField;
  direction: SortDirection;
}

export interface EventFindMany {
  where?: EventWhere;
  orderBy?: EventOrderByClause[];
}

export interface WebhookDelivery {
  id?: string;
  event_id?: string;
  endpoint_id?: string;
  url?: string;
  state?: string;
  response_excerpt?: string | null;
  payload_sha256?: string | null;
  created_at?: string;
  sent_at?: string | null;
  responded_at?: string | null;
  next_attempt_at?: string | null;
}

export interface CreateWebhookDeliveryInput {
  id: string;
  event_id: string;
  endpoint_id: string;
  url: string;
  response_excerpt?: string | null;
  payload_sha256?: string | null;
  sent_at?: string | null;
  responded_at?: string | null;
  next_attempt_at?: string | null;
}

export interface UpdateWebhookDeliveryInput {
  event_id?: string;
  endpoint_id?: string;
  url?: string;
  state?: string;
  response_excerpt?: string | null;
  payload_sha256?: string | null;
  created_at?: string;
  sent_at?: string | null;
  responded_at?: string | null;
  next_attempt_at?: string | null;
}

export interface WebhookDeliveryWhere {
  id?: UuidFilter;
  event_id?: StringFilter;
  endpoint_id?: StringFilter;
  url?: StringFilter;
  state?: StringFilter;
  response_excerpt?: StringFilter;
  payload_sha256?: StringFilter;
  created_at?: DateTimeFilter;
  sent_at?: DateTimeFilter;
  responded_at?: DateTimeFilter;
  next_attempt_at?: DateTimeFilter;
}

export interface WebhookDeliveryOrderByClause {
  field: WebhookDeliverySortField;
  direction: SortDirection;
}

export interface WebhookDeliveryFindMany {
  where?: WebhookDeliveryWhere;
  orderBy?: WebhookDeliveryOrderByClause[];
}

export interface Customer {
  id?: string;
  seq?: number;
  merchant_id?: string;
  livemode?: boolean;
  name?: string | null;
  email?: string | null;
  phone?: string | null;
  address_line1?: string | null;
  address_line2?: string | null;
  address_city?: string | null;
  address_state?: string | null;
  address_postal_code?: string | null;
  address_country?: string | null;
  address_latitude_microdeg?: number | null;
  address_longitude_microdeg?: number | null;
  last_used_at?: string;
  anonymized_at?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface UpdateCustomerInput {
  seq?: number;
  merchant_id?: string;
  livemode?: boolean;
  name?: string | null;
  email?: string | null;
  phone?: string | null;
  address_line1?: string | null;
  address_line2?: string | null;
  address_city?: string | null;
  address_state?: string | null;
  address_postal_code?: string | null;
  address_country?: string | null;
  address_latitude_microdeg?: number | null;
  address_longitude_microdeg?: number | null;
  last_used_at?: string;
  anonymized_at?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface CustomerWhere {
  id?: StringFilter;
  seq?: NumberFilter;
  merchant_id?: StringFilter;
  livemode?: BooleanFilter;
  name?: StringFilter;
  email?: StringFilter;
  phone?: StringFilter;
  address_line1?: StringFilter;
  address_line2?: StringFilter;
  address_city?: StringFilter;
  address_state?: StringFilter;
  address_postal_code?: StringFilter;
  address_country?: StringFilter;
  address_latitude_microdeg?: NumberFilter;
  address_longitude_microdeg?: NumberFilter;
  last_used_at?: DateTimeFilter;
  anonymized_at?: DateTimeFilter;
  created_at?: DateTimeFilter;
  updated_at?: DateTimeFilter;
}

export interface CustomerOrderByClause {
  field: CustomerSortField;
  direction: SortDirection;
}

export interface CustomerFindMany {
  where?: CustomerWhere;
  orderBy?: CustomerOrderByClause[];
}

export interface StaffMember {
  id?: string;
  merchant_id?: string;
  email?: string;
  display_name?: string;
  status?: string;
  is_admin?: boolean;
  created_at?: string;
  updated_at?: string;
  last_sign_in_at?: string | null;
}

export interface CreateStaffMemberInput {
  id: string;
  merchant_id: string;
  email: string;
  display_name: string;
  status: string;
  is_admin: boolean;
  created_at: string;
  updated_at: string;
  last_sign_in_at?: string | null;
}

export interface UpdateStaffMemberInput {
  merchant_id?: string;
  email?: string;
  display_name?: string;
  status?: string;
  is_admin?: boolean;
  created_at?: string;
  updated_at?: string;
  last_sign_in_at?: string | null;
}

export interface StaffMemberWhere {
  id?: StringFilter;
  merchant_id?: StringFilter;
  email?: StringFilter;
  display_name?: StringFilter;
  status?: StringFilter;
  is_admin?: BooleanFilter;
  created_at?: DateTimeFilter;
  updated_at?: DateTimeFilter;
  last_sign_in_at?: DateTimeFilter;
}

export interface StaffMemberOrderByClause {
  field: StaffMemberSortField;
  direction: SortDirection;
}

export interface StaffMemberFindMany {
  where?: StaffMemberWhere;
  orderBy?: StaffMemberOrderByClause[];
}

export interface Credential {
  id?: string;
  staff_member_id?: string | null;
  kind?: string;
  material?: string | null;
  issuer?: string | null;
  subject?: string | null;
  counter?: number;
  must_change?: boolean;
  expires_at?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface CreateCredentialInput {
  id: string;
  staff_member_id?: string | null;
  kind: string;
  material?: string | null;
  issuer?: string | null;
  subject?: string | null;
  counter: number;
  must_change: boolean;
  expires_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface UpdateCredentialInput {
  staff_member_id?: string | null;
  kind?: string;
  material?: string | null;
  issuer?: string | null;
  subject?: string | null;
  counter?: number;
  must_change?: boolean;
  expires_at?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface CredentialWhere {
  id?: StringFilter;
  staff_member_id?: StringFilter;
  kind?: StringFilter;
  material?: StringFilter;
  issuer?: StringFilter;
  subject?: StringFilter;
  counter?: NumberFilter;
  must_change?: BooleanFilter;
  expires_at?: DateTimeFilter;
  created_at?: DateTimeFilter;
  updated_at?: DateTimeFilter;
}

export interface CredentialOrderByClause {
  field: CredentialSortField;
  direction: SortDirection;
}

export interface CredentialFindMany {
  where?: CredentialWhere;
  orderBy?: CredentialOrderByClause[];
}

export interface Invoice {
  id?: string;
  seq?: number;
  merchant_id?: string;
  livemode?: boolean;
  customer_id?: string;
  currency_code?: string;
  status?: InvoiceStatus;
  number?: string | null;
  amount_due?: number;
  amount_paid?: number;
  amount_remaining?: number;
  amount_refunded?: number;
  paid_out_of_band?: boolean;
  due_date?: string | null;
  description?: string | null;
  payment_intent_id?: string | null;
  finalized_at?: string | null;
  paid_at?: string | null;
  voided_at?: string | null;
  marked_uncollectible_at?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface UpdateInvoiceInput {
  seq?: number;
  merchant_id?: string;
  livemode?: boolean;
  customer_id?: string;
  currency_code?: string;
  status?: InvoiceStatus;
  number?: string | null;
  amount_due?: number;
  amount_paid?: number;
  amount_remaining?: number;
  amount_refunded?: number;
  paid_out_of_band?: boolean;
  due_date?: string | null;
  description?: string | null;
  payment_intent_id?: string | null;
  finalized_at?: string | null;
  paid_at?: string | null;
  voided_at?: string | null;
  marked_uncollectible_at?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface InvoiceWhere {
  id?: StringFilter;
  seq?: NumberFilter;
  merchant_id?: StringFilter;
  livemode?: BooleanFilter;
  customer_id?: StringFilter;
  currency_code?: StringFilter;
  status?: EqualityFilter<InvoiceStatus>;
  number?: StringFilter;
  amount_due?: NumberFilter;
  amount_paid?: NumberFilter;
  amount_remaining?: NumberFilter;
  amount_refunded?: NumberFilter;
  paid_out_of_band?: BooleanFilter;
  due_date?: DateTimeFilter;
  description?: StringFilter;
  payment_intent_id?: StringFilter;
  finalized_at?: DateTimeFilter;
  paid_at?: DateTimeFilter;
  voided_at?: DateTimeFilter;
  marked_uncollectible_at?: DateTimeFilter;
  created_at?: DateTimeFilter;
  updated_at?: DateTimeFilter;
}

export interface InvoiceOrderByClause {
  field: InvoiceSortField;
  direction: SortDirection;
}

export interface InvoiceFindMany {
  where?: InvoiceWhere;
  orderBy?: InvoiceOrderByClause[];
}

export interface InvoiceItem {
  id?: string;
  seq?: number;
  invoice_id?: string;
  merchant_id?: string;
  livemode?: boolean;
  description?: string;
  quantity?: number;
  unit_amount?: number;
  amount?: number;
  currency_code?: string;
  created_at?: string;
  updated_at?: string;
}

export interface UpdateInvoiceItemInput {
  seq?: number;
  invoice_id?: string;
  merchant_id?: string;
  livemode?: boolean;
  description?: string;
  quantity?: number;
  unit_amount?: number;
  amount?: number;
  currency_code?: string;
  created_at?: string;
  updated_at?: string;
}

export interface InvoiceItemWhere {
  id?: StringFilter;
  seq?: NumberFilter;
  invoice_id?: StringFilter;
  merchant_id?: StringFilter;
  livemode?: BooleanFilter;
  description?: StringFilter;
  quantity?: NumberFilter;
  unit_amount?: NumberFilter;
  amount?: NumberFilter;
  currency_code?: StringFilter;
  created_at?: DateTimeFilter;
  updated_at?: DateTimeFilter;
}

export interface InvoiceItemOrderByClause {
  field: InvoiceItemSortField;
  direction: SortDirection;
}

export interface InvoiceItemFindMany {
  where?: InvoiceItemWhere;
  orderBy?: InvoiceItemOrderByClause[];
}

export interface ManualPayment {
  id?: string;
  merchant_id?: string;
  livemode?: boolean;
  invoice_id?: string;
  method?: ManualPaymentMethod;
  reference?: string | null;
  received_at?: string;
  amount?: number;
  currency_code?: string;
  created_at?: string;
  paid_out_of_band?: boolean;
}

export interface UpdateManualPaymentInput {
  merchant_id?: string;
  livemode?: boolean;
  invoice_id?: string;
  method?: ManualPaymentMethod;
  reference?: string | null;
  received_at?: string;
  amount?: number;
  currency_code?: string;
  created_at?: string;
  paid_out_of_band?: boolean;
}

export interface ManualPaymentWhere {
  id?: StringFilter;
  merchant_id?: StringFilter;
  livemode?: BooleanFilter;
  invoice_id?: StringFilter;
  method?: EqualityFilter<ManualPaymentMethod>;
  reference?: StringFilter;
  received_at?: DateTimeFilter;
  amount?: NumberFilter;
  currency_code?: StringFilter;
  created_at?: DateTimeFilter;
  paid_out_of_band?: BooleanFilter;
}

export interface ManualPaymentOrderByClause {
  field: ManualPaymentSortField;
  direction: SortDirection;
}

export interface ManualPaymentFindMany {
  where?: ManualPaymentWhere;
  orderBy?: ManualPaymentOrderByClause[];
}

export interface RateLimitWindow {
  id?: string;
  scope?: string;
  window_started_at?: string;
  attempts?: number;
  updated_at?: string;
}

export interface UpdateRateLimitWindowInput {
  scope?: string;
  window_started_at?: string;
  attempts?: number;
  updated_at?: string;
}

export interface RateLimitWindowWhere {
  id?: StringFilter;
  scope?: StringFilter;
  window_started_at?: DateTimeFilter;
  attempts?: NumberFilter;
  updated_at?: DateTimeFilter;
}

export interface RateLimitWindowOrderByClause {
  field: RateLimitWindowSortField;
  direction: SortDirection;
}

export interface RateLimitWindowFindMany {
  where?: RateLimitWindowWhere;
  orderBy?: RateLimitWindowOrderByClause[];
}

export interface EnvelopeNonce {
  id?: string;
  expires_at?: string;
}

export interface UpdateEnvelopeNonceInput {
  expires_at?: string;
}

export interface EnvelopeNonceWhere {
  id?: StringFilter;
  expires_at?: DateTimeFilter;
}

export interface EnvelopeNonceOrderByClause {
  field: EnvelopeNonceSortField;
  direction: SortDirection;
}

export interface EnvelopeNonceFindMany {
  where?: EnvelopeNonceWhere;
  orderBy?: EnvelopeNonceOrderByClause[];
}

export interface ConsoleKey {
  id?: string;
  staff_member_id?: string;
  alg?: string;
  public_key?: string;
  thumbprint?: string;
  label?: string | null;
  paired_at?: string;
  expires_at?: string;
  revoked_at?: string | null;
}

export interface CreateConsoleKeyInput {
  id: string;
  staff_member_id: string;
  alg: string;
  public_key: string;
  thumbprint: string;
  label?: string | null;
  paired_at: string;
  expires_at: string;
  revoked_at?: string | null;
}

export interface UpdateConsoleKeyInput {
  staff_member_id?: string;
  alg?: string;
  public_key?: string;
  thumbprint?: string;
  label?: string | null;
  paired_at?: string;
  expires_at?: string;
  revoked_at?: string | null;
}

export interface ConsoleKeyWhere {
  id?: StringFilter;
  staff_member_id?: StringFilter;
  alg?: StringFilter;
  public_key?: StringFilter;
  thumbprint?: StringFilter;
  label?: StringFilter;
  paired_at?: DateTimeFilter;
  expires_at?: DateTimeFilter;
  revoked_at?: DateTimeFilter;
}

export interface ConsoleKeyOrderByClause {
  field: ConsoleKeySortField;
  direction: SortDirection;
}

export interface ConsoleKeyFindMany {
  where?: ConsoleKeyWhere;
  orderBy?: ConsoleKeyOrderByClause[];
}

export interface SearchPaymentIntentsArgs {
  page: PageInput;
  filter: PaymentIntentListFilter;
}

export interface SearchRefundsArgs {
  page: PageInput;
  filter: RefundListFilter;
}

export interface SearchWebhookDeliveriesArgs {
  page: PageInput;
  filter: WebhookDeliveryListFilter;
}

export interface SearchCustomersArgs {
  page: PageInput;
  filter: CustomerListFilter;
}

export interface SearchCheckoutSessionsArgs {
  page: PageInput;
  filter: CheckoutSessionListFilter;
}

export interface RetrievePaymentIntentArgs {
  id: string;
}

export interface OpenPaymentIntentArgs {
  args: OpenPaymentIntentInput;
}

export interface ConfirmPaymentIntentArgs {
  args: ConfirmPaymentIntentInput;
}

export interface CancelPaymentIntentArgs {
  args: CancelInput;
}

export interface RetrieveRefundArgs {
  id: string;
}

export interface IssueRefundArgs {
  args: IssueRefundInput;
}

export interface CancelRefundArgs {
  args: CancelInput;
}

