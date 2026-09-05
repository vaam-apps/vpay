# Issue #45 — `GET /v1/refunds/{id}` becomes part of the contract and is served

Implementation notes. Branch `claude/issue-45-refund-retrieve`, base
`65a5952` (master). Written 2026-09-05.

## What the maintainer decided, and what that did and did not authorise

The issue asked for a contract decision and offered two shapes for it. The
decision taken was the stronger one: **`GET /v1/refunds/{id}` is part of the
`/v1` contract *and is served***, because the port's own rule — "`query_status`
is the authoritative read … must work indefinitely"
([flows/provider-port.md](../../flows/provider-port.md)) — applies to every
money movement, and a refund was the only one without it. A webhook is not a
substitute when delivery is at-least-once and unordered
([flows/webhooks.md](../../flows/webhooks.md)), and the two refund event types
are emitted by nothing anyway.

That decision authorised a **read**. It did not authorise:

- `POST /v1/refunds`, which stays declared and unrouted — creating a refund
  needs `ProviderAdapter::refund`, `NotImplemented` on MTN (Disbursements is a
  different product) and `Unsupported` on Orange;
- any refund event — `charge.refunded` and `charge.refund.updated` are still
  written by nothing;
- any change to the `refund` object's field list. Issue #46 is adding `fee` to
  the same object on another branch; this change adds the object's **first
  server-side renderer** and gives it exactly the nine keys
  [flows/merchant-auth.md](../../flows/merchant-auth.md) already documented.

## Decisions taken here

**D1 — the tenant is a join, not a new column.** `refunds` has no
`merchant_id`; migration `0017` gives it a `NOT NULL` FK onto
`payment_intents (id)`. `Refunds::get_for_merchant` joins and filters
`p.merchant_id`. A denormalised column was rejected: two answers to "whose
refund is this?" is how one of them goes stale, and the cost of the join is one
primary-key lookup. (A migration would also have collided with the numbering of
two other branches in flight the same day — a reason to notice the choice, not
a reason to make it.) Written up in
[reference/vpay-db.md](../../reference/vpay-db.md) § `refunds`.

**D2 — the repository exposes one read and no write.** A `create` would be a
write path no shipping code calls, i.e. a feature this repository would be
claiming it has. The integration suite therefore `INSERT`s its own rows against
the real schema, the way `support::age_the_crash` writes a column no shipping
code writes. **This is a deliberate deviation from the implementation brief**,
which said to seed through the repository; the brief's own rule that an unbuilt
feature stays visibly unbuilt (`AGENTS.md` rule 2) was taken to outrank it. It
is a reviewer decision to reverse.

**D3 — `status` is a `String` on the wire object, not a closed enum.**
`EventObject::kind`'s argument: the vocabulary is closed by Postgres where it is
*written* (`refund_status`), and a value that failed to parse on the read path
would turn a merchant's `GET` into a `500` instead of showing them the refund.
`every_stored_refund_status_decodes_in_the_merchant_sdk` pins that the four the
database can produce are the four both SDKs model, so widening the enum without
widening the SDKs fails.

**D4 — the `re_` prefix is checked into the *same* `404`, not into a `400`.**
The brief asked for id-prefix validation; `v1::events::retrieve`'s own doc
comment argues, in this repository's words, that a malformed path id is a `404`
and not a shape error. Both are satisfied: the check exists and short-circuits
before Postgres, and the answer is byte-identical to a missing id's. A `400`
would have been one more thing this route tells a caller than `/v1/events/{id}`
does, for no security benefit and at the cost of an inconsistency.

**D5 — `RefundRow` is a projection, not the whole table.** `charge_id`,
`failure_code`, `failure_raw`, `provider_reference_id` and `updated_at` are on
the table, on no wire object, and filled by no writer. `events::EventRow` makes
the same choice for `fanout_attempts`; `checkout_sessions`' one-to-one rule was
not followed, because guessing at the shape of code nobody has written is what
this repository calls claiming a feature.

## Mutations run, and what each one proved

Each was applied, measured, and reverted, on 2026-09-05.

| Mutation | Result |
|---|---|
| Drop `p.merchant_id = $1` from the join in `vpay_db::refunds` | `merchant_b_cannot_read_merchant_as_refund` **FAILS**: `left: 200, right: 404` |
| Delete the `/refunds/{id}` entry from `vpay_api::v1::V1_ROUTES` | 3 of 4 integration cases **FAIL** (the SDK read is `unknown_route`), and the unit test `the_refund_resource_is_mounted_for_a_read_and_for_nothing_else` **FAILS**. **All 136 `vpay-sdk` tests still pass** — the SDK's wiremock cases prove the client, the route test proves the server, and neither substitutes for the other |
| Render the response through a second hand-built map (`created` in ms) | `the_api_response_and_an_events_payload_for_one_refund_are_byte_identical` **FAILS** on the serialised bytes |
| Rename a test named in the `refunds.retrieve` parity row | `verify-sdk-parity` **FAILS**, naming the cell |
| **Delete the `refunds.retrieve` parity row entirely** | `verify-sdk-parity` **still passes** (346 → 342 proving tests). **The gate is one-directional**: it checks that every test a cell names exists, and cannot know that a capability the SDKs have is unrecorded. The brief predicted a failure here; it does not happen, and that is a real limitation of the gate rather than of this change |

## What is not proven

- **Nothing about how a refund comes to exist.** Every row the integration
  suite reads it wrote itself.
- **Nothing through stripe-node.** `sdks/stripe-compat` has no route table and
  no refunds case; `stripe.refunds.retrieve()` working is untested rather than
  known, and [flows/stripe-sdk-compat.md](../../flows/stripe-sdk-compat.md)
  says so in the same words it uses for `stripe.events.list()`.
- **Nothing about the Node SDK against a real vpay.** Its two new cases run
  against `src/testing/test-server.ts`, which is where every other Node
  resource case runs; the standing gap is recorded in
  [sdks/parity.md](../../sdks/parity.md).

## Documentation corrections this change forced

Re-measuring the surface turned up four claims that were already wrong before
this branch, all now struck through with a dated correction rather than
silently rewritten:

1. `flows/merchant-auth.md`'s Resources table listed `GET /v1/events` as
   `⛔ 404`; it has been served since Step 5 (2026-09-03). `GET
   /v1/events/{id}` was missing entirely, and the four Checkout Session routes
   are not in that table at all (they are in `flows/hosted-checkout.md`, which
   the table now points at).
2. `api/README.md`'s "two of its eight resource methods" — the SDKs expose
   **thirteen**, of which **two** have no route. Eight predated Checkout
   Sessions.
3. `status.md`'s "three of the SDKs' eight resource endpoints — refunds,
   events, balance — still have no route at all", and "`confirm` answers
   `501`", stale since Step 5 and Step 3 respectively.
4. `status.md`'s SDK test counts: 113 → 136 (Rust), 126 → 174 (Node),
   re-measured rather than incremented.

## Pre-existing flake, observed and not fixed

`sdks/rust/tests/token_exchange.rs`'s
`a_second_concurrent_401_does_not_discard_the_token_the_first_one_just_fetched`
failed once in five consecutive `cargo nextest run -p vpay-sdk` runs on a
loaded host on 2026-09-05, and passed the other four. Nothing in this change
touches that file. Recorded in `status.md`'s Rust SDK row.
