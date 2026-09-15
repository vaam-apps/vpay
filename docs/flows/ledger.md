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

  **That last paragraph stopped being true on 2026-09-15 (RFC-0003 section 3),
  and the half that changed is precisely the half about application code.**
  `vpay_db::Refunds::create` writes the `refunds` row and increments
  `amount_refund_pending` in **one transaction**, so there is now an
  application path that reaches the constraint. It is still not a
  `SELECT ... FOR UPDATE`: there is no application read of the balance at all,
  which is stronger rather than weaker, because the increment is an expression
  over the row's own column and the row lock is the one any `UPDATE` already
  takes. What remains exactly as described is the guarantee and its mechanism.

  The refusal is `vpay_db::DbError::OverRefund`, its own variant rather than
  the `DbError::Query` an unclassified CHECK violation becomes:
  `Category::Conflict`, `409`, never retried. `Query` is `Category::Storage`,
  which on a refund would tell a merchant to re-send a request that can never
  succeed. `two_concurrent_refunds_race_and_the_database_refuses_the_second`
  in `backends/tests/integration/tests/postgres_smoke.rs` issues two 3,000
  refunds against a 5,000 capture with `tokio::join!` and asserts exactly one
  commits, the kind of the refusal, and the stored figures afterwards. **The
  mutation it exists for**: delete the `amount_refund_pending` increment from
  `payment_intents::reserve_refund_in_tx` and both commit, 6,000 pending
  against a 5,000 capture — measured on 2026-09-15 and reverted
  ([../status/verification/2026-09-15-refunds-write-path.md](../status/verification/2026-09-15-refunds-write-path.md)).

  Separately, the only over-refund guard that exists in Rust today remains
  narrower and non-concurrent: `Money::checked_sub` in
  `backends/crates/vpay-core/src/money.rs` rejects an arithmetic result that
  would go negative (tested by `refunding_more_than_captured_is_rejected`),
  which stops a single refund larger than what remains captured but says
  nothing about two refunds racing each other.

  ~~**No ledger posting yet** — no capture and no refund has ever produced a
  `ledger_transactions` or `ledger_entries` row in any deployment.~~
  **Retired on 2026-09-15 (RFC-0003 sections 3-4).** That sentence had already
  been narrowed once, from "nothing in the application writes to the ledger
  tables today" to "a writer exists and nothing calls it"; two call sites call
  it now. `Settlement::apply_succeeded` posts the capture and
  `apply_refund_succeeded` posts the refund, each inside the transaction that
  settles the thing it records. What is true instead, and is narrower, is in
  section Status: a ledger row has still never been produced in any
  **deployment**, because no deployment has ever taken a payment.

