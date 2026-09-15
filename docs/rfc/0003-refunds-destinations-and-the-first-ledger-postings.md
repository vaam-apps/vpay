# RFC-0003: Refunds, refund destinations, and the first ledger postings

- **Status:** Draft
- **Author:** vpay maintainers
- **Date:** 2026-09-15
- **Related:** [RFC-0001](0001-settlement-and-payouts.md),
  [issue #46](https://github.com/vaam-apps/vpay/issues/46),
  [issue #47](https://github.com/vaam-apps/vpay/issues/47),
  [issue #159](https://github.com/vaam-apps/vpay/issues/159),
  [docs/flows/ledger.md](../flows/ledger.md)

## Problem

vpay cannot return money. Three separate gaps sit behind that one sentence, and
they are usually collapsed into each other:

1. **No route.** `POST /v1/refunds` is declared in
   [docs/flows/merchant-auth/resource-contract.md](../flows/merchant-auth/resource-contract.md)
   and mounted nowhere. Only `GET /v1/refunds/{id}` is served (issue #45).
2. **No writer.** Nothing in this repository inserts a `refunds` row. One
   statement updates them — `refunds::settle_in_tx`, reached through
   `Settlement::apply_refund_succeeded` — and it can only ever act on a row an
   operator or a test put there by hand.
3. **No rail.** `mtn_momo::refund` is `ProviderError::NotImplemented`; MTN
   refunds are the Disbursements product and no deployment holds those
   credentials. `orange_money` does not override the port at all and inherits
   `ProviderError::Unsupported`.

Two further facts shape the proposal and are easy to miss:

- **Nothing writes the ledger either — not for refunds, and not for captures.**
  `ledger_transactions` and `ledger_entries` are named in exactly three places
  in the workspace: the CrateStack drift list in `vpay-db/src/schema.rs`, a
  comment string in `config_reconcile.rs`, and `postgres_smoke.rs`. The
  `vpay-ledger` crate is linked into `vpay-api` and `vpay-worker` only so that
  `LedgerError` has a slot in the error classification tree;
  `Transaction::validate()` is never called by anything that moves money.
  [migrations/0005](../../backends/migrations/0005_create-ledger.sql) says so in
  its own header.
- **`Settlement::apply_refund_succeeded` does not touch the intent.** It flips
  the refund row and calls `invoices::add_refund_for_intent_in_tx`, so the
  *invoice's* `amount_refunded` moves and the *intent's* `amount_refunded` /
  `amount_refund_pending` do not. [docs/flows/ledger.md](../flows/ledger.md)
  § "When refunds post" requires both.

## Proposal

### 1. A rail-agnostic refund destination

Some rails return money to its origin with no destination (a card, a wallet).
Mobile-money rails do not: returning money is an outbound transfer and needs a
payee. vpay will carry more upstreams than mobile money, so the destination
must be agnostic in the core and specific only at the rail.

**Wire shape**, mirroring the confirm path's existing
`payment_method_data[<rail_code>][msisdn]` convention:

```
POST /v1/refunds
  payment_intent=pi_...
  amount=2000                          # omit for a full refund
  reason=requested_by_customer
  destination[mtn_momo][msisdn]=+237...
  metadata[order_id]=1234
```

**Requiredness is a capability, never a provider check** (ADR-0002 forbids
`if provider == "…"` in the core). `vpay_provider::Capabilities` gains:

```rust
/// Where a refund on this rail sends money.
pub enum RefundDestination {
    /// Back to the instrument that paid. The core refuses a `destination`.
    Origin,
    /// An explicit payee. The core refuses a refund without one.
    Required,
}
```

`mtn_momo` and `orange_money` both declare `Required`. A future card rail
declares `Origin` and the merchant sends nothing. No handler learns a rail code.

**The port signature** gains one parameter:

```rust
async fn refund(
    &self,
    charge: &ChargeRef,
    amount: Money,
    destination: Option<&RefundTarget>,
    config: &ProviderConfig,
) -> Result<Refunded, ProviderError>;
```

`RefundTarget` is opaque to the core in the same way `RefExtra` is: the core
validates that one is present when the capability demands it, and the adapter
interprets the interior.

**Where a `Required` rail also supports account-holder lookup**, the core
name-matches the destination through `account_holder_name` before calling
`refund` — the caller [issue #47](https://github.com/vaam-apps/vpay/issues/47)
built that lookup for. `mtn_momo` supports it; `orange_money` does not
(`supports_account_holder_lookup: false`), so on Orange the destination is
accepted unverified and that fact is recorded in `docs/status.md`.

### 2. The four endpoints

`GET /v1/refunds/{id}` is already served. Adding, Stripe-shaped:

| Method | Path | Notes |
| --- | --- | --- |
| `POST` | `/v1/refunds` | create; `payment_intent`, `amount`, `reason`, `destination[…]`, `metadata[…]` |
| `POST` | `/v1/refunds/{id}` | update — `metadata` only, as Stripe |
| `GET` | `/v1/refunds` | list, scoped to the merchant, `payment_intent` filter |
| `POST` | `/v1/refunds/{id}/cancel` | cancel a refund still `pending` |

Every one is registered in the `V1Route` constant so
`every_registered_v1_path_answers_401_without_a_token` walks it, per issue
#159's standing rule that a route absent from the constant does not exist.

### 3. The write path

Creation, in one database transaction:

1. Resolve the intent, check tenancy, check `destination` against the rail's
   `RefundDestination`.
2. Insert the `refunds` row with a caller-minted `re_…` id and a
   `provider_reference_id` generated **before** any rail call
   ([crash-safety.md](../flows/crash-safety.md)).
3. `UPDATE payment_intents SET amount_refund_pending = amount_refund_pending + $n`
   — the `no_over_refund` CHECK from migration `0003` is what refuses an
   over-refund, under concurrency, and it must be this UPDATE that trips it.

Then the rail call, then settlement. `Settlement::apply_refund_succeeded` is
extended to decrement `amount_refund_pending` and increment
`amount_refunded` on the intent, which it does not do today. Failure
decrements pending only — nothing was posted, so there is nothing to reverse.

Idempotency reuses `vpay-db`'s existing `idempotency` store, as the confirm
path does.

### 4. The ledger becomes real

This is the first code in the repository's history to write
`ledger_transactions` / `ledger_entries`, and it cannot be done for refunds
alone: a ledger holding refunds and no captures is worse than an empty one,
because `balance(merchant_payable)` would go negative and invariant 2 would
read as violated on every row.

**In scope, therefore:**

- **Capture postings** in `Settlement::apply_succeeded`, in the transaction
  that already settles the charge.
- **Refund postings** in `Settlement::apply_refund_succeeded`, likewise.
- **Closing the `AccountKind` gap.** `MerchantPayable` has no per-merchant
  dimension, so invariant 2 (`balance(merchant_payable) = Σ captures − Σ fees
  − Σ refunds`) is *uncomputable* as modelled — [ledger.md](../flows/ledger.md)
  and `0005`'s own `GAP` comment both say so. The Rust type gains the
  dimension and `ledger_entries` gains the column that mirrors it. This is a
  change to `vpay_ledger`, not a schema-only patch.
- **The balancing invariant stays application-enforced**, in
  `Transaction::validate()`, which the settlement path must now actually call.
  Invariant 1 is an aggregate over sibling rows and no row-level CHECK can
  express it; [ledger.md](../flows/ledger.md) commits to not faking it with a
  trigger.

**Postings**, unchanged from [ledger.md](../flows/ledger.md):

| Event | Account | Direction |
| --- | --- | --- |
| capture | `payer_clearing` | debit |
| | `merchant_payable` | credit |
| | `platform_fee_revenue` | credit (fee, when any) |
| refund | `merchant_payable` | debit |
| | `payer_clearing` | credit |

**The refund fee still posts nothing.** Issue #46's decision stands: the fee is
reported on the object and posted to no account. Posting it would require
deciding who bears it, which is a marketplace judgement vpay does not own, and
would add a second kind of term to invariant 2. Unchanged by this RFC.

### 5. The rails

**MTN — build it.** Refunds are the Disbursements product: a separate
subscription key, a separately-scoped token, and a `transfer` call. The
adapter is written against MTN's published Disbursements API and proven against
WireMock exactly as `submit` was, plus the `ProviderConfig` and
`config/application.yml` keys to carry a disbursement credential.

**It will not have been called.** No deployment holds a Disbursements
subscription key, so `mtn_momo::refund` will be WireMock-proven and
rail-unproven, and `docs/status.md` must say precisely that — the same posture
`submit` held until 2026-09-15.

**Orange — the model changes, the wire calls do not exist.** Per the
maintainer's decision of 2026-09-15, an Orange refund *is* a transfer back, so
Orange's refusal stops being a fact about the rail and becomes work vpay owes:

- `supports_refunds: false` → `true`
- the adapter overrides `refund` with its own
  `ProviderError::NotImplemented("orange_money::refund")` token instead of
  inheriting `Unsupported`
- that token is declared in `docs/status.md`, where `verify-status` tracks it

**No Orange transfer wire call is written by this RFC.** There is no Orange
transfer API documented in this repository — not even reconstructed. The
existing adapter was built from Orange Developer's public overview and several
community SDKs that agree with each other; for transfers no such source exists
here, so an endpoint path and payload would be invented. That is the failure
mode [CLAUDE.md](../../CLAUDE.md) names first, in the money path. It stays a
declared token until item 5 of
[adapter-orange-money.md](../flows/adapter-orange-money.md)'s "To confirm with
Orange Cameroun" list has an answer.

### 6. Events, webhooks and SDKs

`charge.refunded` and `charge.refund.updated` are documented event types that
nothing has ever emitted. Creation, settlement, failure and cancellation emit
them, rendered through the single `RefundObject` renderer that
`GET /v1/refunds/{id}` already uses. Both merchant SDKs gain the four methods
and their parity rows, or a dated gap — `verify-sdk-parity` fails either way.

## Alternatives considered

**`metadata.payee_number`.** The original sketch, and rejected on privacy
grounds rather than taste. [docs/api/README.md](../api/README.md) commits, for
customer erasure, that "every identifier on it becomes the literal
`[redacted]`, your `metadata` is untouched" — metadata is merchant-opaque by
design and the erasure path deliberately does not reach into it. A payer's
phone number stored there therefore survives an erasure request, sits in the
`events` table, and is already inside every signed webhook delivered, which
walks straight into open issues #145 and #147. It is also unvalidated (a
misspelled key means *no destination*, silently), has no anchor for the
`account_holders` name match, and lands in an object with a ten-key tripwire
test.

**A `destination` on the intent rather than the refund.** Rejected: the payee
is a property of *this* refund, and partial refunds may legitimately go to
different destinations.

**Refund-only ledger postings.** Rejected above: negative
`merchant_payable` balances and a permanently violated invariant 2.

## Open questions

1. **Does a refund destination have to match the payer?** On MTN the confirm
   carries the payer's MSISDN, so vpay *could* refuse a destination that is not
   it. On Orange the payer's number is never learned (`payer_ref: None` on a
   redirect rail), so the same rule cannot be enforced there. Enforcing it on
   one rail and not the other is a real asymmetry the merchant would see.
2. **Retention of the destination MSISDN.** The customer object settled on
   12 months. A refund destination is the same class of data and has no policy
   yet.
3. **Does `RefundTarget` need a non-MSISDN shape before a non-mobile-money
   upstream exists?** Designing for a bank account now risks modelling an
   upstream nobody has chosen.

## Impact on existing invariants

| Invariant | Effect |
| --- | --- |
| `no_over_refund` CHECK (migration `0003`) | First code path that can reach it; it becomes load-bearing rather than test-only |
| ledger invariant 1 (per transaction, debits = credits) | First enforcement in a live path; `Transaction::validate()` starts being called |
| ledger invariant 2 (per merchant) | Becomes *computable* for the first time, once `AccountKind` carries the merchant dimension |
| ledger invariant 3 (`amount_refunded` = Σ succeeded refunds) | First code that maintains the left-hand side on the intent |
| ledger invariant 4 (one capture transaction per succeeded charge) | First code that creates one |
| `partial_refunds_imply_refunds` CHECK (migration `0002`) | Orange flipping to `supports_refunds: true` must not flip `supports_partial_refunds` without intent |
| `verify-status` | Gains `orange_money::refund`; `mtn_momo::refund` is retired from the list |
