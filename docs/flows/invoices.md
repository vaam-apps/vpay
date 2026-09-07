# Invoices

A merchant's bill to one customer: what it is made of, how it moves, what
refuses each move, and what a merchant is told when it does.

This is S4b of the data-layer work, built on S4a's
[Customer](customers.md). (The plan document itself is not in this repository
— migration `0034`'s header cites `docs/plans/2026-09-06-data-layer.md` and no
such file is tracked. That citation is wrong there and is not repeated here.) It is Stripe's `invoice`, narrowed to the
subset a Cameroon merchant needs to bill a phone — there is no PDF, no e-mail,
no tax, no credit note, no dunning and no subscription. Every one of those is
listed under [What is not built](#what-is-not-built) with a date, because a
document that lists only what exists is how somebody comes to believe an
invoice gets sent.

---

## The two objects

### Invoice — `in_…`

| Field | Meaning |
|---|---|
| `id` | `in_…`, minted before the row exists |
| `customer` | the `cus_…` this bills. **Required** |
| `currency` | lower-case ISO-4217; every line is in it |
| `status` | `draft` → `open` → `paid` \| `void` \| `uncollectible` |
| `number` | `{prefix}-{000001}`, assigned at finalize, `null` while a draft |
| `amount_due` / `amount_paid` / `amount_remaining` | integer minor units ([money.md](money.md)) |
| `due_date` | unix seconds, **advisory** — nothing in vpay reads it |
| `description`, `metadata` | the merchant's own |
| `payment_intent` | the `pi_…` paying it, or `null` |
| `hosted_invoice_url` | the checkout session for that intent, or `null` |
| `lines` | every line, expanded, as a `list` |
| `status_transitions` | `finalized_at`, `paid_at`, `voided_at`, `marked_uncollectible_at` |
| `created`, `livemode` | as everywhere else |

**`customer` is required, and a payment intent's is not.** An invoice is a
bill *to somebody*: it carries a number a merchant quotes in a conversation,
it may be chased for months, and on this market the payer's identity is a
phone number (the maintainer's decision of 2026-09-05, [customers.md](customers.md)).
An invoice with no customer is a bill nobody could be asked to pay, and
`POST /v1/invoices/{id}/pay` would have no payer to bind an intent to.

### Line — `ii_…`, rendered as `line_item`

`id`, `description`, `quantity`, `unit_amount`, `amount`, `currency`,
`livemode`. Nothing else: no price, no product, no proration, no period.

**The route says `invoice_items` and the object says `line_item`, and that is
deliberate.** Stripe has two objects where vpay has one. An `invoiceitem`
there is a *pending* charge not yet attached to any document; a `line_item` is
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
       ▼              │  settlement ▼
   ┌──────┐           ▼        ┌───────────────┐
   │ void │      ┌──────┐      │ uncollectible │
   └──────┘      │ paid │      └───────────────┘
                 └──────┘
```

Every one of those arrows is terminal on the right-hand side. Nothing in vpay
moves an invoice out of `paid`, `void` or `uncollectible`, and no route tries.

### Three enforcers, and none of them is a validation function

1. **`invoices_status_enum_check`** closes the vocabulary at five labels.
2. **Every transition is a compare-and-swap.** `UPDATE invoices SET … WHERE id
   = $1 AND merchant_id = $2 AND status = '<from>'`. "Matched no row" *is* the
   refusal. There is no `can_transition_to` anywhere in this codebase, and
   [`vpay_core::InvoiceStatus`](../reference/vpay-core.md) says why: a Rust
   guard beside the write is the thing a future writer calls *instead of*
   taking the lock.
3. **Five multi-column CHECKs** make the combinations a broken transition
   would produce unstorable — `number_is_assigned_at_finalize`,
   `paid_means_nothing_remaining`, `only_a_live_invoice_has_an_intent`,
   `amounts_add_up`, `amount_is_the_product`. They are the guard that survives
   a future writer who forgets rule 2.

Rule 3's constraints are **invisible to `cratestack migrate baseline` in both
directions**, exactly as S4a's `at_least_one_identifier` is, so the drift
report cannot be the guard for them.
`backends/tests/integration/tests/invoices.rs`'s
`the_invoice_invariants_are_enforced_by_the_database_itself` writes the row
each one exists to refuse, straight past the API and the repository.

### A draft is deleted; an issued invoice is voided

Stripe's own shape, and here it is *forced* rather than chosen:
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

### It takes `success_url` and `cancel_url`, and Stripe's `pay` does not

Stripe's `pay` charges a payment method the merchant already has on file, so
there is nowhere to send anybody. vpay has no stored payment methods on this
market — a mobile-money payment is a payer approving a prompt on their own
handset — so paying means creating a hosted session, and migration `0028`'s
`urls_match_ui_mode` requires both URLs on one. They are **required** rather
than defaulted, because vpay does not know where a merchant's "thank you" page
is and inventing one would send a paying customer to a `404`.

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
repository's standing rule that a retry is a *new* PaymentIntent
([AGENTS.md](../../AGENTS.md)).

**Partial payments are out of scope.** `paid_means_nothing_remaining` is where
that stops being a sentence in a document: a `paid` invoice with anything
remaining is a row Postgres refuses.

---

## Events

Four types, all Stripe's own spellings, all written **inside the transaction
of the transition they describe**:

| Type | Written by |
|---|---|
| `invoice.created` | `POST /v1/invoices` |
| `invoice.finalized` | `POST /v1/invoices/{id}/finalize` |
| `invoice.paid` | the settlement transaction (TX1) |
| `invoice.voided` | `POST /v1/invoices/{id}/void` |

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

| Route | Methods | Notes |
|---|---|---|
| `/v1/invoices` | `POST`, `GET` | list takes `customer`, `status`, and the standard cursor |
| `/v1/invoices/{id}` | `GET`, `POST`, `PATCH`, `DELETE` | `POST`/`PATCH` are one handler; both are draft-only, as is `DELETE` |
| `/v1/invoices/{id}/finalize` | `POST` | |
| `/v1/invoices/{id}/void` | `POST` | |
| `/v1/invoices/{id}/mark_uncollectible` | `POST` | |
| `/v1/invoices/{id}/pay` | `POST` | requires `success_url`, `cancel_url` |
| `/v1/invoice_items` | `POST` | no collection `GET` — see below |
| `/v1/invoice_items/{id}` | `GET`, `POST`, `PATCH`, `DELETE` | writes are draft-parent-only |

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

| Concern | File |
|---|---|
| Schema | `backends/migrations/0036_create-invoices.sql` |
| Model | `schemas/vpay.cstack`, `model Invoice` / `model InvoiceItem` |
| Repository | `backends/crates/vpay-db/src/invoices.rs` |
| Settlement hook | `backends/crates/vpay-db/src/settlement.rs`, `flip_invoice` |
| API | `backends/crates/vpay-api/src/v1/invoices.rs`, `.../invoice_items.rs` |
| Wire objects | `backends/crates/vpay-api/src/model.rs`, `InvoiceObject` |
| Worker projection | `backends/crates/vpay-worker/src/handlers.rs`, `invoice_snapshot` |
| Status type | `backends/crates/vpay-core/src/state.rs`, `InvoiceStatus` |

`invoices` and `invoice_items` are the second and third vpay tables **born**
with a `schemas/vpay.cstack` model. Two of the twelve repository methods run
through the generated data layer (`mark_uncollectible`, `items_for_invoice`);
the rest are hand-written for three separate, measured reasons.
[../reference/vpay-db.md](../reference/vpay-db.md) carries that argument in
full.

`invoices_status_enum_check` is the **first enum CHECK in this repository
created under the name CrateStack generates**, so the declared constraint and
the live one are one object to the diff engine rather than a drop-and-add
pair. Migration 0032 had to rename `providers.flow`'s after the fact.

---

## Status

**Built and proven against a real Postgres and the shipping router (2026-09-07,
S4b).** `backends/tests/integration/tests/invoices.rs` is twelve cases;
`vpay-db`'s `tests/repositories.rs` adds five (the settlement transaction and
the two compare-and-swaps an HTTP test cannot isolate);
`postgres_smoke.rs` pins the drift and the multi-column CHECK inventory;
`vpay-db`'s own module adds three with no container.

**What is not built, and is a gap rather than a decision against it:**

- **Neither merchant SDK has invoice methods.** Not the Rust one, not the Node
  one, and the Stripe-compat suite has no invoice cases. Dated ⛔/⛔ rows in
  [../sdks/parity.md](../sdks/parity.md). This is the largest gap in this
  change and it is why `invoices.rs` drives raw HTTP rather than a client:
  writing the suite against an SDK that did not exist would have been the
  "test asserts the implementation back to itself" failure
  [../../CLAUDE.md](../../CLAUDE.md) names.
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
- **Partial payments.** One intent at a time, and `paid` means paid in full.
  A merchant taking a deposit issues two invoices.
- **The dashboard has no invoice screen.** `/dash/v1` exposes nothing about
  this resource.
- **An invoice cannot be issued past `2^53 - 1` minor units** (2026-09-07,
  review). `POST /v1/invoices/{id}/finalize` answers `400` naming `invoice`
  above it, because `pay` mints its intent for `amount_remaining` without
  going through `POST /v1/payment_intents`' own `parse_amount`, and a larger
  number is silently rounded by every JSON client — both vpay SDKs included.
  It is not a limit anyone will meet: 9,007,199,254,740,991 FCFA is four
  orders of magnitude above Cameroon's money supply. It is enforced because
  reaching it needed ninety-one lines and no privilege at all.
