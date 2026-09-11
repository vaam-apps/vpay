# `vpay-api` — the account-holder route and Customers

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## The account-holder route (`v1/account_holders.rs`)

`GET /v1/account_holders`, mounted 2026-09-05 for
[issue #47](https://github.com/vaam-apps/vpay/issues/47).
[../flows/account-holder-lookup.md](../../flows/account-holder-lookup.md) is the
process and the policy; this section is why the _code_ is shaped the way it
is.

### The handler matches the port's result rather than `?`-ing it

Every other `/v1` handler that reaches a rail writes
`adapter.submit(..).await?` and lets `ApiError`'s `#[from]` and `Classify`
do the rest. This one matches, and the reason is the counter: `error` and
`not_found` are two _different_ outcomes on
`vpay_account_holder_lookups_total`, and a `?` would leave the failure arm
invisible — "a merchant is asking a rail that is not answering" is exactly the
rate an operator wants, and it is the one thing a route with no persistence
leaves no other trace of.

The classification is still not re-decided: the `Err` arm counts, logs a
masked line, and hands the error to `ApiError` unchanged, which is what
derives the status (ADR-0011).

### `MerchantScope` is bound and unused

There is nothing to scope: no query runs, no row is read, and the answer is a
property of the rail. The extractor is bound anyway, because it is what makes
the _authentication_ boundary structural rather than remembered —
`MerchantScope::from_request_parts` fails closed with a paging 500 when the
middleware is not mounted, so a refactor that dropped the layer would fail
here instead of serving an unauthenticated identity lookup on a route that
returns a stranger's name. It is also the value an audit log would be keyed on
the day that reserved decision is taken.

### Three refusals, one envelope

An unknown rail, a rail an operator has **disabled**, and a rail whose
`supports_account_holder_lookup` is false all produce the byte-identical `400`
naming `payment_method_type` (`unsupported_rail`, one function, asserted by
`a_disabled_or_unknown_rail_is_the_same_refusal_as_an_incapable_one`).
Telling them apart would let a merchant enumerate which rails a deployment has
configured but switched off, and the fix is the same for all three.

It is a `400` and not the `409` `ProviderError::Unsupported` classifies to,
because the rail is never _called_: ADR-0002 asks the core to branch on the
capability first, and at that point the wrong thing is the merchant's
parameter, which is what `param` should name.

### The MSISDN validator is the first server-side copy of that rule

`frontends/apps/checkout/src/lib/msisdn.ts` had been the only implementation
of "Cameroon E.164, three input spellings, this separator set". The two are
deliberately **not** shared: the browser's is a form affordance that also
formats for display, and this one is a trust boundary — the page can be
bypassed entirely by a merchant calling `/v1` directly, which is the ordinary
way this route is used. Sharing would mean the server trusting a client-side
rule. What is shared is the specification, and both files say so.

Validating at all, rather than letting MTN refuse a malformed number, buys
three things: a rail call on our own credentials is not spent on an input we
could see was not a phone number; the hex WireMock steering numbers
(`237600000f01`) are unreachable through a production-shaped route; and an
arbitrary path segment cannot be interpolated into MTN's API. The adapter
percent-encodes it as well (`vpay_provider::http::path_segment`), belt and
braces, because the port is reachable by any caller in the process.

### The mask, and the column it does not write

`masked` produces `+2376••••200` — the shape
`charges.payer_ref_masked` is documented to hold, with a **fixed** four
bullets rather than one per hidden digit, because a mask whose length revealed
the input's length would be a small oracle for free.

**Nothing writes that column.** The confirm path stores `NULL`
(`open_attempt`), so this is the first producer of the shape in the workspace
and the two are not wired together. That is a gap rather than an oversight:
writing the column is a change to the charge path, not to this route, and
`docs/status.md` says so rather than this function pretending otherwise.

## Customers (`v1/customers.rs`)

Five routes, and three decisions worth reading before the code.
[`../flows/customers.md`](../../flows/customers.md) is the product document —
what a customer _is_, and the privacy rules around it. This is why the module
is shaped the way it is.

### The one-of rule is decided above the statement, and has to be

Migration `0034`'s `at_least_one_identifier` is a multi-column `CHECK`, and
`vpay_db::classify_write` routes a `23514` to `Category::Storage` — a `503`
telling a merchant to wait for a database that is perfectly healthy. So the
API decides it and the constraint is the backstop, exactly as
`checkout_sessions`' URL bounds front `0028`'s CHECKs.

On **create** that is a fact about the request. On **update** it is not: the
request may say only `phone=`, and whether that leaves the customer nameless
depends on the _stored_ row. `validate_update` therefore takes the row, and
computes what each field will be _after_ the patch — the patch's value where
it says something, the stored value where it does not. The case that a naive
"count the stored identifiers" check gets wrong is a patch that clears one and
adds another in the same request, and
`a_patch_may_clear_one_identifier_while_adding_another` is the test for it.

### `metadata` is merged, so the update reads first — and what that costs

Stripe's `metadata` is merged key-wise and a key sent empty is removed, so the
merge needs the stored map. That makes the update a **read-modify-write**, and
the window is real: two concurrent updates that each add one key can lose one.

It is not closed, and the reason is that closing it in Rust is not possible —
the merge is defined over the stored value, so the only fix is a `jsonb ||` in
the statement, which would move Stripe's merge semantics into a migration and
out of the layer that documents them. Stripe's own API has the same shape.

What is **not** at risk is the three scalar fields: `vpay_db::customers`'
`UPDATE` assigns each column through a `CASE WHEN $n::BOOLEAN` gated on "did
the request mention this field", so a concurrent update that touched a
different field cannot be clobbered by this one.

### The three-state patch, and why `Option<Option<String>>` reaches this far

An absent key leaves a field alone; a value sets it; **an empty value clears
it**. All three are Stripe's contract and a merchant depends on all three —
without the last, an email a payer asked to have removed cannot be removed
without deleting the whole customer, which the foreign keys may forbid
anyway.

The form decoder already distinguishes "absent" from "present and empty", so
the distinction is available here; `patch_field` maps it into
`vpay_db::CustomerPatch`'s double options, and both SDKs carry the same three
states in their own type systems. Every layer that could collapse two of the
three is one word away from doing so, which is why each proves it by
asserting the **body** rather than the type.

### `resolve_for_attachment` lives here, and stamps

Three call sites resolve a merchant-supplied `customer=cus_…` — an intent's
create, a session's create, and (off the intent's own column) the confirm
path — and every one makes the same three decisions: refuse a malformed id
with a `400` naming `customer`, refuse a customer that is not this merchant's
with the _same_ `400` and the same sentence, and record the use.

