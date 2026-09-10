# exp47 — invoice follow-ups (issue #91 items 3 and 4)

Branch `claude/exp47-invoice-followups`, base `44e0c80` (master).
Date 2026-09-10.

Two of the brief's four items are built and proven. Two are **not attempted**
and are listed at the bottom with what would be involved. This page exists for
one reason above the others: the brief pre-decided a schema change that turned
out not to follow from the decision it cited, and the deviation is recorded
here rather than absorbed silently.

---

## The deviation: `paid_means_nothing_remaining` was not amended

**The brief said:** "the invoice stays `paid` and gains `amount_refunded`
(Stripe-shaped) … the `paid_means_nothing_remaining` CHECK is amended in
migration 0042 so a refunded paid invoice is storable", with a decisive
mutation of "drop the CHECK amendment → the storable test FAILS".

**What was built:** the column, the two new CHECKs it needs, and **no
amendment to `paid_means_nothing_remaining`**.

**Why.** The constraint is `status <> 'paid' OR amount_remaining = 0`. Under
the shape the issue itself decided — Stripe-shaped, a *gross* counter beside
the money columns — a refund moves neither `amount_paid` nor
`amount_remaining`, so a refunded paid invoice satisfies it already, and
`amounts_add_up` too. The amendment would have been vacuous, and its named
mutation could not have fired: there is no storable-test that fails when a
vacuous constraint change is reverted.

The only design under which that sentence is true is the one D5's own text
describes as the *problem* rather than the answer: decrement `amount_paid`,
let `amounts_add_up` push the difference into `amount_remaining`, and relax the
CHECK to `amount_remaining = amount_refunded`. That was written, considered and
rejected here, because it makes a fully refunded invoice read

```
status = paid   amount_paid = 0   amount_remaining = 5000   amount_refunded = 5000
```

— a settled document claiming the payer owes the whole bill again.
`amount_remaining` is the number `POST /v1/invoices/{id}/pay` mints an intent
for, the number the merchant SDKs surface, and the number a merchant chases a
payer with. On the money path that is the misreading least worth taking.

The schema already had the precedent: `payment_intents.amount_refunded`
(migration `0003`) is a gross counter beside `amount`, with
`amount_refunded_non_negative` and `no_over_refund` guarding it, and migration
`0017`'s GAP note describes the writer that would move it. `invoices` now has
the same shape.

**What replaces the mutation.** The load-bearing CHECK is a *new* one,
`refunded_at_most_paid` (`amount_refunded <= amount_paid`), and it is what
makes the settlement fail closed. Both required mutations were run:

| Mutation | Result |
|---|---|
| Move the `amount_refunded` write out of the settlement transaction (commit the refund flip, update the invoice on the pool) | `two_refunds_against_one_invoice_add_up_and_an_over_refund_is_refused` **FAILS** — refund left `succeeded`, invoice unmoved |
| Delete `refunded_at_most_paid` from migration `0042` | the same case **FAILS** — 5,500 refunded committed against a 5,000 bill |

**This is reversible, and it is a maintainer's call.** If the gross shape is
wrong, two places change: migration `0042` (the CHECK) and
`vpay_db::invoices::add_refund_for_intent_in_tx` (one statement). Nothing else
in the tree assumes either shape.

---

## What was built

### 1. D5 — refunds against a paid invoice

- Migration `0042_invoices-amount-refunded.sql`: `amount_refunded BIGINT NOT
  NULL` with **no DEFAULT** (added with one purely to backfill, dropped on the
  next line), `amount_refunded_non_negative`, `refunded_at_most_paid`.
- `model Invoice` declares it, so the column contributes **zero** drift; the
  +1 is `amount_refunded_non_negative` as an undeclared single-column CHECK.
  `refunded_at_most_paid` is multi-column and invisible to `migrate baseline`,
  so it is pinned in `postgres_smoke.rs`'s live `pg_constraint` inventory
  instead.
- `Settlement::apply_refund_succeeded` — one transaction, two statements: the
  `refunds` row `pending → succeeded` (compare-and-swap), then
  `amount_refunded = amount_refunded + $n` on the invoice the refunded intent
  paid (compare-and-swap on `status = 'paid'`). Both statement functions are
  `pub(crate)`, so the pairing is enforced by visibility rather than by
  convention.
