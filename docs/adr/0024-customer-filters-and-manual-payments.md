# ADR-0024: Customer filters on list endpoints, and manual (out-of-band) invoice payments

- **Status:** Accepted in part.
  - **D1–D8 are accepted.** They are RFC-0004 § 5 (first bullet) and § 6 as
    that RFC was merged on 2026-09-23 (#244). The maintainer then directed,
    the same day, that step A be built ("Start on RFC-0004 step A: customer
    filters and manual payments") and that this ADR be written ("yes, write an
    ADR for step A").
  - **D9–D11 are proposed and need the maintainer's confirmation.** They were
    added by the implementing agent while briefing the work, and appear in no
    document the maintainer read before directing it. Each is marked where it
    stands.
- **Date:** 2026-09-23
- **Deciders:** the vpay maintainer (D1–D8, by directing RFC-0004 step A to be
  built); the implementing agent (D9–D11, pending the maintainer)
- **Implements:** [RFC-0004](../rfc/0004-billing-on-top-of-invoices.md) § 5
  (the `customer` filters only; not the invoice preview, and not
  `/v1/subscriptions`, which does not exist) and § 6. The rest of RFC-0004
  stays **Draft**.
- **Number checked at branch time, not assumed:** `ls docs/adr` on
  `origin/master` `d98fdaf` returns `0001`…`0023`, 23 files for 23 numbers.
  `gh pr list --state open` on 2026-09-23 returns no open pull request, so
  none touches `docs/adr/`.

## Context

RFC-0004 names two gaps that need nothing else in the RFC to close:

1. **Nothing but invoices can be listed by customer.** `GET /v1/invoices`
   takes `customer`. `GET /v1/payment_intents`, `/v1/checkout/sessions` and
   `/v1/refunds` do not. The first takes only `limit`, `starting_after` and
   `ending_before` (`vpay-api/src/v1/payment_intents.rs`, `ListParams`).
   Lago's per-customer collections (`/customers/{id}/payments`, …) are, in
   Stripe's shape, filters on the parent list.
2. **A merchant cannot record money that did not cross a rail.** An invoice
   becomes `paid` only inside the settlement transaction of a succeeded
   payment intent ([invoices.md](../flows/invoices.md) § When the intent
   succeeds). A merchant paid in cash, by cheque, or by a bank transfer
   received directly can only `void` or `mark_uncollectible` a bill that was
   in fact settled. Both are false statements about the document.

Both are step A of RFC-0004's delivery table because they depend on nothing:
no catalog, no schedule, no new rail.

## Decision

### Customer filters

**D1. `customer` filters `GET /v1/payment_intents`, `GET
/v1/checkout/sessions` and `GET /v1/refunds`.** The value is a `cus_…`.

**D2. The filter is applied in the same `WHERE` as `merchant_id`.** Another
merchant's customer, or one that never existed, yields an **empty list**,
never a `404` or any answer that differs from the one an unused id of the
caller's own gets. A list filter that answered differently for a foreign id
would be an existence oracle over another tenant's payers. That is the rule
`checkout_sessions`' `payment_intent` filter already follows, and the reason
[merchant-auth.md](../flows/merchant-auth.md) makes every `/v1` 404
byte-identical.

**D3. A malformed value is refused exactly as the existing filters refuse
one.** One rule, not a second spelling of it.

**D4. `GET /v1/customers` stays unfiltered.** This reaffirms the reason its
`ListParams` doc comment gives: a filter on a payer identifier (email, phone)
turns a list into a lookup. D1 filters **by** a customer id the merchant
already holds. It does not search **for** a customer, and nothing here
changes that.

### Manual payments

**D5. Stripe's spelling, on the existing route.** `POST
/v1/invoices/{id}/pay` with `paid_out_of_band=true`, plus vpay-native
`out_of_band[method]` (`cash` | `cheque` | `bank_transfer` | `other`),
`out_of_band[reference]` and `out_of_band[received_at]`. No new route. A
merchant whose Stripe-shaped client already sends `paid_out_of_band` reaches
the same state.

**D6. It is a compare-and-swap from `open` to `paid`, refused while a live
intent is attached.** It uses the same `NO_LIVE_INTENT` condition as `pay`,
`void` and `mark_uncollectible`, and the same `409` naming the intent. So a
payer cannot pay through the hosted link after the merchant recorded cash, and
cancelling the intent is the way back, exactly as for the other transitions.
Draft, void, uncollectible and already-paid invoices are refused in the shape
those transitions already use.

**D7. One `manual_payments` row per invoice (`mp_…`), and payment stays
all-or-nothing.**

- `paid_means_nothing_remaining` holds unchanged.
- The invoice gains `paid_out_of_band`, backed by a CHECK that it implies
  `status = 'paid'`.
- The row and the transition are one transaction. A recorded payment with an
  unpaid invoice, or the reverse, is not a state a crash can leave.

**D8. `invoice.paid` gains a second writer; the ledger gains nothing.**

- The event is written inside the transition's own transaction. It is already
  in `type_is_a_documented_event`, so the database's vocabulary does not move.
- **Nothing is posted to the ledger.** No money crossed `payer_clearing`, and
  under pass-through ([RFC-0001](../rfc/0001-settlement-and-payouts.md)) vpay
  has no account the money arrived in.
- The row is **a record of a merchant's statement**, and every surface
  describing it says vpay cannot verify it.
- Only the merchant API writes it. An operator recording a payment on a
  merchant's behalf needs [ADR-0008](0008-dashboard-scope.md)'s unbuilt
  dashboard writes and their audit log.

### Proposed — added while briefing the implementation, pending the maintainer

**D9. The invoice object gains two keys, not one:** `paid_out_of_band`
(bool, Stripe's) and `out_of_band_payment` (`null`, or `{id, method,
reference, received_at}`, vpay-native). RFC-0004 § 6 named only the first.
Without the second, the method and reference a merchant sent could be stored
and never read back through `/v1`, which is data with no reader. Nineteen keys
become twenty-one, and the key-count pin and both SDKs' decoders move with
them.

**D10. The out-of-band path neither needs nor accepts redirect URLs.**

- `success_url` or `cancel_url` sent with `paid_out_of_band=true` is a `400`
  naming the parameter, because they have no meaning here and silently
  ignoring them would hide a client bug.
- The path does **not** require `merchant_clients[].invoices` to be
  configured.
- `out_of_band[…]` without `paid_out_of_band=true` is a `400`.

**D11. `out_of_band[method]` is required; `received_at` is bounded.** It
defaults to now, and is refused in the future or before the invoice's
`finalized_at`. An undated or future-dated record of cash received is a
bookkeeping error the API can catch at no cost.

## Alternatives considered

- **A separate route** (`POST /v1/invoices/{id}/mark_paid`). It is clearer to
  read, but it is not Stripe's spelling, and a merchant's existing client
  sends `paid_out_of_band` to `pay`. Rejected for the same reason
  `invoice_items` kept Stripe's route name.
- **Posting out-of-band payments to the ledger** under a memo account. It
  would make `merchant_payable` include money vpay never saw. Under
  pass-through that balance already means "what crossed a rail for this
  merchant", and mixing merchant statements into it would make it mean
  neither.
- **Partial manual payments.** `paid_means_nothing_remaining` is where
  partial payment stops being a sentence; RFC-0004 keeps it, and so does
  this. A merchant who took a deposit issues two invoices, as today.
- **Filtering `GET /v1/customers` by phone or email.** Refused, per D4.

## Consequences

- `open → paid` has **two writers**: the settlement transaction and the
  out-of-band compare-and-swap. `invoices.md`'s state machine, Events table
  and Status move with the code, in the same change.
- **A migration** adds `invoices.paid_out_of_band` and `manual_payments`,
  both modelled in `schemas/vpay.cstack` and classified in
  `schemas/privacy-inventory.yaml`.
  - `manual_payments.reference` is free text a merchant may put a payer's
    details into, so it is personal data.
  - What customer erasure does to it is decided by the implementation and
    recorded in `docs/flows/customers.md`'s erasure page and the privacy
    inventory, not here.
- **Both SDKs** gain the list `customer` parameter and the `pay`
  parameters, with parity rows in the same change (ADR-0015).
- `docs/flows/customers.md`'s "no lookup by customer" gap, and
  `docs/flows/invoices.md`'s "cash can only be voided" consequence, each close
  with a dated correction saying what the page said before.
- RFC-0004's status line records §§ 5–6 as accepted through this ADR. The
  rest of the RFC stays Draft, and nothing here decides any of its open
  questions.

## Left to the maintainer

1. **Confirm or reverse D9–D11.** Each is reversible before the
   implementation merges, and costs a schema or wire change after.
2. **Should the out-of-band path accept `received_at` before `finalized_at`**
   at all? A merchant may have been paid before issuing the document. D11
   refuses it, as the stricter default.
