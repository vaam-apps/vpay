# RFC-0001: Settlement and payouts

- **Status:** Draft
- **Date:** 2026-08-08

## Problem

v1 is **pass-through**: each merchant brings their own merchant account on each
rail, and funds move payer → merchant's own wallet. vpay never holds merchant
money, which keeps it a technical service provider rather than a payment
institution.

Merchants will eventually ask for aggregation — one vpay account collecting, with
periodic settlement to them. That is money transmission.

## Proposal

Not yet. This RFC exists to record why the schema is *ready* for it while the
product is not:

- The ledger is already double-entry with a `platform_fee_revenue` account, so
  aggregation needs no new accounting model.
- `merchant_payable` is already credit-normal, so a settlement run is a debit
  against an existing balance.

## The blocker is not technical

In CEMAC, aggregation puts vpay under BEAC/COBAC payment-services regulation:
licence or a sponsoring bank/EMI, capital requirements, a compliance function.
Multi-month.

**Do not start the code before the licence conversation.** Almost every gateway
that dies in year one dies here.

## Open questions

1. Licence directly, or ride a sponsoring bank?
2. Settlement cadence and currency.
3. Does any target rail *force* aggregation by not supporting pass-through
   settlement? (Open for Orange — see the adapter doc.)
