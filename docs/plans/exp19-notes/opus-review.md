# S4a — the Customer object, sabotage review

Reviewer's transcript for `claude/exp19-customers-opus`, base `aa912bb`,
implementer's head `525b0be`. Everything below was measured on this branch on
2026-09-06/07 against `postgres:16-alpine` under the pinned 1.98.0 toolchain,
Node 22.23.2 from `.nvmrc`, and `cratestack 0.11.1`.

The implementer's own notes are `opus.md`. This file records only what the
review measured, and disagrees with `opus.md` in three places.

---

## Verdict

**Not safe as delivered — but every finding was a missing gate, not a wrong
behaviour.** Five mutations that a payments repository must not survive were
survived by the delivered suite. No incorrect behaviour was found anywhere in
S4a: every test added by this review passed on the delivered code the first
time it ran. The implementation is right; four things it claimed were proved
were not proved, and one claim was simply false.

`just ci` was green as delivered (1437/1437, 44 binaries, 0 ignored, exit 0),
and is green now (see the tail of this file). That is the point: the gates did
not move, and the holes were underneath them.

---

## Findings

| # | Severity | Finding |
|---|---|---|
| 1 | **gate-hole** + misleading-claim | The object's key set was guarded by nothing, and the tripwire that two documents named did not exist |
| 2 | **gate-hole** | `GET /v1/customers`' cursor could be de-scoped across tenants and nothing objected |
| 3 | **gate-hole** | `POST /v1/customers`' idempotency was exercised by no case |
| 4 | **gate-hole** | Four documented rules for a **checkout session's** `customer` had no test at all |
| 5 | misleading-claim | `touch_last_used`'s published contract names a statement that is never rendered |
| 6 | misleading-claim | "Phone is the customer identity" never said that it does not deduplicate |
| 7 | rule-break (ADR-0015) | Neither SDK models a checkout session's `customer`, in either direction, with no gap row |

### 1. The customer object's key set — nothing guarded it

`CustomerObject`'s own rustdoc and `docs/flows/customers.md` both named
`the_customer_object_is_the_documented_seven_keys` as "the tripwire" keeping
the retention clock off the wire. **No test of that name existed.**

Adding `last_used_at: i64` to `CustomerObject` and rendering it:

```text
vpay-api                     422 tests run: 422 passed
integration::customers        14 tests run:  14 passed
@vaam-apps/vpay-sdk (node)   190 passed (190)
```

Nothing objected to publishing the sweep's clock. This object is
`customer.deleted`'s `data.object`, so the leak would have been signed,
delivered at-least-once and stored in `events` **forever** — the one place
vpay cannot retract a field.

**And the count in all three places was wrong.** The object is `id`, `object`,
`name`, `email`, `phone`, `metadata`, `created`, `livemode` — *eight* keys,
which is exactly what `customers.md`'s own table lists row for row while its
prose said seven. The test is named for the measured count and the documents
were corrected to it, not the reverse.

Closed by `the_customer_object_is_the_documented_eight_keys` and
`a_phone_only_customer_renders_the_absent_identifiers_as_null` (`3ea722c`).

### 2. The list cursor's tenancy

`list_page`'s cursor is a correlated subquery and has to be scoped separately
from the list's own `WHERE merchant_id = $1`. Dropping `AND merchant_id = $1`
from the forward subquery: **13 tests run: 13 passed**. With the new case it
pages three of merchant A's rows anchored in merchant B's sequence.

The delivered code is correct — both subqueries were already scoped. Nothing
measured it.

### 3. `POST /v1/customers`' idempotency

No delivered case replayed a create. Replacing `post.finish(..)` with
`post.release(..); outcome` — the copy-paste of the error path — passed all
thirteen others. A retried create then mints a second `cus_…` for one payer,
which no unique index can catch because there deliberately is none.

### 4. The checkout session's `customer`

`docs/flows/customers.md` documents four rules (accepted, stored, rendered,
inherited from the intent, refused when the two disagree).
`checkout_sessions.rs` gained a `customer_id: None` field filler and a
key-count bump on the *nested intent*. Nothing else.

Deleting the contradiction check and running both suites:

```text
46 tests run: 45 passed, 1 failed
```

The entire thirty-one-case `checkout_sessions.rs` suite green. The session is
the only object in vpay where two rows can name two different payers, and that
branch was the untested one.

The same case also closes the `checkout_sessions` half of the sweep's
`NOT EXISTS` guard — `the_sweep_deletes_an_idle_unreferenced_customer_and_keeps_the_other_two`
only ever exercised `payment_intents`.

### 5. `GREATEST` is never rendered

`Customers::touch_last_used`'s trait rustdoc said, flatly: "The statement is
`SET last_used_at = GREATEST(last_used_at, $2)`." Migration `0034` said the
same. Neither is true — the method goes through CrateStack,
`UpdateCustomerInput` renders a plain assignment, and the monotonicity comes
from `where_(last_used_at.lt(now))`.

