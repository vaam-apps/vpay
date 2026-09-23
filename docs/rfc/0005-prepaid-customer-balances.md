# RFC-0005: Prepaid customer balances

- **Status:** Draft
- **Author:** vpay maintainers
- **Date:** 2026-09-23
- **Related:** [RFC-0001](0001-settlement-and-payouts.md) (pass-through, and
  why aggregation is a licence question),
  [RFC-0003](0003-refunds-destinations-and-the-first-ledger-postings.md)
  (refunds and the first ledger postings),
  [RFC-0004](0004-billing-on-top-of-invoices.md) (subscriptions, invoices),
  [docs/flows/ledger.md](../flows/ledger.md). The gap list this RFC answers
  was drawn up against Lago's `wallets` API (v1.53.0,
  <https://swagger.getlago.com/>) on 2026-09-23.

Nothing in this document is built. vpay has no per-customer balance, no
stored-value concept anywhere, and no ledger row has yet been written in any
deployment.

## Problem

On mobile money a subscription cannot collect on its own. Every payment is a
payer approving a prompt or visiting a page ([RFC-0004](0004-billing-on-top-of-invoices.md)
§ 2), so every period costs the payer an action at a moment vpay chooses.

A **prepaid balance** changes when that action happens: the payer tops up when
it suits them, and renewals draw the balance down with nobody present. On this
market it may be the only honest way to get "automatic" renewals without a
card. Lago calls this a wallet; Stripe calls it a customer balance.

## The blocker is not technical

This is RFC-0001's warning again, and it binds harder here. **A balance a
payer has paid into and can spend later is stored value.** In CEMAC, issuing
electronic money is regulated by BEAC/COBAC. Being on the wrong side of that
line is a licence problem, not a bug.

This RFC proposes only the narrowest form, and asks counsel whether even that
is outside the regulation:

| Property                       | Closed-loop merchant credit (proposed)                      | Open-loop stored value (not proposed) |
| ------------------------------ | ----------------------------------------------------------- | ------------------------------------- |
| Where the money is             | The **merchant's own** rail account, collected pass-through | An account vpay or an issuer controls |
| What it can pay for            | That merchant's invoices only                               | Anything, anyone                      |
| Transferable between customers | No                                                          | Yes                                   |
| Redeemable for cash            | Only as a refund of an unspent top-up, through RFC-0003     | Yes                                   |
| Who owes it                    | The merchant owes the customer goods or services            | The issuer owes the holder money      |

**Do not start the code before counsel confirms that closed-loop merchant
credit, held pass-through, needs no licence for the merchant or for vpay.**
Whether an unspent balance must be refundable in cash on request (consumer
protection), and whether unclaimed balances may expire, are part of the same
conversation.

## Proposal

### 1. Shape: Stripe's customer balance, not Lago's wallets

- A customer has a `balance` per currency (XAF in practice). The Stripe
  sign convention applies: **negative is credit the customer holds**, positive
  is an amount they owe that the next invoice will collect.
- Every movement is a `customer_balance_transaction` (`cbtxn_…`) with
  `amount`, `currency`, `type` (`top_up`, `applied_to_invoice`,
  `top_up_refunded`, `adjustment`), `invoice` or `top_up`, `ending_balance`,
  `description` and `created`. The rows are append-only, so the balance is
  their sum and is never set.
- Routes, Stripe's spellings: `GET /v1/customers/{id}/balance_transactions`,
  `GET …/balance_transactions/{id}`, and `POST …/balance_transactions` for a
  merchant `adjustment` (a goodwill credit, for instance). An adjustment is a
  merchant's statement with no money behind it, and the object says so with
  `type=adjustment`.
- Lago's multiple wallets per customer, credit-to-currency rates and priority
  ordering are **not** proposed. One balance per currency is what an invoice
  can be netted against without an allocation policy.

### 2. Top-ups: `balance_top_up` (`btu_…`)

Stripe has no customer-facing top-up object, so this one is vpay-native, and
it is the only one in this RFC.

- `POST /v1/customers/{id}/balance_top_ups` with `amount`, `currency`,
  `success_url`, `cancel_url`. It mints an ordinary payment intent and an
  ordinary hosted checkout session, exactly as `POST /v1/invoices/{id}/pay`
  does, and answers the top-up with its `url`.
- **The balance moves in the settlement transaction**, beside `flip_invoice`.
  One transaction takes the charge to `succeeded`, writes the `top_up`
  balance transaction, and emits the event. A second write afterwards would
  leave money collected and not credited, and a balance has no poller that
  would ever notice. This is the same argument `invoices.md` makes for
  `invoice.paid`.
- A failed or cancelled intent credits nothing.
- **Automatic top-up is not available on mobile money**, for the same reason a
  subscription cannot collect there. On a rail with `supports_off_session`
  (RFC-0004 § 7), "top up by N when the balance falls below M" becomes
  possible. It is a later step, not v1.

### 3. Applying the balance to an invoice

- **At finalize,** credit is applied to `amount_due` up to the whole amount,
  inside the finalize transaction and under the number lock the finalize
  already holds.
- The debit is `balance = balance + $n WHERE balance + $n <= 0` — an
  expression over the row, the technique `amount_refunded` uses. It is backed
  by `CHECK (balance <= 0)`, because v1 proposes credit only: Stripe's
  positive balance (an amount owed, collected by the next invoice) is not
  proposed. Two invoices finalizing concurrently cannot both spend the same
  credit; the second applies what is left.
