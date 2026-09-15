# Ledger

Double-entry. Convention: `balance(account) = SUM(credit) - SUM(debit)`.
`merchant_payable` is credit-normal — a positive balance is money the merchant
received.

## Postings

**Capture of 5,000 XAF, no fee**

| Account            | Direction | Amount |
| ------------------ | --------- | ------ |
| `payer_clearing`   | debit     | 5000   |
| `merchant_payable` | credit    | 5000   |

**Capture with a 100 XAF platform fee**

| Account                | Direction | Amount |
| ---------------------- | --------- | ------ |
| `payer_clearing`       | debit     | 5000   |
| `merchant_payable`     | credit    | 4900   |
| `platform_fee_revenue` | credit    | 100    |

**Refund of 2,000 XAF** (fee not refunded — Stripe's default)

| Account            | Direction | Amount |
| ------------------ | --------- | ------ |
| `merchant_payable` | debit     | 2000   |
| `payer_clearing`   | credit    | 2000   |

## When refunds post

A refund is asynchronous, so:

- **On creation:** increment `amount_refund_pending` under a row lock. This
  must not be an application read-then-check, which races on two concurrent
  over-refunds. **Correction of the correction:** an earlier pass through
  this doc said a database CHECK claim was false, because at the time there
  was neither a database schema nor `schemas/vpay.cstack`'s grammar could
  express a cross-column constraint. The schema has since been implemented
  in raw SQL. `payment_intents` now carries `CONSTRAINT no_over_refund CHECK
(amount_refunded + amount_refund_pending <= amount)`
  (`backends/migrations/0003_create-payment-intents.sql:73`), proven to fire
  by `over_refund_is_rejected_by_the_database` in
  `backends/tests/integration/tests/postgres_smoke.rs` against a real
  Postgres 16.

  **Be precise about what this does and does not guarantee** (mirroring the
  comment block on the constraint itself in the migration). It **does**
  guarantee, unconditionally and including under concurrency, that no
  _committed_ row can ever end up over-refunded: two concurrent `UPDATE`s
  racing to increment `amount_refund_pending` still serialize at the
  database — the second writer blocks on the row lock Postgres's MVCC
  already takes for any `UPDATE`, then re-evaluates the CHECK against the
  first writer's committed value, so the second racing writer fails the
  CHECK instead of silently over-committing. It does **not** mean an
  application-level `SELECT ... FOR UPDATE` (or equivalent explicit locking)
  exists anywhere in this repo — nothing persists refunds yet, so there is
  no application code path that could even reach this constraint outside of
  a test issuing raw SQL directly. The row-lock-and-recheck semantics above
  are a property of the CHECK plus Postgres's own MVCC, not of anything vpay
  has written.

  Separately, the only over-refund guard that exists in Rust today remains
  narrower and non-concurrent: `Money::checked_sub` in
  `backends/crates/vpay-core/src/money.rs` rejects an arithmetic result that
  would go negative (tested by `refunding_more_than_captured_is_rejected`),
  which stops a single refund larger than what remains captured but says
  nothing about two refunds racing each other. **No ledger posting yet** —
  no capture and no refund has ever produced a `ledger_transactions` or
  `ledger_entries` row in any deployment. That sentence used to add "nothing
  in the application writes to the ledger tables today"; since 2026-09-15
  (RFC-0003 § 4) a writer exists — `vpay_db::ledger::post_in_tx` — and
  **nothing calls it**, which is the half that keeps the claim true. See
  § Status.

- **On success:** in one transaction, decrement pending, increment refunded,
  write the ledger transaction.
- **On failure:** decrement pending only. Nothing was posted, so **no reversal
  entry is needed** — which is why the reservation column exists rather than
  posting optimistically and unwinding.

## A rail-charged refund fee is reported, not posted

**Decided 2026-09-05 ([issue #46](https://github.com/vaam-apps/vpay/issues/46)),
and stated here because "it does not post" is a decision, not an omission.**

The `refund` object carries a `fee` — what the rail charged _us_ to move the
money back ([merchant-auth.md](merchant-auth.md)). It is **reported to the
merchant and posted nowhere.** None of the three postings above gains an
entry, `platform_fee_revenue` is untouched, and the refund posting stays the
two lines it is today.

Why not post it. A posting rule has to answer _who pays_, and that answer is
not vpay's: the integrator whose report opened the issue needs the fee to
follow fault (their platform eats it on a platform error, the merchant on
theirs), which is a marketplace judgement about a specific order, not
something a rail's response contains. Writing a rule now would mean choosing
one of those answers for every deployment. And there is a plainer reason:
**no rail reports a refund fee to this repository today**, so any posting rule
would be written against a number that has never existed and tested against a
fixture — see [../status.md](../status.md).

**The invariant that changes when it does post** is invariant 2 below,
`balance(merchant_payable) = Σ captures − Σ fees − Σ refunds`. `Σ fees` is
capture-time `platform_fee_revenue` today. A merchant-borne refund fee adds a
second kind of term to it, and the invariant would have to say which — at
which point invariant 2 also finally needs the per-merchant dimension
`AccountKind` still does not have (see below). Nothing in this repository may
start posting a refund fee without changing that line and the `AccountKind`
gap in the same commit; a fee that debits `merchant_payable` while invariant 2
still reads only capture fees is an invariant that quietly stops holding.

**What is deliberately not on the object either**, and belongs to the
integrator rather than to vpay: `fee_borne_by` and `fee_settlement_ref`. vpay
reports what the movement cost; who eats it is a marketplace decision.

## Invariants (asserted nightly)

1. Per transaction: `SUM(debit) = SUM(credit)`, per currency.
2. Per merchant: `balance(merchant_payable) = Σ captures − Σ fees − Σ refunds`.
3. `amount_refunded` equals the sum of succeeded refunds for that intent.
4. Every succeeded charge has exactly one capture transaction.

**Invariant 1 is deliberately not a database constraint, and won't become
one.** `SUM(debit) = SUM(credit)` is an aggregate over every `LedgerEntry`
row sharing a `transaction_id` — a row-level SQL `CHECK` evaluates one row
at a time and cannot see its siblings, so no schema grammar (raw SQL
included, not just `schemas/vpay.cstack`'s CrateStack subset) can express it
that way without a trigger. This invariant stays application-enforced, in
`vpay_ledger::Transaction::validate()`, tested by
`a_capture_with_a_fee_balances` and `an_unbalanced_transaction_is_rejected`.
The `LedgerTransaction` model's own `GAP` comment in `schemas/vpay.cstack`
says the same thing.

**~~Invariant 2 has a modelling gap, surfaced while writing the design-sketch
schema in `schemas/vpay.cstack`.~~ Closed on 2026-09-15 by RFC-0003 § 4.**
The paragraph that stood here said, in full:

> `vpay_ledger::AccountKind` has exactly three variants — `MerchantPayable`,
> `PayerClearing`, `PlatformFeeRevenue` — with no per-merchant dimension.
> "Per merchant: `balance(merchant_payable) = …`" cannot actually be computed
> from that type as modelled: nothing says _which_ merchant a given
> `MerchantPayable` posting belongs to. Fixing this needs a new field on the
> Rust type (and the table that mirrors it), not a schema-only patch — adding
> a `merchant_id` column to the design sketch without a corresponding Rust
> field would be inventing structure the code doesn't have.

It was fixed in the order that paragraph demanded, and the order is the point.
`AccountKind::MerchantPayable { merchant_id: String }` is the new field;
migration `0045` is the table mirroring it; `model LedgerEntry` in
`schemas/vpay.cstack` declares the column only because the Rust field now
exists behind it.

**It is a variant payload, not a field on `Entry`**, because the two are
different claims. A field would admit a `payer_clearing` posting carrying a
merchant — meaningless, the clearing account is vpay's and pooled across
tenants — and a `merchant_payable` posting carrying none, which is the gap
reintroduced. `ledger_entries` mirrors the sum type with the CHECK
`ledger_entries_merchant_id_iff_merchant_payable`:
`(account = 'merchant_payable') = (merchant_id IS NOT NULL)`. That constraint
is multi-column, so `cratestack migrate baseline` is blind to it in both
directions, which is why `a_merchant_payable_entry_must_name_its_merchant` and
`a_pooled_account_entry_must_not_name_a_merchant` in `postgres_smoke.rs` write
both rows it refuses.

Invariant 2 is therefore computable for the first time, in two independent
implementations that the same test checks against each other:
`vpay_ledger::balance(entries, account, currency)` in memory and
`vpay_db::Ledger::merchant_payable_balance(merchant_id, currency_code)` in SQL.
**Computable is not the same as asserted nightly** — nothing schedules it, and
nothing posts to the ledger in the first place; see § Status.

## Status

**The refund `fee` posts nothing, and nothing posts it** — see the section
above. The column (`refunds.fee`, migration `0031`) and the wire field
(`vpay_api::model::RefundObject::fee`) exist and are asserted; no application
code writes a `refunds` row at all, and no adapter can produce a fee to write.

Invariant 1 is implemented and tested in `vpay-ledger`
(`a_capture_with_a_fee_balances`, `an_unbalanced_transaction_is_rejected`),
and is intentionally application-only — see above. The over-refund guard
(not one of the four numbered invariants above, but the other constraint
this doc covers) now has a real database CHECK in addition to `Money`'s
Rust-level guard — see "When refunds post" above.

**Persistence: the writer exists and nothing calls it (2026-09-15, RFC-0003
§ 4). Those are two facts and the second is the one that matters.** This
section said "persistence and invariants 2–4 are not started" until then, and
half of that has moved:

- `vpay_db::ledger::post_in_tx`, reached from outside `vpay-db` only through
  `TxRepositories::post_ledger_transaction_in_tx`, is the first statement in
  this repository's history to write `ledger_transactions` / `ledger_entries`.
  It runs inside the caller's transaction — there is deliberately no pooled
  variant, because a posting that committed apart from the settlement that
  caused it would be a ledger disagreeing with the charge.
- **`Transaction::validate()` is finally called by something that writes.** It
  runs before the first statement, and its failure is `DbError::Ledger` — an
  error the caller must handle, never an `expect`. That is what makes
  "invariant 1 stays application-enforced" a property of the write path rather
  than of a function nothing called; proven by
  `an_unbalanced_ledger_posting_is_refused_and_writes_nothing`, which asserts
  both the error **and** that neither table gained a row.
- Invariant 2 is computable — see the two implementations named above —
  and `two_merchants_payable_balances_do_not_mix_in_the_database` asserts it
  against a real Postgres for two merchants at once.

**What has NOT moved, and no reading of the above should suggest otherwise:**

- **No capture and no refund has ever produced a ledger row, in any
  deployment.** `Settlement::apply_succeeded` and
  `Settlement::apply_refund_succeeded` do not post. Every deployment's ledger
  is empty, and it will stay empty until those call sites are wired.
- Invariant 2 is computable, **not asserted nightly** — nothing schedules it.
- Invariants 3 and 4 are not started.
- Invariant 1's "per currency" clause is still not enforced by
  `Transaction::validate()`, which sums minor units across every leg whatever
  currency it is in. Neither `Transaction::capture` nor `Transaction::refund`
  can build a mixed-currency posting, so nothing reaching the database today
  can trip it; the gap is recorded on the function.
- **A ledger transaction has no minter and no bound.**
  `ledger_transactions.id` is supplied by the caller, `vpay_core::ids` has no
  `lt_`/`le_` prefix, and neither ledger table carries the
  `CHECK (char_length(id) BETWEEN 1 AND 64)` every other caller-named id
  column in `backends/migrations` has. `vpay_db::ledger::post_in_tx` derives
  each entry's id as `{transaction_id}_{index}`, which is deterministic —
  and, for two transactions whose ids differ only by a `_N` suffix, not
  unique; the primary key would refuse the second, loudly. Whether the id
  vocabulary grows a ledger prefix is a maintainer's call, recorded in
  migration `0045`'s header rather than decided by the branch that wrote the
  first writer.
- **Nothing checks that a posting's merchant is the merchant of the charge it
  is attributed to.** `ledger_entries.merchant_id` is denormalised from
  `ledger_transactions -> charges -> payment_intents.merchant_id` on purpose
  (a ledger must not change when an operational row does — migration `0045`
  § "Why a column at all"), and no SQL constraint can span three tables.
  Whoever assembles the `AccountKind::MerchantPayable` is the only guarantor,
  so the call sites that eventually post owe that a test.

Evidence:
[../status/verification/2026-09-15-ledger-merchant-dimension.md](../status/verification/2026-09-15-ledger-merchant-dimension.md),
and the row in [../status/backend.md](../status/backend.md) — see
[../status.md](../status.md).