The stamp is the half that is easy to leave out and impossible to notice
missing. A customer a merchant uses on every order but which nothing stamps is
deleted by the twelve-month sweep, with a `customer.deleted` event that is
simply wrong. A **failure** to stamp is logged and swallowed, deliberately:
failing a merchant's `POST /v1/payment_intents` because a retention clock
could not be moved trades a real payment for a bookkeeping row, and the
exposure that buys is bounded and slow — the customer keeps its previous
`last_used_at` and the next use stamps again.

### `DELETE` is the only one on this API

It carries an `Idempotency-Key` like every other write, and that verb is where
a replay is most confusing without one: the second call would otherwise answer
`404` for a deletion that succeeded, which a merchant retrying a timed-out
request cannot tell from "somebody else deleted it".

**There is no `409` on this route since 2026-09-10.** A customer with payment
history is anonymised rather than refused (migration `0041`, issues #68 and
#96 item 2), so `DELETE` succeeds or answers the uniform `404`.

What the `409` used to express is still enforced, and still by the **foreign
key** rather than by a preceding `SELECT`: a payment is never detached from
its payer. `vpay_db::customers::erase_in_tx` reads `NOT (UNREFERENCED)` under
the row lock to choose between hard-deleting and anonymising, and that lock
does not stop a concurrent `POST /v1/payment_intents` inserting a reference —
an insert takes only a share lock on the customer. So the branch can be wrong
by one race, in exactly one direction, and the database catches it: the hard
delete raises `23503`, the whole transaction rolls back, and the retry takes
the other branch. The opposite race cannot happen, because history is never
removed. A count-then-delete would have had no such backstop.
