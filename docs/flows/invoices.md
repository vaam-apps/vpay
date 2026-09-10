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

| Field                                             | Meaning                                                                            |
| ------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `id`                                              | `in_…`, minted before the row exists                                               |
| `customer`                                        | the `cus_…` this bills. **Required**                                               |
| `currency`                                        | lower-case ISO-4217; every line is in it                                           |
| `status`                                          | `draft` → `open` → `paid` \| `void` \| `uncollectible`                             |
| `number`                                          | `{prefix}-{000001}`, assigned at finalize, `null` while a draft                    |
| `amount_due` / `amount_paid` / `amount_remaining` | integer minor units ([money.md](money.md))                                         |
| `amount_refunded`                                 | how much of `amount_paid` has been given back — **gross**, see [Refunds](#refunds) |
| `due_date`                                        | unix seconds, **advisory** — nothing in vpay reads it                              |
| `description`, `metadata`                         | the merchant's own                                                                 |
| `payment_intent`                                  | the `pi_…` paying it, or `null`                                                    |
| `hosted_invoice_url`                              | the checkout session for that intent, or `null`                                    |
| `lines`                                           | every line, expanded, as a `list`                                                  |
| `status_transitions`                              | `finalized_at`, `paid_at`, `voided_at`, `marked_uncollectible_at`                  |
| `created`, `livemode`                             | as everywhere else                                                                 |

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
= $1 AND merchant_id = $2 AND status = '<from>'`. "Matched no row" _is_ the
   refusal. There is no `can_transition_to` anywhere in this codebase, and
   [`vpay_core::InvoiceStatus`](../reference/vpay-core.md) says why: a Rust
   guard beside the write is the thing a future writer calls _instead of_
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

**No vpay rail can refund.** `mtn_momo::refund` is
`ProviderError::NotImplemented` (refunds are MTN's Disbursements product, for
which this deployment has never held a credential) and Orange Money answers
`Unsupported` (its Web Payment product documents no refund API at all).
`POST /v1/refunds` is unrouted and `vpay_db::Refunds` exposes no `create` —
[../status.md](../status.md) carries all four. So **`amount_refunded` is `0`
on every invoice in every deployment**, `apply_refund_succeeded` is called by
no shipping binary, and the cases that prove it seed a `pending` refunds row
with a raw `INSERT`, exactly as `backends/tests/integration/tests/refunds.rs`
already does.

What is built is the _database's_ answer and the transaction that writes it.
What is not built is everything that would produce a refund in the first
place. The decision was worth landing as a statement rather than a paragraph
because the schema had already taken a position and a sentence in a document
is not something a future writer trips over.

---

## Events

Four types, all Stripe's own spellings, all written **inside the transaction
of the transition they describe**:

| Type                | Written by                        |
| ------------------- | --------------------------------- |
| `invoice.created`   | `POST /v1/invoices`               |
| `invoice.finalized` | `POST /v1/invoices/{id}/finalize` |
| `invoice.paid`      | the settlement transaction (TX1)  |
| `invoice.voided`    | `POST /v1/invoices/{id}/void`     |

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

| Route                                  | Methods                          | Notes                                                                     |
| -------------------------------------- | -------------------------------- | ------------------------------------------------------------------------- |
| `/v1/invoices`                         | `POST`, `GET`                    | list takes `customer`, `status`, and the standard cursor                  |
| `/v1/invoices/{id}`                    | `GET`, `POST`, `PATCH`, `DELETE` | `POST`/`PATCH` are one handler; both are draft-only, as is `DELETE`       |
| `/v1/invoices/{id}/finalize`           | `POST`                           |                                                                           |
| `/v1/invoices/{id}/void`               | `POST`                           |                                                                           |
| `/v1/invoices/{id}/mark_uncollectible` | `POST`                           |                                                                           |
| `/v1/invoices/{id}/pay`                | `POST`                           | `success_url`, `cancel_url` — sent, or from `merchant_clients[].invoices` |
| `/v1/invoice_items`                    | `POST`                           | no collection `GET` — see below                                           |
| `/v1/invoice_items/{id}`               | `GET`, `POST`, `PATCH`, `DELETE` | writes are draft-parent-only                                              |

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

| Concern                 | File                                                                                |
| ----------------------- | ----------------------------------------------------------------------------------- |
| Schema                  | `backends/migrations/0036_create-invoices.sql`, `0042_invoices-amount-refunded.sql` |
| Model                   | `schemas/vpay.cstack`, `model Invoice` / `model InvoiceItem`                        |
| Repository              | `backends/crates/vpay-db/src/invoices.rs`                                           |
| Settlement hook         | `backends/crates/vpay-db/src/settlement.rs`, `flip_invoice`                         |
| Refund settlement       | `backends/crates/vpay-db/src/settlement.rs`, `apply_refund_succeeded`               |
| Per-merchant `pay` URLs | `backends/crates/vpay-config/src/oauth.rs`, `InvoiceDefaults`                       |
| API                     | `backends/crates/vpay-api/src/v1/invoices.rs`, `.../invoice_items.rs`               |
| Wire objects            | `backends/crates/vpay-api/src/model.rs`, `InvoiceObject`                            |
| Worker projection       | `backends/crates/vpay-worker/src/handlers.rs`, `invoice_snapshot`                   |
| Status type             | `backends/crates/vpay-core/src/state.rs`, `InvoiceStatus`                           |

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

**The merchant SDKs caught up on 2026-09-08 (exp33).** `sdks/rust` adds
sixteen cases in `tests/resources.rs` (164 in the crate, 0 ignored) and
`sdks/nodejs` seventeen in `src/client.test.ts` (207 in the package, 0
skipped), asserting the exact bytes each of the thirteen methods puts on the
wire and the decode of all **nineteen** keys (eighteen until migration `0042`
added `amount_refunded` on 2026-09-10). Both are stub-backed, deliberately:
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
- **Partial payments.** One intent at a time, and `paid` means paid in full.
  A merchant taking a deposit issues two invoices.
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
