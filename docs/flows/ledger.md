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
  over-refunds. **Correction:** an earlier version of this doc claimed a
  database CHECK constraint rejects the second of two concurrent over-refunds
  "at the database level." That was false, and there is neither a database
  schema nor the row lock itself in this repo yet — `amount_refund_pending`
  is not persisted anywhere. Even once `schemas/vpay.cstack` is wired in, a
  plain CHECK could not provide this guarantee under any grammar: "the second
  of two concurrent writes loses" is row-lock-and-recheck behaviour, not a
  constraint a single row's CHECK can express, and CrateStack's own
  `@db_enforce` only promotes a single-field `@range`/`@length`/`@iso4217`
  validator in any case — there is no `@@check(expr)` for a cross-column or
  cross-row rule. The only over-refund guard that exists in Rust today is
  narrower and non-concurrent: `Money::checked_sub` in
  `backends/crates/vpay-core/src/money.rs` rejects an arithmetic result that
  would go negative (tested by `refunding_more_than_captured_is_rejected`),
  which stops a single refund larger than what remains captured but says
  nothing about two refunds racing each other. The row lock described above
  — and therefore the actual concurrency guarantee — is **not implemented**.
  **No ledger posting yet.**
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

**Invariant 2 has a modelling gap, surfaced while writing the design-sketch
schema in `schemas/vpay.cstack`.** `vpay_ledger::AccountKind` has exactly
three variants — `MerchantPayable`, `PayerClearing`, `PlatformFeeRevenue` —
with no per-merchant dimension. "Per merchant: `balance(merchant_payable) = …`"
cannot actually be computed from that type as modelled: nothing says *which*
merchant a given `MerchantPayable` posting belongs to. Fixing this needs a new
field on the Rust type (and the table that mirrors it), not a schema-only
patch — adding a `merchant_id` column to the design sketch without a
corresponding Rust field would be inventing structure the code doesn't have.

## Status

Invariant 1 is implemented and tested in `vpay-ledger`
(`a_capture_with_a_fee_balances`, `an_unbalanced_transaction_is_rejected`).
**Persistence and invariants 2–4 are not started**, and invariant 2 additionally
cannot be computed from the current `AccountKind` type as noted above — see
[../status.md](../status.md).
