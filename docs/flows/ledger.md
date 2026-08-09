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

- **On creation:** increment `amount_refund_pending` under a row lock. The
  over-refund CHECK rejects the second of two concurrent over-refunds *at the
  database level* — this must not be an application read-then-check, which
  races. **No ledger posting yet.**
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

## Status

Invariant 1 is implemented and tested in `vpay-ledger`. **Persistence and
invariants 2–4 are not started** — see [../STATUS.md](../STATUS.md).