- The increment is an **expression over the row's own column**, never a total
  read first: two refunds settling concurrently add up, and the second blocks
  on the row lock and re-evaluates the CHECK against the first's committed
  value.
- `invoice.paid` is **not** re-emitted, and no refund event is emitted at all.
- `InvoiceObject` gains a nineteenth key; both SDK types carry it, tolerant of
  a pre-`0042` server, and both **live** suites assert the server sends it —
  the offline fixtures cannot, because the field defaults.

**Nothing in a shipping binary calls any of it**, and every doc that mentions
it says so. No rail can refund, `POST /v1/refunds` is unrouted,
`vpay_db::Refunds` still exposes no `create`. The four cases seed a `pending`
refunds row with a raw `INSERT`, exactly as
`backends/tests/integration/tests/refunds.rs` already does. What is proven is
what the *database* does when a refund lands; nothing about how one comes to
exist.

### 2. D2 — per-merchant default return URLs

- `merchant_clients[].invoices.success_url` / `.cancel_url`, validated at boot
  (`vpay_config`'s `validate_invoice_urls`) and **again** at request time
  through the same `checked_forward_url` a passed URL goes through.
- Resolution is request → configuration → refuse. A passed URL always wins.
- Neither sent nor configured is **one** `400` naming both, kept under
  `ApiError`'s 200-character ceiling so the sentence about configuring them
  survives truncation.
- A query string and a fragment are legal here and are refused on
  `checkout.public_base_url`, because vpay appends `/c/{id}` to that value and
  appends nothing to these. That asymmetry is the thing most likely to be
  "tidied" into a bug later, so it has a test on each side of the crate
  boundary:
  `an_invoice_url_is_bounded_http_s_and_may_carry_a_query_or_a_fragment`
  (vpay-config) and
  `every_invoice_url_shape_the_config_admits_is_admitted_by_the_request_path`
  (vpay-api), reading the same four values.
- Mutation: ignore the configured default in `forward_urls` → the wire case
  **FAILS** (a merchant that configured both gets a `400`).

---

## What was NOT done

**Item 3 of the brief — the payer sees the invoice number (D6).** Not
attempted. It is a change to `frontends/apps/checkout` plus, on the reading the
brief prefers, a new structured `invoice_number` on the browser session read
(`GET /v1/browser/checkout/sessions/{id}`) with the uniform-404 rules intact —
a server change, a page change, a vitest suite and a screenshot. The gap row in
`docs/flows/invoices.md` and `docs/status.md` is **unchanged and still open**.

**Item 4 of the brief — a Cypress case through `hosted_invoice_url`.** Not
attempted. `just test-e2e` shares fixed Cypress fixture ports (4180/4181) on
master and another agent's stack was up on this machine for most of this pass;
running it would have been a collision, and running it *badly* would have been
worse than not running it. The gap row is unchanged and still open.

Both are listed in the brief's own priority order — items 1 and 2 first if the
four were more than one pass. They were.

**Also not done, and deliberately:** `payment_intents.amount_refunded` and
`amount_refund_pending` are **not** maintained by the refund settlement.
Migration `0017`'s GAP note pairs them with the `INSERT` that creates a
`refunds` row, and that insert does not exist. Incrementing one half of a
paired total whose other half nothing writes would leave `no_over_refund`
counting money twice the day the insert lands. Migration `0042`'s footer says
so, in the file a future writer of that insert will read.

---

## Migration numbering

`0042`, not `0040`. `0040` (exp44) and `0041` (exp46) were taken by branches in
flight when this one was written, so this tree had a two-wide gap in the
prefixes. `sqlx::migrate!` orders by prefix and does not require density;
`schema_migrates_cleanly_on_an_empty_database` counts **rows** in
`_sqlx_migrations` rather than assuming the highest prefix is the count, and
its message says so.

**Amended 2026-09-11 by the review.** `0040` (issue #88) landed on `master` in
PR #108 while this branch was open, and the rebase brought it in: the gap is
now **one** wide and sits at `0041`, and the assertion is **41** rows, not 40.
The number in this paragraph was one of three places on the branch that the
rebase falsified — see `opus-review.md`.
