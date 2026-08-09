# Ledger

Double-entry. Convention: `balance(account) = SUM(credit) - SUM(debit)`.
`merchant_payable` is credit-normal — a positive balance is money the merchant
received.

## Postings

**Capture of 5,000 XAF, no fee**

| Account | Direction | Amount |
|---|---|---|
| `payer_clearing` | debit | 5000 |
| `merchant_payable` | credit | 5000 |

**Capture with a 100 XAF platform fee**

| Account | Direction | Amount |
|---|---|---|
| `payer_clearing` | debit | 5000 |
| `merchant_payable` | credit | 4900 |
| `platform_fee_revenue` | credit | 100 |

**Refund of 2,000 XAF** (fee not refunded — Stripe's default)

| Account | Direction | Amount |
|---|---|---|
| `merchant_payable` | debit | 2000 |
| `payer_clearing` | credit | 2000 |

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
  *committed* row can ever end up over-refunded: two concurrent `UPDATE`s
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
  nothing in the application writes to `payment_intents` or the ledger
  tables today; only the integration test does, directly, to prove the
  constraint fires.
- **On success:** in one transaction, decrement pending, increment refunded,
  write the ledger transaction.
- **On failure:** decrement pending only. Nothing was posted, so **no reversal
  entry is needed** — which is why the reservation column exists rather than
  posting optimistically and unwinding.

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

**Invariant 2 has a modelling gap, surfaced while writing the design-sketch
schema in `schemas/vpay.cstack`.** `vpay_ledger::AccountKind` has exactly
three variants — `MerchantPayable`, `PayerClearing`, `PlatformFeeRevenue` —
with no per-merchant dimension. "Per merchant: `balance(merchant_payable) = …`"
cannot actually be computed from that type as modelled: nothing says *which*
merchant a given `MerchantPayable` posting belongs to. Fixing this needs a new
field on the Rust type (and the table that mirrors it), not a schema-only
patch — adding a `merchant_id` column to the design sketch without a
corresponding Rust field would be inventing structure the code doesn't have.
This is unchanged by the migrations landing: `ledger_transactions` and
`ledger_entries` exist in `backends/migrations/0005_create-ledger.sql`
mirroring the same three-variant `AccountKind`, so the gap is now present in
real SQL too, not just the design sketch.

## Status

Invariant 1 is implemented and tested in `vpay-ledger`
(`a_capture_with_a_fee_balances`, `an_unbalanced_transaction_is_rejected`),
and is intentionally application-only — see above. The over-refund guard
(not one of the four numbered invariants above, but the other constraint
this doc covers) now has a real database CHECK in addition to `Money`'s
Rust-level guard — see "When refunds post" above. **Persistence and
invariants 2–4 are not started**, and invariant 2 additionally cannot be
computed from the current `AccountKind` type as noted above — see
[../status.md](../status.md).
