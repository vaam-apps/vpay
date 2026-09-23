# Invoices

A merchant's bill to one customer: what it is made of, how it moves, what
refuses each move, and what a merchant is told when it does.

This is S4b of the data-layer work, built on S4a's
[Customer](customers.md). (The plan document itself is not in this repository
— `docs/plans/2026-09-06-data-layer.md` is cited by the first line of migration
`0034`'s header **and by the first line of `0036`'s**, and no such file is
tracked or on disk. This paragraph named only `0034`'s until the S4b review on
2026-09-07 and said the citation "is not repeated here", which was true of this
document and not of the migration the same commit wrote. Neither `.sql` can be
corrected in place — `sqlx::migrate!` checksums a migration's whole bytes and
`just verify-migrations` pins them, which is why `0006`, `0013` and `0017`
carry their corrections here too rather than in the file. `verify-links` does
not read `.sql`, so nothing but a reader was ever going to catch it.) It is Stripe's `invoice`, narrowed to the
subset a Cameroon merchant needs to bill a phone — there is no PDF, no e-mail,
no tax, no credit note, no dunning and no subscription. Every one of those is
listed under [What is not built](#what-is-not-built) with a date, because a
document that lists only what exists is how somebody comes to believe an
invoice gets sent.

---

## The two objects

### Invoice — `in_…`

| Field                                             | Meaning                                                                                    |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `id`                                              | `in_…`, minted before the row exists                                                       |
| `customer`                                        | the `cus_…` this bills. **Required**                                                       |
| `currency`                                        | lower-case ISO-4217; every line is in it                                                   |
| `status`                                          | `draft` → `open` → `paid` \| `void` \| `uncollectible`                                     |
| `number`                                          | `{prefix}-{000001}`, assigned at finalize, `null` while a draft                            |
| `amount_due` / `amount_paid` / `amount_remaining` | integer minor units ([money.md](money.md))                                                 |
| `amount_refunded`                                 | how much of `amount_paid` has been given back — **gross**, see [Refunds](#refunds)         |
| `paid_out_of_band`                                | Stripe's key: `true` when the merchant recorded payment outside vpay                       |
| `out_of_band_payment`                             | `null`, or `{id: "mp_…", method, reference, received_at}` — see [below](#paid-out-of-band) |
| `due_date`                                        | unix seconds, **advisory** — nothing in vpay reads it                                      |
| `description`, `metadata`                         | the merchant's own                                                                         |
| `payment_intent`                                  | the `pi_…` paying it, or `null`                                                            |
| `hosted_invoice_url`                              | the checkout session for that intent, or `null`                                            |
| `lines`                                           | every line, expanded, as a `list`                                                          |
| `status_transitions`                              | `finalized_at`, `paid_at`, `voided_at`, `marked_uncollectible_at`                          |
| `created`, `livemode`                             | as everywhere else                                                                         |

**`customer` is required, and a payment intent's is not.** An invoice is a
bill _to somebody_: it carries a number a merchant quotes in a conversation,
it may be chased for months, and on this market the payer's identity is a
phone number (the maintainer's decision of 2026-09-05, [customers.md](customers.md)).
An invoice with no customer is a bill nobody could be asked to pay, and
`POST /v1/invoices/{id}/pay` would have no payer to bind an intent to.

### Line — `ii_…`, rendered as `line_item`

`id`, `description`, `quantity`, `unit_amount`, `amount`, `currency`,
`livemode`. Nothing else: no price, no product, no proration, no period.

**The route says `invoice_items` and the object says `line_item`, and that is
deliberate.** Stripe has two objects where vpay has one. An `invoiceitem`
there is a _pending_ charge not yet attached to any document; a `line_item` is
what appears on `invoice.lines` once it is. vpay's `POST /v1/invoice_items`
writes straight onto a named draft — there is no pending-charge inbox, because
that is a subscription feature and subscriptions are not built. So the route
keeps Stripe's spelling (a merchant's existing client calls it) and the object
keeps Stripe's `lines` spelling (a Stripe-shaped handler switches on it).

`amount` is never a parameter. It is `quantity * unit_amount`, computed by the
statement and checked by the database (`amount_is_the_product`), so a
caller-supplied value that disagreed with its own factors is not a state this
API can be asked to store.

---

## The state machine

```text
              POST /v1/invoices
                     │
                     ▼
                  ┌───────┐   DELETE /v1/invoices/{id}
                  │ draft │ ──────────────────────────▶ (gone, lines and all)
                  └───┬───┘
                      │ POST /v1/invoices/{id}/finalize
                      │   · takes the next number, under a row lock
                      │   · freezes the lines
                      │   · sums amount_due, once
                      ▼
                  ┌──────┐
       ┌──────────│ open │──────────┐
       │          └───┬──┘          │
       │              │             │
       │ /void        │ /pay        │ /mark_uncollectible
       │              │  then the   │
       │              │  settlement │
       │              │   ─ or ─    │
       │              │  /pay with  │
       │              │  paid_out_  │
       ▼              │  of_band    ▼
   ┌──────┐           ▼        ┌───────────────┐
   │ void │      ┌──────┐      │ uncollectible │
   └──────┘      │ paid │      └───────────────┘
                 └──────┘
```

Every one of those arrows is terminal on the right-hand side. Nothing in vpay
moves an invoice out of `paid`, `void` or `uncollectible`, and no route tries.

**`open → paid` has two writers since 2026-09-23** (RFC-0004 § 6, migration
`0049`): the settlement transaction of a succeeded intent, and
`POST /v1/invoices/{id}/pay` with `paid_out_of_band=true`, which records the
merchant's statement that they were paid outside vpay. Both are
compare-and-swaps on `status = 'open'`; the second also carries
`NO_LIVE_INTENT`, so a settlement and an out-of-band payment cannot both win.
See [Paid out of band](#paid-out-of-band).

### Three enforcers, and none of them is a validation function

1. **`invoices_status_enum_check`** closes the vocabulary at five labels.
2. **Every transition is a compare-and-swap.** `UPDATE invoices SET … WHERE id
= $1 AND merchant_id = $2 AND status = '<from>'`. "Matched no row" _is_ the
   refusal. There is no `can_transition_to` anywhere in this codebase, and
   [`vpay_core::InvoiceStatus`](../reference/vpay-core.md) says why: a Rust
   guard beside the write is the thing a future writer calls _instead of_
   taking the lock.
3. **Five multi-column CHECKs** make the combinations a broken transition
   would produce unstorable — `number_is_assigned_at_finalize`,
   `paid_means_nothing_remaining`, `only_a_live_invoice_has_an_intent`,
   `amounts_add_up`, `amount_is_the_product`. They are the guard that survives
   a future writer who forgets rule 2. Migration `0042` added a sixth
   (`refunded_at_most_paid`), and migration `0049` three more on `invoices`
   (`paid_out_of_band_means_paid`, `paid_names_how`,
   `paid_out_of_band_is_never_refunded`) and one on `manual_payments`
   (`received_before_recorded`) — see [Paid out of band](#paid-out-of-band).

Rule 3's constraints are **invisible to `cratestack migrate baseline` in both
directions**, exactly as S4a's `at_least_one_identifier` is, so the drift
report cannot be the guard for them.
`backends/tests/integration/tests/invoices.rs`'s
`the_invoice_invariants_are_enforced_by_the_database_itself` writes the row
each one exists to refuse, straight past the API and the repository.

### A draft is deleted; an issued invoice is voided

Stripe's own shape, and here it is _forced_ rather than chosen:
`number_is_assigned_at_finalize` says a non-draft invoice has a number, so a
voided draft would be a non-draft row with no number, which the database
refuses. Rather than discover that as a `23514`, `void`'s statement never
attempts it — the `WHERE` names `'open'` alone.

A voided invoice **keeps its number**, for the same constraint and for the
reason migration `0036` gives: a number that vanished is a hole an accountant
reads as a destroyed document.

---

## Numbers

`{prefix}-{000001}`. The prefix is eight upper-case characters of Crockford's
alphabet, minted once per merchant by their first finalize and stored in
`invoice_number_sequences`; the sequence is 1-based and per-merchant.

**Upper case and Crockford's alphabet**, because this is the one identifier in
this system a human types back in — off a paper receipt, into a bank transfer
reference, over the phone. `i`, `l`, `o` and `u` are absent, so there is no
`1`/`l` and no `0`/`O` to get wrong.

**Random rather than derived from the merchant**, because an invoice number is
quoted in e-mail and read out loud, and a prefix derived from `merchant_id`
would put the deployment's internal tenant identifier on every one of them.
Two merchants sharing a prefix is a cosmetic coincidence and not a conflict —
`invoices_merchant_number_key` is scoped to one merchant.

### A rolled-back finalize burns no number

`invoice_number_sequences` is an **ordinary table row and not a Postgres
`SEQUENCE`**. `nextval` is non-transactional by design, so a finalize that
failed after taking a number would leave a hole. Stripe's numbering has holes
and that is defensible for Stripe; it is not defensible here, because a
Cameroonian merchant's invoice numbers are read by a tax authority that treats
a missing number as a destroyed document.

Two concurrent finalizes serialise on that row: the second blocks on the
`INSERT … ON CONFLICT (merchant_id) DO UPDATE`'s lock and, under `READ
COMMITTED`, re-reads the committed value when it is released.
`two_concurrent_finalizes_take_consecutive_numbers` proves both halves — two
consecutive numbers, no gap, no reuse — and
`a_refused_finalize_does_not_burn_a_number` proves the rollback.

---

## Paying

`POST /v1/invoices/{id}/pay` mints an ordinary `pi_…` for `amount_remaining`,
creates an ordinary **hosted checkout session** for it, attaches the intent to
the invoice with a compare-and-swap, and answers the invoice with a
`hosted_invoice_url`.

**No new rail code and no invoice page.** A payer following that URL sees the
existing checkout page — the merchant's name and the amount — and pays through
the rails that already exist ([hosted-checkout.md](hosted-checkout.md)).
Building an invoice page would be a second payer surface to keep correct, and
the one that exists is the one the browser tests already drive.

**The payer is not shown the invoice number, and that is a gap rather than a
decision** (recorded by the S4b review, 2026-09-07). `pay` writes
`Invoice {number}` into the intent's `description`, so the number _reaches_
the payer's browser — `GET /v1/browser/checkout/sessions/{id}` expands the
intent and `description` is one of its keys — and
`frontends/apps/checkout` renders the amount and the merchant name and
nothing else. So a payer following a `hosted_invoice_url` from an e-mail sees
a bill they cannot tie to the document that asked for it. Closing it is a
change to the checkout screens, not to this resource; nothing here forecloses
it, and the number is already on the wire the page reads.

### It takes `success_url` and `cancel_url`, and Stripe's `pay` does not

Stripe's `pay` charges a payment method the merchant already has on file, so
there is nowhere to send anybody. vpay has no stored payment methods on this
market — a mobile-money payment is a payer approving a prompt on their own
handset — so paying means creating a hosted session, and migration `0028`'s
`urls_match_ui_mode` requires both URLs on one.

**They may be configured per merchant instead of sent per request** (issue
#91, D2, 2026-09-10). A registration may carry

```yaml
merchant_clients:
  - client_id: acme-cameroon
    merchant_id: acme-cameroon-tenant
    invoices:
      success_url: https://shop.acme.example/invoice-paid
      cancel_url: https://shop.acme.example/invoice-cancelled
```

and then `POST /v1/invoices/{id}/pay` may omit both. The resolution order is
**request, then configuration, then refuse**:

| Sent on the request | Configured | Result                                              |
| ------------------- | ---------- | --------------------------------------------------- |
| both                | either     | the request's, always                               |
| one                 | the other  | the request's for one, the configured for the other |
| neither             | both       | the configured pair                                 |
| neither             | neither    | one `400` naming **both** parameters                |

A request that sends its own **wins**, because a merchant may legitimately
want one bill to land somewhere else and a default that could not be
overridden would need an operator to edit the deployment's YAML. It is also
what makes the key safe to add to a running deployment: every request that
worked before behaves identically after.

vpay still never invents one. A merchant with neither configured nor passed
gets a `400`, because guessing a "thank you" page would send a paying customer
to a `404`.

**Why an invoice has this and `POST /v1/checkout/sessions` does not.** An
invoice is paid from a link in an e-mail, days after the merchant's process
ran; a checkout session is created by that process, which already has the
payer's context in hand. Repeating two constants on every monthly bill is two
more strings to get wrong on every bill.

**Configured values are validated at boot** — bounded, `http(s)`, a host, no
embedded credentials, `https` under `deployment.livemode` — because a typo in
a value whose whole job is to save every request from repeating it is a typo in
every invoice that merchant ever raises. And they are validated **again** at
request time by the same function a passed URL goes through, so one rule
decides where a payer may be sent rather than two that can drift. See
[../reference/vpay-config.md](../reference/vpay-config.md).

### When the intent succeeds

The **settlement transaction** — the one that takes the charge terminal —
marks the invoice `paid` and emits `invoice.paid` inside itself. Not a second
write afterwards: that would leave a window in which the intent is `succeeded`
and the invoice still says money is owed, and a crash in that window would make
it permanent. A checkout session at least has a page that polls; an invoice has
no poller, no sweep and no job that would ever notice.

`apply_succeeded_pays_the_invoice_the_intent_was_for` and
`an_aborted_settlement_leaves_the_invoice_open_and_emits_nothing` are the two
halves of that claim, both against a real Postgres.

**A failed intent leaves the invoice `open` and emits nothing on the
invoice.** There is no `invoice.payment_failed` — Stripe has one, vpay does
not, and nothing writes it. The merchant learns from
`payment_intent.payment_failed`, which they already receive.

### One payment at a time, and cancelling the intent is the way back

While an intent is attached and **not `canceled`**, `pay`, `void` and
`mark_uncollectible` all answer `409` naming the intent. Cancel it with
`POST /v1/payment_intents/{id}/cancel` and all three become available again.

The condition is `canceled` and deliberately not "not `processing`": a
rail-declined intent lands back on `requires_payment_method`
([payment-lifecycle.md](payment-lifecycle.md)), which is also where a fresh
unconfirmed intent sits, so "not processing" would let a merchant mint a second
intent one millisecond after the first. `canceled` is the one status that says
a human decided this attempt is over. It is also consistent with this
repository's standing rule that a retry is a _new_ PaymentIntent
([AGENTS.md](../../AGENTS.md)).

**Partial payments are out of scope.** `paid_means_nothing_remaining` is where
that stops being a sentence in a document: a `paid` invoice with anything
remaining is a row Postgres refuses.

### Paid out of band

**Built 2026-09-23** (RFC-0004 § 6, step A; migration `0049`). A merchant
records that an **open** invoice was settled outside vpay — cash, a cheque, a
bank transfer received directly, or anything else:

```text
POST /v1/invoices/{id}/pay
paid_out_of_band=true
out_of_band[method]=cash|cheque|bank_transfer|other   optional, default other
out_of_band[reference]=…                               optional, ≤ 500 chars
out_of_band[received_at]=<unix seconds>                optional, default now
```

**`paid_out_of_band=true` alone is enough.** Stripe's `pay` has that flag and
nothing else, so a client written against Stripe sends no `out_of_band[…]` at
all; vpay records it with the method `other`. A method that **is** sent is
still held to the four labels. (Until the review of 2026-09-23 the method was
required and its absence a `400`, which contradicted ADR-0024's D5 — a
Stripe-shaped client must reach the same state — so D11 was reversed to make
it optional; `paid_out_of_band_alone_is_recorded_with_the_method_other` pins
it.)

The invoice moves `open → paid` with `amount_paid = amount_due`,
`amount_remaining = 0`, `status_transitions.paid_at` and
`paid_out_of_band = true`; one `manual_payments` row (`mp_…`) records the
statement; `invoice.paid` is emitted in the same transaction; and the object
answers with `out_of_band_payment` filled in.

**It is a record of what the merchant said, and nothing more.** No money
crossed a rail vpay talks to, nothing in vpay can verify it, and **nothing is
posted to the ledger** — no money crossed `payer_clearing`, and under
pass-through (RFC-0001) vpay has no account it could have arrived in.
`paying_out_of_band_needs_no_configured_urls_and_posts_nothing` asserts the
ledger tables stay empty.

**What refuses it, and in what order.** Every `400` names its parameter:
an unknown `out_of_band[method]` (an absent one is `other`); an `out_of_band[…]` key that is not
one of the three (named as `out_of_band`, never echoed); `reference` over 500
characters; `received_at` that is not a timestamp, is more than 30 seconds in
the future, or is earlier than the second the invoice's `finalized_at` falls
in — both comparisons are **to the second**, see decision 2; `success_url` or
`cancel_url` sent with the flag (there is nowhere to forward anybody);
`out_of_band[…]` sent **without** the flag (a payment described and not asked
to be recorded, refused rather than a checkout minted); and a
`paid_out_of_band` that is neither `true` nor `false`. Then the `409`s the
other transitions give: not `open` (naming the status), and an intent attached
that is not `canceled` (naming the intent — cancel it, exactly as for `void`).
The compare-and-swap is what actually enforces the second; the read only
words the refusal, and a refusal diagnosed after the write names whichever of
the two conditions it found. Another merchant's `in_…` is the uniform `404`
before any of this.

**No configured URLs are needed, by construction.** The `paid_out_of_band`
fork in `pay_once` is taken **before** `forward_urls` resolves
`merchant_clients[].invoices`, so the out-of-band path neither consults them
nor needs a publishable key. The case that proves it pays out of band as a
merchant that has neither.

**The record cannot disagree with its bill, and the database says so.** A
CHECK sees one row of one table, so this is a **composite foreign key**:
`manual_payments_agree_with_their_invoice`, from `manual_payments (invoice_id,
merchant_id, livemode, currency_code, amount, paid_out_of_band)` onto
`invoices_payment_record_key`, a `UNIQUE (id, merchant_id, livemode,
currency_code, amount_paid, paid_out_of_band)` on `invoices` that exists only to
be its target. With `manual_payments.paid_out_of_band` pinned `true` by
`records_an_out_of_band_payment`, a record can exist only for an invoice
flagged `paid_out_of_band`, and only with that invoice's amount, tenant, mode
and currency; `NO ACTION` on update freezes those columns of the invoice once
the record exists. The writer is built so it never needs the key — no caller
supplies any of them; the insert is `INSERT … SELECT invoices.amount_paid, …
FROM invoices WHERE id = $2 AND paid_out_of_band`, in the transaction whose
compare-and-swap just paid the invoice — and the key is the guard that
survives a writer who does not. (This paragraph said "it is not a CHECK" and
that the database could not state it, until the review of 2026-09-23 pointed
out that a composite foreign key can; migration `0049` had not shipped, so it
was edited rather than followed by a `0050`.) The extra index costs one btree
entry per invoice insert and per non-HOT update, and makes no update non-HOT
that was HOT before — every write that changes `amount_paid` or
`paid_out_of_band` also changes the already-indexed `status` — and it is one
undeclared-index line in the drift report, because a `@@unique` over six
columns would generate a name past Postgres's 63-byte limit.
`paying_out_of_band_records_the_invoices_own_amount_and_posts_nothing`
(`vpay-db`'s `tests/repositories.rs`) is the writer's evidence and
`the_out_of_band_invariants_are_enforced_by_the_database_itself` the key's.

**The constraints** (the multi-column ones invisible to `migrate baseline`,
all written straight past the API by
`the_out_of_band_invariants_are_enforced_by_the_database_itself`, each refusal
pinned to its constraint's name): `paid_out_of_band_means_paid`;
`paid_names_how` (a `paid` row was paid by an intent or out of band — it had
been storable with neither); `paid_out_of_band_is_never_refunded`; on
`manual_payments`, `received_before_recorded` and
`records_an_out_of_band_payment`; the composite foreign key above; and one
record per invoice, `manual_payments_invoice_id_key`.

#### The decisions this made, and why — ACCEPTED in ADR-0024 (2026-09-23)

Every decision below is **accepted** in ADR-0024
(`docs/adr/0024-customer-filters-and-manual-payments.md`, PR #248; the path is
written as code rather than a link because the ADR is not on this branch).
The maintainer confirmed D9–D19 "as proposed" on 2026-09-23. The ADR's own
numbers are given beside each item, and the reasons are kept here beside the
behaviour they explain. _(This heading read "PROPOSED, not accepted", and the
paragraph said none was settled until the ADR was accepted, until that
confirmation the same day.)_ Beside the items below the ADR also records D9
(the two new keys on the invoice object), D10 (no redirect URLs on this path,
and none needed) and D19 (unknown `out_of_band[…]` keys are a `400` naming
`out_of_band`; both SDKs take one optional `out_of_band` object). Its open
question 2 — whether `received_at` may precede `finalized_at` — is **settled
by D11: it may not**, to the second, as item 2 describes.

0. **`out_of_band[method]` is optional and defaults to `other`** (D11): a
   Stripe-shaped client sending only `paid_out_of_band=true` must succeed,
   because D5 commits to Stripe's spelling on the existing route.

1. **`out_of_band[reference]` is bounded at 500 characters** (D13). The ceiling
   one `metadata` value has: a reference is one value a merchant attaches — a
   cheque number, a transfer reference — not a note on a document, which is
   what `description`'s 1000 is for. `reference_length` in the database is
   the backstop; the API refuses first, naming the parameter.
2. **`received_at` may be up to 30 seconds in the future** (D14), **and
   no earlier than the invoice's `finalized_at`** (D11). One TOTP step
   (`vpay_api::staff_auth::totp`'s `SKEW_STEPS`) is the only tolerance this
   codebase grants a caller's clock, and a merchant whose clock runs a few
   seconds fast must not be refused for it; anything later is money received
   after vpay recorded that it had been. The same bound is
   `received_before_recorded` in the database. At the other end it may be as
   early as **the second `finalized_at` falls in** and no earlier: both
   comparisons are to the second, because the wire carries whole unix seconds
   and `status_transitions.finalized_at` is rendered floored, so a merchant
   who echoes back the `finalized_at` they were shown — up to 999 ms earlier
   than the stored instant — is accepted. The read of `finalized_at` is
   authoritative rather than a race, because no statement moves it once set.
   `received_at_defaults_to_now_and_is_bounded_on_both_sides` tests both
   boundaries against a `finalized_at` 900 ms into its second.
3. **A canceled intent stays attached** (D15). An invoice whose hosted attempt was
   canceled and which was then paid in cash keeps naming that attempt in
   `payment_intent`, exactly as a voided invoice keeps its intent —
   migration `0036`: "the payment record survives the document". Every CHECK
   admits the row: `only_a_live_invoice_has_an_intent` forbids an intent on a
   draft and nothing else. Nothing can act on the pair afterwards: the
   settlement's lookup needs `status = 'open'`, `attach_intent` needs `open`,
   a canceled intent never settles, and the refund counter now carries
   `AND NOT paid_out_of_band` with `paid_out_of_band_is_never_refunded` behind
   it. `paying_out_of_band_keeps_the_canceled_intent_and_nothing_can_move_it_again`
   pins all of it. The alternative — clearing `payment_intent_id` — would
   erase the record of an attempt a payer may have seen. **One visible
   consequence, recorded rather than changed:** `hosted_invoice_url` on such an
   invoice keeps pointing at the canceled attempt's checkout session, because
   the renderer derives it from the attached intent and does not ask whether
   that intent is still payable. A payer following it meets a session for a
   canceled intent on a bill that is already `paid`; the same is true of a
   voided invoice today.
4. **`paid_names_how`** (D16). Before `0049` the settlement was the only writer of
   `paid` and always matched on an intent, so "paid with neither an intent nor
   the flag" was unreachable and unguarded. A second writer made it worth a
   CHECK: a settled bill nobody can account for is exactly the broken state
   the multi-column CHECKs exist to make unstorable. It changed one existing
   fixture — `the_invoice_invariants_are_enforced_by_the_database_itself`
   wrote two `paid` rows with no intent — and the fix attaches the intent that
   case already creates, and pins each refusal to its **constraint name**, so
   the case still asserts `paid_means_nothing_remaining` and
   `refunded_at_most_paid` rather than being refused by the new CHECK for the
   wrong reason.
5. **A reference is refused on an erased customer's invoice** (D17; the
   lock order that makes it race-free is D18). The reference
   is classified personal data (`payment_reference`, `subject: payer`): a
   cheque or a transfer reference routinely names the payer. Once the payer is
   erased, storing new text about how they paid would re-attach them. The
   payment itself may still be recorded — without the reference — because the
   merchant's books still need the invoice settled. The check is under the
   erasure's own lock (below), so it cannot be raced.

**Erasure, and the lock order.** The customer erasure writes the marker over
`manual_payments.reference` and every copy of it — the `invoice.paid` body,
live deliveries' digests and excerpts, the stored `pay` response — in the
erasure's transaction (`redact_out_of_band_references`), and the idempotency
store runs the same statement on the far side of the issue-#111 race for the
responses it marks `ResponseSubject::OutOfBandInvoice` (the out-of-band `pay`,
and a `POST /v1/invoices/{id}` on an invoice paid out of band). The paying
transaction's **first** statement is `SELECT … FROM customers … FOR SHARE` on
the invoice's customer; every erasure's first statement is `FOR UPDATE` on the
same row, and `POST /v1/customers/{id}` takes `FOR UPDATE` first as well and
touches no invoice. So every transaction that holds both a customer lock and
an invoice or `manual_payments` lock takes the **customer first**, which is
the condition under which two lock-takers cannot form a cycle; the settlement,
`void`, `attach_intent` and the other invoice writes take no customer lock at
all, and `POST /v1/invoices`' foreign-key check takes `FOR KEY SHARE`, which
does not conflict with `FOR SHARE`. That is an argument from the statements,
and `an_erasure_racing_an_out_of_band_payment_neither_deadlocks_nor_leaves_the_reference`
is its empirical half: a `40P01` would surface as a `500`; each round sends an
`Idempotency-Key` and records which side won; the case fails unless at least
one round let the payment write its reference first; and in every such round
the record, the event and the replayed idempotent response carry the marker.
The store's late-write branch — a response stored **after** the erasure —
is driven directly through the repository seam by
`a_stored_invoice_response_written_after_an_erasure_is_redacted_as_it_lands`,
and the bodiless touch by
`a_touch_of_an_invoice_paid_out_of_band_replays_redacted_after_an_erasure`.

---

## Refunds

**Decided 2026-09-10** (issue #91, D5; it was an open maintainer question from
2026-09-07 until then).

A refund against the intent that paid an invoice **leaves the invoice `paid`**
and adds its amount to `amount_refunded`. There is no credit note object, no
sixth status, and no second `invoice.paid`.

| Column             | Before a 2,000 refund on a 5,000 bill | After    |
| ------------------ | ------------------------------------- | -------- |
| `status`           | `paid`                                | `paid`   |
| `amount_due`       | 5000                                  | 5000     |
| `amount_paid`      | 5000                                  | **5000** |
| `amount_remaining` | 0                                     | **0**    |
| `amount_refunded`  | 0                                     | **2000** |

**`amount_refunded` is gross and sits beside the arithmetic, not inside it.**
It is not subtracted from `amount_paid` and takes no part in `amounts_add_up`
— exactly the shape `payment_intents.amount_refunded` has had since migration
`0003`. The alternative (decrement `amount_paid`, let the difference land in
`amount_remaining`, amend `paid_means_nothing_remaining` to tolerate it) makes
a fully refunded invoice read `paid` with the whole bill _remaining_, and
`amount_remaining` is the number `pay` mints an intent for and the number a
merchant chases a payer with. Migration `0042`'s header carries the argument
in full; it is a call a maintainer can reverse, and the two places to change
are that migration and one statement.

**Two guards, and only one of them is visible to any tool.**
`amount_refunded_non_negative` refuses a rebate, which vpay has no concept of.
`refunded_at_most_paid` (`amount_refunded <= amount_paid`) refuses giving back
money nobody collected — including any refund at all against a `void` or
`uncollectible` document, whose `amount_paid` is 0. It is multi-column and
therefore invisible to `cratestack migrate baseline` in both directions, which
is why
`two_refunds_against_one_invoice_add_up_and_an_over_refund_is_refused` reaches
it through the real settlement rather than trusting a drift report.

**The write is in the refund's own settlement transaction.** The `refunds` row
moves `pending` → `succeeded` and the invoice's total moves in the same commit
(`vpay_db::Settlement::apply_refund_succeeded`). A second write afterwards
would leave a window in which money is recorded as returned and the document
still says the whole amount was kept — and an invoice has no poller, no sweep
and no job that would ever notice. The increment is
`amount_refunded = amount_refunded + $n`, an expression over the row's own
column rather than a total read first, so two refunds settling concurrently
add up and the over-refund is refused by the database rather than clamped.

**`invoice.paid` is not re-emitted.** The invoice did not transition. Telling a
merchant a second time that a bill was settled, because part of it came back,
would be a lie about a transition that did not happen.

### None of this is reachable today

**No vpay rail has ever refunded anything, and neither answers
`Unsupported`.** `mtn_momo::refund` is written as of 2026-09-15 (RFC-0003 § 5)
— MTN's Disbursements `transfer` call — but no deployment has ever held a
Disbursements credential and that product has never been called from this
repository, so it answers `ProviderError::Config` wherever it is reached.
`orange_money::refund` is `ProviderError::NotImplemented` since the same day
(RFC-0003 § 5: an Orange refund is an outbound transfer back to the payee,
which Orange makes — what is missing is vpay's call, and this repository has
no Orange transfer specification). _(This read "Orange Money answers
`Unsupported` (its Web Payment product documents no refund API at all)" and
"`mtn_momo::refund` is `ProviderError::NotImplemented`" until that date; the
conclusion below did not move, only the reasons, and they moved in opposite
directions on the two rails.)_
~~`POST /v1/refunds` is unrouted until wave 3~~ — **corrected 2026-09-16:
wave 3 mounted all five refund routes (RFC-0003 § 2)** —
[../status.md](../status.md) carries all three. **`amount_refunded` is still
`0` on every invoice in every deployment** and `apply_refund_succeeded` is
still called by no shipping binary, and the route is not what changes that:
nothing settles a `pending` refund, because the port has no refund status read
and there is no refund poll ladder (RFC-0003 open question 8). The route being
mounted changed the reason that column has no reachable writer, not the fact.
_(This paragraph also read "`vpay_db::Refunds` exposes no `create`" until
2026-09-15, when RFC-0003 § 3 added `Refunds::create` and `Refunds::cancel`;
the conclusion did not move, because no rail and no route can reach them.)_
The cases that prove it now seed their `pending` refund **through
`Refunds::create`** — a hand-written `INSERT` would produce a refund with no
matching reservation, a state the write path cannot reach — while
`backends/tests/integration/tests/refunds.rs` still seeds its rows directly,
for the reason its module header gives.

What is built is the _database's_ answer and the transaction that writes it.
What is not built is everything that would produce a refund in the first
place. The decision was worth landing as a statement rather than a paragraph
because the schema had already taken a position and a sentence in a document
is not something a future writer trips over.

---

## Events

Four types, all Stripe's own spellings, all written **inside the transaction
of the transition they describe**:

| Type                | Written by                                                                                          |
| ------------------- | --------------------------------------------------------------------------------------------------- |
| `invoice.created`   | `POST /v1/invoices`                                                                                 |
| `invoice.finalized` | `POST /v1/invoices/{id}/finalize`                                                                   |
| `invoice.paid`      | the settlement transaction (TX1), **and** `POST /v1/invoices/{id}/pay` with `paid_out_of_band=true` |
| `invoice.voided`    | `POST /v1/invoices/{id}/void`                                                                       |

**`invoice.paid` has two writers since 2026-09-23** (RFC-0004 § 6). It needed
no vocabulary change — the label has been in `type_is_a_documented_event`
since migration `0036`. A merchant tells the two apart by the body:
`paid_out_of_band` is `true` and `out_of_band_payment` is filled in on the
out-of-band writer's, and `false` / `null` on the settlement's. The
out-of-band body carries the record in full because it is the row the same
transaction just inserted — no second query, and no sequence lock is held on
that path.

A refused transition writes **none** — the transaction is abandoned rather
than committed, which is also what makes a refused finalize burn no number.

This resource deliberately does **not** copy [customers.md](customers.md)'s
gap: `customer.created` and `customer.updated` are Stripe types with no writer
and are therefore absent from the database's vocabulary (issue #66). Every one
of the four above has a writer in the same commit that added the label.

**`invoice.*` webhook bodies carry `lines.data` empty.** The event's `data` is
rendered inside the transaction that wrote the row, and reading the lines there
would put a second query on a connection holding the sequence row's lock. A
merchant who needs the lines reads `GET /v1/invoices/{id}`. This is a real
difference from the object a merchant reads over `/v1`, and it is stated here
rather than discovered.

---

## The surface

| Route                                  | Methods                          | Notes                                                                                                                                                                               |
| -------------------------------------- | -------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `/v1/invoices`                         | `POST`, `GET`                    | list takes `customer`, `status`, and the standard cursor                                                                                                                            |
| `/v1/invoices/{id}`                    | `GET`, `POST`, `PATCH`, `DELETE` | `POST`/`PATCH` are one handler; both are draft-only, as is `DELETE`                                                                                                                 |
| `/v1/invoices/{id}/finalize`           | `POST`                           |                                                                                                                                                                                     |
| `/v1/invoices/{id}/void`               | `POST`                           |                                                                                                                                                                                     |
| `/v1/invoices/{id}/mark_uncollectible` | `POST`                           |                                                                                                                                                                                     |
| `/v1/invoices/{id}/pay`                | `POST`                           | `success_url`, `cancel_url` — sent, or from `merchant_clients[].invoices`; **or** `paid_out_of_band=true` + `out_of_band[method\|reference\|received_at]`, with no URL (2026-09-23) |
| `/v1/invoice_items`                    | `POST`                           | no collection `GET` — see below                                                                                                                                                     |
| `/v1/invoice_items/{id}`               | `GET`, `POST`, `PATCH`, `DELETE` | writes are draft-parent-only                                                                                                                                                        |

`PATCH` is mounted beside `POST` on both `{id}` paths. Stripe's API has no
`PATCH` — a merchant's existing client, and the real `stripe` package, send
`POST` — so `POST` is the one that has to work; `PATCH` is there because a
partial update is what the verb means and mounting only one would make somebody
guess. They are the same function, so the two cannot answer differently.

There is **no collection `GET` on `/v1/invoice_items`**. An invoice's lines are
read from the invoice, and a merchant-wide list of every line ever written is a
query nobody asked for over rows that only mean anything beside their parent.

Tenancy, idempotency, error envelopes and cursor rules are
[merchant-auth.md](merchant-auth.md)'s and [../api/README.md](../api/README.md)'s,
unchanged. A merchant asking for another merchant's `in_…` gets the same 404,
byte for byte, as one asking for an id that never existed — on **every** route,
including the transitions, which is what stops a `409` naming a status from
being an existence oracle.

---

## Where the code is

| Concern                 | File                                                                                                            |
| ----------------------- | --------------------------------------------------------------------------------------------------------------- |
| Schema                  | `backends/migrations/0036_create-invoices.sql`, `0042_invoices-amount-refunded.sql`, `0049_manual-payments.sql` |
| Model                   | `schemas/vpay.cstack`, `model Invoice` / `model InvoiceItem` / `model ManualPayment`                            |
| Out-of-band payment     | `backends/crates/vpay-db/src/invoices.rs`, `pay_out_of_band_in_tx`; `vpay-api`'s `pay_out_of_band_once`         |
| Its erasure             | `backends/crates/vpay-db/src/customers.rs`, `redact_out_of_band_references`                                     |
| Repository              | `backends/crates/vpay-db/src/invoices.rs`                                                                       |
| Settlement hook         | `backends/crates/vpay-db/src/settlement.rs`, `flip_invoice`                                                     |
| Refund settlement       | `backends/crates/vpay-db/src/settlement.rs`, `apply_refund_succeeded`                                           |
| Per-merchant `pay` URLs | `backends/crates/vpay-config/src/oauth.rs`, `InvoiceDefaults`                                                   |
| API                     | `backends/crates/vpay-api/src/v1/invoices.rs`, `.../invoice_items.rs`                                           |
| Wire objects            | `backends/crates/vpay-api/src/model.rs`, `InvoiceObject`                                                        |
| Worker projection       | `backends/crates/vpay-worker/src/handlers.rs`, `invoice_snapshot`                                               |
| Status type             | `backends/crates/vpay-core/src/state.rs`, `InvoiceStatus`                                                       |

`invoices` and `invoice_items` are the second and third vpay tables **born**
with a `schemas/vpay.cstack` model, and `manual_payments` (migration `0049`)
is the fourth. Three of the thirteen repository methods run through the
generated data layer (`mark_uncollectible`, `items_for_invoice`, and since
2026-09-23 `manual_payment_for_invoice`); the rest are hand-written for three
separate, measured reasons, and the out-of-band insert for a fourth — it
copies the amount off the invoice inside the statement, which a generated
`create` (values, not expressions) cannot say.
[../reference/vpay-db.md](../reference/vpay-db.md) carries that argument in
full.

`invoices_status_enum_check` is the **first enum CHECK in this repository
created under the name CrateStack generates**, so the declared constraint and
the live one are one object to the diff engine rather than a drop-and-add
pair. Migration 0032 had to rename `providers.flow`'s after the fact.

---

## Status

**Built and proven against a real Postgres and the shipping router (2026-09-07,
S4b, amended the same day by review).**
`backends/tests/integration/tests/invoices.rs` is **sixteen** cases (twelve as
delivered, three from the review — two concurrent `pay` requests attaching
exactly one intent, a foreign list cursor, and the representable ceiling — and
one from 2026-09-10 for D2's forwarding URLs);
`vpay-db`'s `tests/repositories.rs` adds **ten** (the settlement transaction,
the two compare-and-swaps an HTTP test cannot isolate, from the review that the
settlement flips only the invoice its own intent is bound to, and from
2026-09-10 four for the refund settlement — D5);
`postgres_smoke.rs` pins the drift and the multi-column CHECK inventory;
`vpay-db`'s own module adds three with no container; `vpay-api`'s `model`
module pins the wire object's **nineteen** keys.

**Manual (out-of-band) payments — built and proven against a real Postgres
and the shipping router, 2026-09-23 (RFC-0004 § 6, migration `0049`), then
reworked the same day on review.** `invoices.rs` is now **twenty-nine**
cases, 29 passed, 0 ignored — the sixteen above plus thirteen for this: the
happy path for a merchant with no configured URLs and no publishable key,
with no ledger rows; the bare Stripe flag recorded as `other`; ten `400`s each
naming its parameter; the live-intent `409` and success after cancelling it;
refusal from every other status; an idempotent replay; a hosted-vs-out-of-band
race with one winner, five rounds; every new constraint written straight past
the API and pinned by name, the composite foreign key included; the erasure of
the reference everywhere it was copied, webhook deliveries' digests and
excerpts included; an erasure racing a payment, staggered so the rounds must
include one where the payment wrote a reference first; a touch replayed
redacted after an erasure; a stored response that lands after an erasure;
and the cross-merchant `404`. `customers.rs`' whole-database erasure scan
(24 cases, 24 passed, 0 ignored) now seeds a paid-out-of-band invoice whose
reference names the payer. `tests/repositories.rs` adds **two** (twelve on
invoices): the record's amount is the invoice's own and nothing reaches the
ledger, and the canceled intent stays attached and inert. `postgres_smoke.rs`'
migration count, its multi-column CHECK inventory and its drift constants
moved (194 → 199 → **201** over 26 relations, each step derived first and then
measured); `vpay-api`'s `model` module pins **twenty-one** keys
(`the_invoice_object_is_the_documented_twenty_one_keys`) and a flag/record
disagreement in both directions. Two mutations were run against the suite:
resolving `merchant_clients[].invoices` **before** the `paid_out_of_band` fork
turns seven of the new cases red, and removing the intent the rewritten
`the_invoice_invariants_are_enforced_by_the_database_itself` fixture now
attaches turns it red on `paid_names_how`. **Both live SDK suites ran green
against a compose stack** (`just sdk-live`: `sdks/rust` 5 passed, 0 skipped;
`sdks/nodejs` 6 passed), each with its new case —
`an_invoice_is_marked_paid_out_of_band_and_carries_its_record` and `records
an invoice as paid out of band and reads its record back`.
[../status/verification/2026-09-23-manual-payments.md](../status/verification/2026-09-23-manual-payments.md)
has the gate output and every count. _(Twice that day this paragraph had to
record container evidence as not yet run, because the host's Docker died
with a full disk and then again behind a "running" status; both times it was
restarted and every suite here ran before the branch was committed. The live
suites were recorded as "not run against a stack" in the first commit for the
same reason.)_

**The top-level [`README.md`](../../README.md)'s `/v1` route table did not
list any of these eight routes until 2026-09-13**, although this page's own
[surface table](#the-surface) always had. It was not a code gap — `finalize`,
`void`, `mark_uncollectible`, `pay` and the rest shipped in S4b — only a stale
top-level index. `README.md` now lists all eight, matching `vpay_api::V1_ROUTES`.

**The merchant SDKs caught up on 2026-09-08 (exp33).** `sdks/rust` adds
sixteen cases in `tests/resources.rs` (164 in the crate, 0 ignored) and
`sdks/nodejs` seventeen in `src/client.test.ts` (207 in the package, 0
skipped), asserting the exact bytes each of the thirteen methods puts on the
wire and the decode of all **nineteen** keys (eighteen until migration `0042`
added `amount_refunded` on 2026-09-10; **twenty-one** since migration `0049`
on 2026-09-23, when `sdks/rust` gained five out-of-band cases — 183 in the
crate, 0 skipped — and `sdks/nodejs` four — 226 in the package, 0 skipped).
Both are stub-backed, deliberately:
what proves the _server_ is `invoices.rs`, and what these prove is that a
merchant's client sends what the server documents. **Since the exp33 review
the same day, each SDK also has a live suite** — two cases in
`sdks/rust/tests/live_invoices.rs` and three in
`sdks/nodejs/src/invoices.live.test.ts` — driving a real `vpay-server` over a
socket, because "the stub answers the way this SDK expects" is not evidence
about the server and the delivered suites had nothing else.

**Three mutations escaped the suite as delivered and are now caught**
(2026-09-07 review, each measured by applying the mutation and re-running):
deleting `NO_LIVE_INTENT` from `attach_intent` left every wire case green
while two concurrent `pay` requests minted two intents and two hosted URLs for
one bill; unscoping either list cursor left every case green while another
merchant's `in_…` paged the caller's own rows; and keying the settlement's
invoice flip on anything but its intent left all seven delivered invoice cases
green while a settlement paid an invoice it was never bound to.

**What is not built, and is a gap rather than a decision against it:**

- ~~**Neither merchant SDK has invoice methods.**~~ **Closed 2026-09-08
  (exp33): both do.** `sdks/rust`'s `client.invoices()` /
  `client.invoice_items()` and `sdks/nodejs`'s `client.invoices` /
  `client.invoiceItems` each ship thirteen methods — the five CRUD, the four
  transitions, and the four on a line — and both event unions know the four
  `invoice.*` types. Fifteen ✅/✅ rows in
  [../sdks/parity.md](../sdks/parity.md), where three dated ⛔/⛔ rows stood.
  `invoices.rs` still drives raw HTTP and should: it was written before the
  clients existed, and a suite rewritten to drive one would assert the SDK's
  encoding rather than the server's contract.
- ~~**Neither SDK has been exercised against a running vpay**~~ **— closed
  2026-09-08 by the exp33 review.** Each SDK has a live suite that drives a
  real `vpay-server`: `sdks/rust/tests/live_invoices.rs`, a cargo target
  behind the `live-stack` feature (so `cargo nextest run --workspace` neither
  builds nor counts it), and `sdks/nodejs/src/invoices.live.test.ts`, its own
  vitest project excluded from `pnpm test`. `just sdk-live` brings the stack
  up and runs both; CI's `e2e` job runs them beside `sdks/stripe-compat`.
  Neither skips — with no `VPAY_BASE_URL` they fail naming the variable, which
  is the whole difference between this row being closed and it being
  laundered. **The first run found a defect**: `currency` is required on
  `POST /v1/invoices` and both SDKs had it optional, documented as letting
  "the server apply this deployment's own default"; there is no such default,
  and no stub answering `201` to anything could have said so.
  `invoices.rs` is still what proves the _server_, over a socket, against a
  real Postgres.
- **The Stripe-compat suite still has no invoice cases.** `sdks/stripe-compat`
  drives the real `stripe@22.6.1` package rather than either merchant SDK, so
  the row above says nothing about it. It gets no parity rows of its own
  (ADR-0015 decision 4), and this is recorded here instead.
- **No Cypress case renders an invoice's `hosted_invoice_url`.** The URL is
  asserted to be a real checkout-session URL by the integration suite, and the
  checkout page it points at is covered by `checkout.cy.ts` — but nothing has
  driven a browser from an invoice to a paid charge.
- **No PDF, no e-mail, no hosted invoice page.** There is nothing to send and
  nothing to send it with. `hosted_invoice_url` is a checkout session, not a
  document.
- **No taxes, no discounts, no credit notes.** A line is a description, a
  quantity and a unit amount. A merchant who needs VAT puts it on a line.
- **No subscriptions and no recurring invoices.** The maintainer's decision of
  2026-09-05 is that these will be built in-house on CrateStack when they are
  built; nothing here forecloses it, and `invoice_items` deliberately has no
  `subscription` column rather than a null one.
- **No dunning and no automatic `uncollectible`.** `due_date` is stored and
  read by nothing. There is no reminder job, no retry schedule and no timer
  that moves an invoice anywhere.
- **`invoice.marked_uncollectible` and `invoice.payment_failed` are not
  emitted.** Both are Stripe types; neither is in
  `type_is_a_documented_event`, because nothing writes them. Migration
  `0023`'s rule applied, and `customers`' `customer.created` is the standing
  precedent. A merchant learns about a write-off from
  `GET /v1/invoices?status=uncollectible`, and about a failed payment from
  `payment_intent.payment_failed`, which they already receive.
- **`invoice.*` webhook bodies carry empty `lines`.** See
  [Events](#events). The `/v1` object always carries them.
- **Recording an out-of-band payment does not stamp the customer's
  `last_used_at`** (2026-09-23), so it does not restart their twelve-month
  retention clock. Hosted `pay` does not either — only creating the invoice
  does, through `resolve_for_attachment` — so this is the existing gap, not a
  new one; it is listed because paying is the most recent use of a payer a
  merchant has.
- **Partial payments.** One intent at a time, and `paid` means paid in full.
  A merchant taking a deposit issues two invoices. An out-of-band payment is
  all-or-nothing too (one `manual_payments` row per invoice, 2026-09-23).
- ~~**A merchant paid in cash, by cheque or by a transfer received directly
  cannot record it**; they can only `void` or `mark_uncollectible` a bill that
  was in fact settled~~ — **built 2026-09-23 (RFC-0004 § 6):
  `POST /v1/invoices/{id}/pay` with `paid_out_of_band=true`**, see
  [Paid out of band](#paid-out-of-band). _(This gap was stated in RFC-0004's
  problem list and not on this page, which is itself the kind of omission this
  list exists to prevent; recorded here on the day it closed.)_ What stays
  unbuilt beside it: **no operator can record one** — only the merchant `/v1`
  surface writes it, because ADR-0008's dashboard writes and their audit log
  are unbuilt; **nothing in vpay verifies the statement**; **no automatic
  matching** of a bank transfer to an invoice (that is RFC-0007); and **no
  way to undo one** — a paid invoice is terminal, as it always was.
- **The dashboard has no invoice screen.** `/dash/v1` exposes nothing about
  this resource.
- **The payer's checkout page does not show the invoice number** (2026-09-07).
  See [Paying](#paying): the number is in the intent's `description` and on
  the wire the page reads; the page renders the amount and the merchant name
  only.
- ~~**A refund does not move an invoice** (2026-09-07)~~ **— decided
  2026-09-10 (issue #91, D5); see [Refunds](#refunds).** The invoice stays
  `paid` and gains `amount_refunded`. No credit note and no sixth status.
  **No rail can still refund**, so the column is `0` in every deployment; what
  changed is that the answer is now a statement rather than an open question.
- **An invoice cannot be issued past `2^53 - 1` minor units** (2026-09-07,
  review). `POST /v1/invoices/{id}/finalize` answers `400` naming `invoice`
  above it, because `pay` mints its intent for `amount_remaining` without
  going through `POST /v1/payment_intents`' own `parse_amount`, and a larger
  number is silently rounded by every JSON client — both vpay SDKs included.
  It is not a limit anyone will meet: 9,007,199,254,740,991 FCFA is four
  orders of magnitude above Cameroon's money supply. It is enforced because
  reaching it needed ninety-one lines and no privilege at all.