- The invoice gains Stripe's `starting_balance`, `ending_balance` and
  `applied_balance`:
  - **Fully covered:** `amount_remaining = 0`, and the invoice goes straight
    to `paid` in the finalize transaction with no intent. That is a third
    writer of `paid` (settlement, out-of-band, balance), and `invoice.paid`
    gains a third writer with it.
  - **Partly covered:** `amount_remaining` is the difference, and `pay` mints
    the intent for that. `paid_means_nothing_remaining` still holds: the
    invoice is `paid` only when the rest has settled.
- **This is a partial payment in all but name,** and `invoices.md` lists
  partial payments as out of scope. The case for the exception is that the
  balance is applied once, at finalize, before any intent exists. The
  one-intent-at-a-time rule is untouched. (Open question 3.)
- A merchant may opt a subscription or an invoice out
  (`apply_customer_balance=false`).

### 4. Ledger

This depends on RFC-0003 § 4 making the ledger real, and it adds one account:

- **`customer_credit{merchant_id, customer_id}`**, credit-normal. It is a
  liability the **merchant** holds to their customer, recorded by vpay as
  bookkeeping, not money vpay holds.

| Event                         | Debit                   | Credit                                 |
| ----------------------------- | ----------------------- | -------------------------------------- |
| Top-up settles                | `payer_clearing`        | `customer_credit{m, c}`                |
| Balance applied to an invoice | `customer_credit{m, c}` | `merchant_payable{m}`                  |
| Unspent top-up refunded       | `customer_credit{m, c}` | `payer_clearing` (RFC-0003's reversal) |
| Merchant adjustment (credit)  | `merchant_payable{m}`   | `customer_credit{m, c}`                |

The customer's balance is then a ledger read, and the balance-transaction rows
are its projection. One of the two must be the source of truth; the proposal
is **the ledger**, with the rows written in the same transaction, and a
postgres-smoke case asserting that they agree.

### 5. Refunds, erasure, expiry

- **Refunding a top-up** is an RFC-0003 refund against its intent, limited to
  the unspent credit. `refunded_at_most_paid`'s pattern supplies the guard
  (`refund <= remaining credit from this top-up`), so it needs per-top-up
  remaining-credit accounting: first-in-first-out consumption, recorded on the
  `applied_to_invoice` rows.
- **Erasure:** a customer holding credit cannot be erased or anonymised. Both
  `DELETE /v1/customers/{id}` and the idle sweep refuse while the balance is
  non-zero, because the balance is money the merchant owes a person. What
  happens to an abandoned balance is a legal question (open question 2), and
  the sweep does not decide it.
- **Expiry:** not proposed until counsel answers.

### 6. Events

Stripe has no dedicated customer-balance webhook; a balance change surfaces as
`customer.updated`, which vpay already emits (migration `0039`). A top-up's own
lifecycle is its payment intent's events. So **no new event type is proposed**,
and a merchant learns about a top-up through `payment_intent.succeeded` (its
`metadata` names the top-up) and `customer.updated`.

A low-balance alert (Lago's wallet alerts) has no Stripe type, and vpay adds
only Stripe types. It is therefore **not** proposed as an event. A merchant
reads the balance.

## Alternatives considered

- **Lago's wallet model** (several wallets, credit units at a rate,
  priorities). More expressive, but it needs an allocation policy at every
  finalize, and its credit units add a second number to reconcile against
  money.
- **Stripe Billing credit grants.** Designed for promotional credits that
  expire, not for money a payer paid in, and a closer fit to § 1's
  `adjustment` than to a top-up.
- **A separate balance service.** The balance must be debited inside the
  invoice finalize transaction, and a debit across a service boundary is a
  distributed transaction. It belongs in vpay's database.
- **Open-loop stored value.** Rejected: it is e-money issuance and needs a
  licence.

## Open questions

1. **Is closed-loop merchant credit, held pass-through, outside BEAC/COBAC
   e-money regulation, for the merchant and for vpay?** Nothing else in this
   RFC proceeds without an answer. (Counsel.)
2. **Must an unspent balance be refundable in cash on request, and may it
   expire?** (Counsel.)
3. **Partial application:** accept the balance covering part of an invoice,
   or only whole invoices? Whole-only keeps "paid means paid in full"
   literal, but a customer with 4,000 FCFA of credit and a 5,000 bill would
   see none of it used. (Maintainer.)
4. **Ledger or rows as the source of truth** for the balance (§ 4 proposes the
   ledger). (Maintainer.)
5. **Does a top-up need a document** (a receipt through RFC-0004 § 10)?
   (Accountant.)

## Impact on existing invariants

- **Pass-through holds** only if open question 1 says it does. That is the
  point of asking first.
- **`paid` gains a third writer:** finalize, when the balance covers the
  invoice.
- **"Partial payments are out of scope"** gains one exception, the balance
  applied at finalize, if open question 3 accepts it.
- **The ledger gains** `customer_credit`, which is its first account keyed by
  customer.
- **Customer erasure gains a blocker:** a non-zero balance.
- **The settlement transaction gains a hook** beside `flip_invoice`.
- **No new event types.**