The implementation's inline comment and `docs/reference/vpay-db.md` had it
right, so the repository contradicted itself in four places and the two that
were wrong are the two a reader reaches first. `opus.md` § 1 describes the
filter correctly and did not notice that the shipped doc and the migration
still claimed otherwise.

Corrected in `d44b6df`, which also records the one observable difference the
old text hid: the filter form returns `Ok(false)` for a backwards stamp where
a `GREATEST` assignment returns `Ok(true)`.

**Editing an applied migration is a checksum mismatch on the next boot.** 0034
is unmerged and no deployment has run it, so this was the last moment it could
be corrected.

### 6. "Phone is the customer identity" is not "phone deduplicates"

The document stated decision 1, and separately that there is no unique index
on `phone`, and left the reader to notice that two `POST /v1/customers` with
one number make two customers. The natural reading of "identity" is the
opposite, and it is the reading a merchant builds on. A new section says it,
says why (the index that makes a phone unique is the index that makes
cross-merchant identity possible), and bounds what canonicalisation buys.

### 7. Neither SDK can reach a session's `customer` — ADR-0015

`CreateCheckoutSessionParams` has no `customer` field in Rust or TypeScript,
and neither `CheckoutSession` type carries the key the server now returns. A
merchant driving sessions through an SDK cannot attach a customer at all.

`verify-sdk-parity` did not notice because it is two-directional between the
**SDKs**, and the SDKs agree with each other — they are short of the *server*.
Recorded as dated ⛔/⛔ rows owned by the SDK maintainers rather than
implemented here (ADR-0015 allows either), because adding a resource field to
two SDKs is a change to Checkout Sessions and not to the review of Customers.
**This is the one thing in S4a a merchant can reach for and not find.**

---

## What the review confirmed rather than found

Verified by mutation, and each held:

| Mutation | Result |
|---|---|
| `from_wire` back on its own private list, missing `SweepIdleCustomers` (the *original* bug, re-armed exactly) | `the_wire_spelling_is_the_same_by_both_routes` **FAILS in 12 ms**; `the_sweep_deletes_an_idle_unreferenced_customer_and_keeps_the_other_two` **FAILS naming the dead letter and `alert: true`**. Two gates, as claimed |
| Rename the Rust test the `customers.del` parity row names | `verify-sdk-parity` fails naming row, SDK and test |
| Rename Node's `customers.del` | `verify-sdk-parity` fails naming the unrowed method |
| Drop the tenant filter from `list_page`'s cursor | caught only by the new case (finding 2) |
| `create` releases the key instead of storing the response | caught only by the new case (finding 3) |
| Remove the session's contradiction check | caught only by the new case (finding 4) |

Also confirmed by reading and by the green gate: `delete_idle` re-checks its
guard **inside the `DELETE`** rather than after a read, so the sweep cannot
race a use; the sweep is bounded at 100 per pass with a progress-conditional
reschedule; the second sweep pass emits no second event; a second `DELETE` is
the uniform 404; a foreign customer on an intent or a session is the identical
`400`; `expand` does not exist, so there is no expanded-object leak to test;
the drift constants are re-derived by
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` against a
real container rather than asserted against themselves; and
`every_action_this_module_calls_has_an_allow_arm` asserts the *absence* of the
three arms the model does not grant, not only the presence of two.

`park_the_housekeeping_jobs`' second `ensure` (525b0be) is a real fix and
keeps the promise its doc makes.

---

## Maintainer decisions surfaced

1. **The session `customer` SDK gap (finding 7).** Implement in both SDKs, or
   leave the dated gap. It is the only merchant-visible hole in S4a.
2. `opus.md` § 7's four decisions stand and this review adds nothing to them
   (`address`; `customer.created`/`.updated`; the horizon as a constant; that
   deletion is not a complete erasure).
3. **The checkout session object has no key-count tripwire at all** — it
   gained `customer` in S4a and nothing would have noticed a ninth key either.
   That absence predates S4a and was left alone; `PaymentIntentObject` and
   `RefundObject` have one and `CheckoutSessionObject` does not.
4. `CustomerObject` derives `Debug` with the payer's name, email and phone in
   clear. Nothing formats one today (`CustomerRow`, which *is* logged, hand-
   redacts all three), so this is latent rather than a leak — but it is one
   `tracing::warn!(?object, …)` away from being one.

## Not checked

* Nothing about `cratestack` 0.11.1 beyond what the change calls.
* No rolling-deploy test of `0034`; `opus.md` § 8's reasoning was not
  re-derived.
* Cypress, the e2e compose stack, the shop demo, and any live rail.
* Creating a customer with **only an email** is not exercised by any case; it
  is the same code path as the name-only creates the cursor case makes, so it
  was reasoned about rather than measured.
* The `aa912bb` side of the drift delta (101/16/17) is `opus.md`'s
  measurement, not re-derived here; the *current* side is gate-enforced.
