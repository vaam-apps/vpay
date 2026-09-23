# ADR-0027: Customer erasure also reaches payments through a checkout session

- **Status:** Accepted
- **Date:** 2026-09-23
- **Deciders:** the vpay maintainer, who decided on 2026-09-23 that "erasure
  must also reach payments through `checkout_sessions.customer_id`". The
  decision reached this change through the brief for it, and the guard in
  D2 was specified in the same brief. D3–D5 are how it was built. They were
  not re-decided.
- **Answers:** the question ADR-0025 leaves open. ADR-0025 is
  vaam-apps/vpay#253, which is open and not on `master`, so it is cited here
  by path only: `docs/adr/0025-session-customer-onto-intent.md`, § Context,
  effect 3, and § Consequences, "this ADR does not decide whether erasure
  should also go through `checkout_sessions.customer_id`". **ADR-0025 is not
  edited by this change.**
- **Number:** 0025 belongs to #253, and 0026 is reserved for another change
  in flight. `ls docs/adr` on `origin/master` `9184e42` returns `0001`…`0024`.
- **Implementation:** the same change as this ADR (branch
  `fix/erasure-through-checkout-sessions`).
  [The verification page](../status/verification/2026-09-23-erasure-through-checkout-sessions.md)
  and the `docs/status/` row say what is built and proven. This ADR records
  the decision, not what works.
- **Related:** [ADR-0020](0020-privacy-controls-and-evidence.md) (privacy
  controls), [ADR-0024](0024-customer-filters-and-manual-payments.md) (D12:
  sessions filter on their own customer column),
  [customers/privacy-and-erasure.md](../flows/customers/privacy-and-erasure.md).

## Context

Customer erasure (`DELETE /v1/customers/{id}` and the twelve-month sweep;
both run `vpay_db::customers::erase_in_tx`) rewrites the per-payment copies
of a payer in one transaction. There are three statements, and on
`9184e42` all three found a payment the same way, through
`payment_intents.customer_id = X`:

| Statement         | What it writes                                                                                                  |
| ----------------- | --------------------------------------------------------------------------------------------------------------- |
| `charges`         | `payer_ref` (the MSISDN the rail was given) → the marker, `payer_ref_masked` → NULL, `failure_raw` → the marker |
| `refunds`         | `failure_raw` of those charges' refunds → the marker                                                            |
| `payment_intents` | `last_payment_error_code` and `_message` → NULL, together (`lpe_paired`)                                        |

Nothing else in the erasure reaches a payment. The other copies it rewrites
are found through the customer's own id: `customer.*` event bodies, their
deliveries' digests and excerpts, stored `POST /v1/customers` responses, and
the out-of-band references on the customer's invoices. No intent, session or
refund body that vpay stores renders a payer identifier. They carry
`customer: "cus_…"` and nothing else of the payer's, so none of them needs a
statement on either path.

Before vpay#253, a checkout session created with `customer=X` on an intent
that had no customer stored `X` on the session row only. The charge
collected through that session hangs off an intent with no customer. So
erasing `X` left that charge's `payer_ref`, the rail's words about `X` on the
charge and on its refund, and the intent's decline text, all in place.
vpay#253 stops new rows taking this shape and does not backfill, for reasons
ADR-0025 gives. **Rows written before it keep the gap.**

## Decision

**D1. An erasure of `X` reaches an intent that names `X`, and also a
customer-less intent that a checkout session naming `X` points at.** The
reach is one constant, `vpay_db::customers::PAYERS_INTENTS`, and all three
per-payment statements interpolate it:

```sql
SELECT id FROM payment_intents WHERE customer_id = $1
UNION ALL
SELECT p.id FROM checkout_sessions s
JOIN payment_intents p ON p.id = s.payment_intent_id
WHERE s.customer_id = $1 AND p.customer_id IS NULL
```

**D2. The guard: a session leads the erasure only to an intent whose own
`customer_id` is NULL or `X`.** In the SQL, the guard is
`p.customer_id IS NULL` on the session branch, and the first branch covers
the case where the intent names `X`. An intent that names a different
customer `Y` is `Y`'s payment, and its charge carries `Y`'s MSISDN. A session
on it that names `X` does not make it `X`'s, and erasing `X` never redacts
it. The API has always refused a session that contradicts its intent's
customer, but a database reaches this shape anyway. Before vpay#253 an
expired session could name `X` on a customer-less intent. Since vpay#253, a
later session naming `Y` writes `Y` onto that intent.