- **On success:** in one transaction, decrement pending, increment refunded,
  write the ledger transaction. `vpay_db::Settlement::apply_refund_succeeded`
  does all three since 2026-09-15, plus the `invoices.amount_refunded` update
  it already did (issue #91's D5). Before that it did **only** the invoice
  half, so a refund moved the document and left the intent claiming the whole
  amount was still kept — which is why invariant 3 below had no code
  maintaining its left-hand side.
- **On failure:** decrement pending only. Nothing was posted, so **no reversal
  entry is needed** — which is why the reservation column exists rather than
  posting optimistically and unwinding.
  `vpay_db::Settlement::apply_refund_failed` is that path, and
  `a_failed_refund_releases_the_reservation_and_posts_nothing` asserts the
  ledger tables are untouched rather than holding a pair of legs that net to
  zero. A netting pair would satisfy invariants 1 and 2 and still be a ledger
  claiming money moved twice when it never moved at all.
- **On cancellation:** the same release, with the refund moving `pending` to
  `canceled` (`vpay_db::Refunds::cancel`). Nothing was posted, so nothing is
  reversed.

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
`a_capture_with_a_fee_balances`, `an_unbalanced_transaction_is_rejected` and
— for the "per currency" clause — `a_mixed_currency_transaction_does_not_balance`
and `each_currency_balances_on_its_own_book`. The `LedgerTransaction` model's
own `GAP` comment in `schemas/vpay.cstack` says the same thing.

**"Per currency" is part of the check, and became so on 2026-09-15 because
the first writer made the gap reachable.** `validate()` used to sum minor
units across every leg whatever currency it was in, so 100 XAF debited
against 100 EUR credited balanced. While nothing wrote the ledger that was a
statement about an unreachable function; the writer takes a `Transaction`
whose `entries` field is `pub`, so a hand-built mixed-currency posting reached
`ledger_entries` — measured, by running
`a_mixed_currency_posting_is_refused_and_writes_nothing` (then in
`postgres_smoke.rs`, now in `vpay_db::ledger`'s own test module) against the
code before the fix and watching it commit. It was reachable from *any*
consumer at the time, through a `pub` trait method that no longer exists; it
is reachable only from inside `vpay-db` now, which narrows who could make the
mistake and not whether the guard is needed. `validate()` now balances each
currency in `Currency::ALL` on its own book and `LedgerError::Unbalanced`
names the currency that is short. Nothing in the schema could have caught
this: `currency_code` is per row, and per the paragraph above invariant 1 is
not a database constraint, so this function is the only guard there is.

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

**Persistence: the writer exists and two settlement call sites call it
(2026-09-15, RFC-0003 sections 3-4).** This section said "persistence and
invariants 2-4 are not started" until 2026-09-15, then "the writer exists and
nothing calls it" for the half-day between wave 1 and wave 2. Both are
superseded:

- `vpay_db::ledger::post_in_tx` is the first statement in this repository's
  history to write `ledger_transactions` / `ledger_entries`.
  It runs inside the caller's transaction — there is deliberately no pooled
  variant, because a posting that committed apart from the settlement that
  caused it would be a ledger disagreeing with the charge.
- **It is `pub(crate)` and on no public trait.** It was briefly reachable
  from outside `vpay-db` through `TxRepositories::post_ledger_transaction_in_tx`,
  which let any consumer holding a `PendingTransaction` post an arbitrary
  balanced transaction against an arbitrary `charge_id` under an id of its
  choosing, with no settlement anywhere near it — while that trait's own doc
  claimed the opposite. The method is gone; the shape is
  `vpay_db::refunds::settle_in_tx`'s, and what a consumer can name is the
  business operation and never the raw double entry. The six cases that drive
  the raw writer moved into `vpay_db::ledger`'s own `#[cfg(test)]` module with
  it, because giving a test a public door is publishing a capability vpay does
  not have.
- **`Settlement::apply_succeeded` posts the capture and
  `apply_refund_succeeded` posts the refund**, each in the transaction that
  settles the thing it records. The capture posting is **two legs, not three**:
  no column in this schema holds a capture-time platform fee and no
  configuration computes one, so the call site passes `None` and
  `platform_fee_revenue` is never credited. That is a fact about this
  repository rather than a simplification, and
  `a_settled_charge_posts_its_capture_in_the_same_transaction` asserts the
  two-leg shape so that a fee model cannot land silently.
- **A ledger transaction id is minted.** `vpay_core::ids::ledger_transaction_id`
  (`lt_`) was added with migration `0046`'s
  `CHECK (char_length(id) BETWEEN 1 AND 64)` on both ledger tables — the two
  halves of the gap migration `0045`'s header recorded and left open.
- **`Transaction::validate()` is finally called by something that writes.** It
  runs before the first statement, and its failure is `DbError::Ledger` — an
  error the caller must handle, never an `expect`. That is what makes
  "invariant 1 stays application-enforced" a property of the write path rather
  than of a function nothing called; proven by
  `an_unbalanced_posting_is_refused_and_writes_nothing`, which asserts
  both the error **and** that neither table gained a row.
- Invariant 2 is computable — see the two implementations named above —
  and `two_merchants_payable_balances_do_not_mix` asserts it
  against a real Postgres for two merchants at once.
- **The caller's `transaction_id` is the idempotency key, and the duplicate
  it raises must not be swallowed inside the transaction that raised it.**
  `a_replayed_transaction_id_is_refused_and_adds_no_legs` pins the
  first half: a replay is a `UniqueViolation` on `ledger_transactions_pkey`
  and adds no legs, and a _different_ posting reusing a spent id is refused
  by the same key.
  `swallowing_a_duplicate_posting_inside_a_transaction_discards_the_whole_transaction`
  pins the trap in the second: Postgres aborts a
  transaction at the first failed statement and turns the following
  `COMMIT` into a `ROLLBACK` without raising, so a call site that catches the
  duplicate and commits anyway is handed `TxOutcome::Commit` while its
  charge, its event and its posting are all discarded. A settlement that may
  run twice has to abandon and re-read, or take a `SAVEPOINT` — not treat
  the violation as "already done, carry on". **It is a property of every write
  in `vpay-db`, not of the ledger**, which is why
  `swallowing_a_duplicate_write_inside_a_transaction_discards_the_whole_transaction`
  in `postgres_smoke.rs` pins the same thing for a duplicate `event_id`
  through the real `UnitOfWork` seam. The two posting call sites let the
  duplicate propagate; neither can raise one, because both mint a fresh id.

**Invariant 3 is maintained, and invariant 4 has one guard rather than two
(2026-09-15, RFC-0003 section 3).**

- Invariant 3 (`amount_refunded` = the sum of succeeded refunds for that
  intent) has code on both sides for the first time:
  `Settlement::apply_refund_succeeded` moves `amount_refunded` up and
  `amount_refund_pending` down in one statement, and
  `apply_refund_failed` / `Refunds::cancel` release the reservation without
  touching `amount_refunded`.
  `amount_refunded_is_the_sum_of_succeeded_refunds_after_two_partials`
  asserts the counter against a `SUM` over the rows, after two partials **and
  a failure** — the failure being what tells an implementation that adds every
  refund it sees apart from one that adds only the succeeded ones.
- Invariant 4 (one capture transaction per succeeded charge) is upheld by
  `apply_succeeded`'s compare-and-swap on the charge still being live, which
  is what stops a settlement running twice at all. It is **not** upheld by
  `ledger_transactions_pkey`: the transaction id is minted afresh, so the
  primary key cannot be a second, independent guard. A deterministic id, or a
  `kind` column and a partial unique index on `charge_id` (a bare
  `UNIQUE (charge_id)` is wrong — refunds post against the same charge), would
  add one. Which of those, if any, is a maintainer decision and is recorded in
  `vpay_core::ids::ledger_transaction_id`'s own doc rather than taken by the
  branch that wired the first call site.

**What has NOT moved, and no reading of the above should suggest otherwise:**

- **No ledger row has ever been produced in any deployment.** The call sites
  are wired and proven against a real Postgres 16 in CI; no deployment has ever
  taken a payment, so every deployment's ledger is empty for the same reason
  every deployment's `charges` table is.
- **No rail can execute a refund.** `ProviderAdapter::refund` is
  `NotImplemented` on both rails and `POST /v1/refunds` is unrouted until
  wave 3, so the write path above is reachable from tests and from nothing
  else. `../status.md` carries the gap.
- Invariant 2 is computable, **not asserted nightly** — nothing schedules it.
  Neither are 1, 3 or 4.
- **The refund posting emits no event.** `charge.refunded` and
  `charge.refund.updated` are documented types nothing emits; the wire object
  is `vpay-api`'s to shape and there is no caller until wave 3.
- **`{transaction_id}_{index}` is still not injective in general, and
  `post_in_tx` does not refuse an id that could collide.** `x` and `x_0` both
  derive `x_0_0`. No pair of minted `lt_` ids can be in that relation — every
  body is exactly 24 characters of an alphabet with no `_`, asserted by
  `vpay_core::ids::tests::two_minted_ledger_ids_cannot_derive_the_same_entry_id`
  — and both call sites mint, so it is unreachable from the write path rather
  than closed at the writer. An in-crate caller that hand-built the colliding
  pair would be refused by `ledger_entries_pkey`, loudly, never silently.
  Closing it at `post_in_tx` needs a shape check there and a `DbError` variant
  to carry the refusal; left open deliberately and recorded here.
- **Nothing in the schema checks that a posting's `merchant_id` is the merchant
  the charge belongs to, and nothing can.** `ledger_entries.merchant_id` is a
  bare `TEXT` with a length CHECK and the pair CHECK; the merchant is three
  tables away (`ledger_entries -> ledger_transactions -> charges ->
  payment_intents.merchant_id`) and a row-level CHECK sees one row. The
  denormalisation is deliberate — a ledger must not change when an operational
  row does (migration `0045` section "Why a column at all").

  **This is a call-site obligation, and since 2026-09-15 it is discharged
  rather than merely stated.** `settlement::post_capture` and
  `settlement::post_refund` build the `AccountKind::MerchantPayable` from the
  `merchant_id` of the intent row *that same transaction* wrote, and from
  nothing a caller passed — `apply_succeeded` and `apply_refund_succeeded`
  take no merchant argument at all, so threading one through would be a change
  to those signatures rather than a value someone could quietly pass.
  `a_posting_is_attributed_to_the_intents_own_merchant_and_not_to_a_caller`
  captures and refunds for two merchants in one database and joins **every**
  `merchant_payable` row back through `ledger_transactions -> charges ->
  payment_intents`, asserting no row disagrees. The balances alone would not
  catch a consistent mix-up; the join is what does.

Evidence:
[../status/verification/2026-09-15-ledger-merchant-dimension.md](../status/verification/2026-09-15-ledger-merchant-dimension.md),
and the row in [../status/backend.md](../status/backend.md) — see
[../status.md](../status.md).