**D3. Same transaction, same statements.** Nothing is added to the erasure
and nothing moves out of it. The three statements keep their shape, their
bind parameters and their place in `redact_stored_copies`, and only the
sub-select that finds the intents changes. The reason for one transaction is
the existing one: a sweep afterwards would be a window in which "vpay erased
this payer" is true of one table and false of another. `erase_in_tx` still
issues fifteen statements. The count was corrected on its doc comment in the
same change, because it said "six more" and the true figure is thirteen
more.

**D4. The lock order is unchanged: the customer first, then its rows.** The
erasure still takes `FOR UPDATE` on the customer before anything else. Every
lock it takes after that is on a row it is about to rewrite. The
`payment_intents` statement now also filters on
`last_payment_error_code IS NOT NULL`, so it locks only an intent that has
decline text to erase. That is an intent with a charge outcome. A session
can only be created on an intent that has no charge
(`vpay_api::v1::checkout_sessions`), so an intent a new session is being
created on is never locked by an erasure. This matters once vpay#253 lands.
Its session create `UPDATE`s the intent first, and then waits on the
customer for the session's foreign key, which is the opposite order. The
`charges` and `refunds` statements lock charges and refunds, and never an
intent.

**D5. Erasure writes no customer onto any intent and detaches nothing.**
After an erasure, the intent still names nobody and the session still names
`X`. What changes is that the payer is gone from the rows.

## Consequences

- **The three per-payment copies of a historical session-only payment are
  now erased** by both erasure paths. `an_erasure_reaches_a_payment_whose_only_link_to_the_payer_is_a_checkout_session`
  (both paths),
  `an_erasure_through_a_session_never_reaches_an_intent_that_names_another_customer`
  (D2), `an_erasure_takes_no_lock_on_a_session_reached_intent_it_has_nothing_to_erase_on`
  (D4) and the whole-database scan
  `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`, which
  now includes this shape, are the evidence
  (`backends/tests/integration/tests/customers.rs`).
- **An ambiguous historical intent is redacted by either payer's erasure.**
  ADR-0025's effect 2 is a customer-less intent with sessions naming both
  `X` and `Y`. Nothing records which of them paid. Under D1 and D2, erasing
  `X` redacts that intent's charge, and so does erasing `Y`. If `Y` paid,
  `X`'s erasure removes `Y`'s MSISDN from vpay's own charge row. The error
  goes toward erasure and never toward disclosure: nothing is sent anywhere,
  and a later erasure of `Y` finds the copy already gone. What is lost is
  vpay's record of which number paid, which is dispute evidence. The
  alternative is the third one below, and it was not chosen.
- **A refund of such a charge after the erasure hands the rail the marker**
  as `payer_ref`. The same is already true of a charge on an intent that
  names an erased customer. Nothing new is decided here.
- The list filters are unchanged. ADR-0024's D12 and ADR-0025's "no
  backfill" still stand. This ADR changes what an erasure reaches, not what
  a list returns or what an intent names.
- `vpay_db::sql_audit`'s `EXPECTED_ASSERT_SITES` goes 71 → 74. The three
  statements were plain literals, and now each interpolates the one
  constant, rather than spelling the reach three times.
- No migration. `schemas/privacy-inventory.yaml` does not change, because no
  column is added and no classification moves. The inventory page's
  description of the redaction statements does change.

## Alternatives considered

- **Leave it.** This is ADR-0025's status quo for historical rows. Rejected
  by the maintainer: it leaves a payer's MSISDN in `charges.payer_ref` after
  that payer's erasure, on exactly the payments a checkout session collected.
- **Backfill `payment_intents.customer_id` from the sessions, then erase
  through the intent as before.** Rejected for ADR-0025's reason: a backfill
  has to pick between sessions that named different customers, and a wrong
  pick attaches a payment to a payer who did not make it. It would also
  change what the intent and refund filters return, which is ADR-0025's
  question, not this one.
- **Reach through a session only when no session on the intent names
  anyone else.** Not chosen. It spares the ambiguous intent above, and so it
  leaves `X`'s MSISDN in place after `X`'s erasure whenever `X` did pay.
  The guard the maintainer specified is about the intent's own customer,
  and this ADR does not add a second one. If the maintainer wants
  ambiguous intents treated differently, the change is one more predicate
  on the session branch of `PAYERS_INTENTS`, plus a test.
- **Reach through any session, whatever the intent names.** Rejected: that
  is the removal of D2's guard. It would redact `Y`'s payment on `X`'s
  erasure, and the guard's test fails on exactly that mutation.
