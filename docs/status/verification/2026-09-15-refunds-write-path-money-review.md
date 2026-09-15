# 2026-09-15 — adversarial money/concurrency review of the refunds write path

Branch `review/w2c-money`, off `refunds/w2-writepath` at `36a3a5b8`. Lens:
money correctness and concurrency only. A second review covered conventions
and blast radius; nothing below is a conventions finding.

Companion to
[2026-09-15-refunds-write-path.md](2026-09-15-refunds-write-path.md), which is
the implementation's own evidence page. Where the two disagree, this page says
so explicitly rather than quietly superseding it.

## What was claimed, and what measurement said

| Claim under review                                                                                            | Verdict                                                                              |
| ------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| The `no_over_refund` CHECK is the only over-refund guard; there is no read-then-compare in Rust               | **TRUE**                                                                             |
| Deleting the `amount_refund_pending` increment makes both concurrent 3 000 refunds commit (2 vs 1)            | **TRUE**, reproduced exactly                                                         |
| The capture and refund postings match `docs/flows/ledger.md` § Postings leg by leg                            | **TRUE**                                                                             |
| Invariant 2 holds per merchant after a capture and two partial refunds                                        | **TRUE**                                                                             |
| Merchant attribution is discharged at the call site, and the join test would catch a caller-supplied merchant | **TRUE**, and the count assertion beside it is load-bearing too                      |
| No `UniqueViolation` is swallowed; both postings mint fresh ids                                               | **TRUE**                                                                             |
| The duplicate-`event_id` case proves the `TxOutcome::Commit`-with-nothing-written half                        | **TRUE**                                                                             |
| Invariant 4's one guard holds against a retried settlement                                                    | **TRUE**, and it was untested under concurrency — now pinned                         |
| `refunded_at_most_paid` is "a backstop the write path cannot reach"                                           | **TRUE** — but the narrowing that followed from it dropped a second, unrelated claim |

## The finding: an atomicity claim went out with the provocation

`two_refunds_against_one_invoice_add_up_and_an_over_refund_is_refused` used to
end on a rider:

> the refund flip is INSIDE the transaction the invoice write aborted; a refund
> recorded as settled against an invoice that could not be updated is a
> permanent inconsistency nothing in vpay would ever notice

That sentence was the **only** assertion anywhere on this branch that
`Settlement::apply_refund_succeeded` is one transaction. Moving the over-refund
guard forward to `Refunds::create` removed the thing that provoked the failure,
and the rider left with it. The replacement — a raw-SQL `UPDATE invoices` that
proves migration `0042`'s CHECK still fires — is a correct and worthwhile
assertion about the **constraint**, and it says nothing at all about the
**transaction**.

That is the wrong direction for this change to have moved, because the same
change made that transaction two statements longer. `apply_refund_succeeded`
now flips the refund, moves `amount_refunded` and `amount_refund_pending` on
the intent, touches the invoice, and posts two ledger legs. A partial commit of
that sequence is a refund a merchant is told succeeded with nothing recording
the money leaving, or a ledger permanently short one refund with invariant 2
wrong for that merchant forever — which is exactly what
`vpay_db::ledger`'s own module doc promises cannot happen ("a posting that
committed apart from the settlement that caused it would be a ledger
disagreeing with the charge").

**Measured.** With `apply_refund_succeeded` split into two commits — the refund
flip and the intent counters committed, then a fresh transaction for the invoice
and the posting — **every refund and ledger case on the branch still passes**:

| Suite                                          | Under the split-transaction mutation |
| ---------------------------------------------- | ------------------------------------ |
| `vpay-db` `-E 'test(/refund/)'` (as delivered) | 11 passed, 0 failed                  |
| `postgres_smoke` refund/ledger/capture cases   | 14 passed, 0 failed                  |

`a_refund_settlement_that_cannot_finish_rolls_back_the_flip_the_counters_and_the_posting`
restores the claim. It provokes the invoice backstop deliberately — with a raw
`UPDATE invoices SET amount_refunded = amount_paid`, a state the write path
cannot produce, which the case says in its own doc — and then asserts, each on
its own because a partial commit satisfies any single one:

- the refund is still `pending`,
- both intent counters are exactly where they were,
- no refund posting reached `ledger_transactions`,
- `merchant_payable_balance` is still the whole capture,
- and a retry, once the document is repaired, settles it.

Under the split-transaction mutation it fails on the first of those
(`left: "succeeded"`, `right: "pending"`). Reverted.

## Why the backstop really is unreachable, since the narrowing turns on it

The invoice ceiling is `amount_refunded <= amount_paid`;
`mark_paid_for_intent_in_tx` sets `amount_paid = amount_due`. The intent
ceiling is `amount_refunded + amount_refund_pending <= amount`, and
`vpay_api::v1::invoices::intent_for` mints the intent with
`amount = invoice.amount_remaining`, which on an `open` invoice is
`amount_due`. The two ceilings are therefore the **same number**, and the
intent's is evaluated one statement earlier. `attach_intent` does not check the
two amounts against each other, so this is a property of how the intent is
minted rather than a constraint — worth writing down, because a future
`pay`-time change to `intent_for` would make the backstop reachable and the
raw-SQL assertion would not notice.

## Invariant 4 under concurrency, which nothing tested

`a_settled_charge_posts_its_capture_in_the_same_transaction` settles twice **in
sequence**. Production produces the other shape: a poll job and a rail callback
reaching `apply_succeeded` for one charge at the same moment, neither able to
see the other's uncommitted write. `docs/flows/ledger.md` § Status states that
the charge compare-and-swap is the only guard there is, because the transaction
id is minted and `ledger_transactions_pkey` cannot be a second one.

`two_settlements_racing_one_charge_post_exactly_one_capture` is that case. The
two `event_id`s differ deliberately, so `events_pkey` cannot stand in for the
charge guard. It passes as delivered: one `Ok(Some(_))`, one `Ok(None)`, one
ledger transaction, two legs, `merchant_payable_balance` 5 000, one event.

**A correction to the branch's own wording.** `docs/flows/ledger.md` says
invariant 4 "has one guard rather than two". Measured, there are two things
standing between this schema and a second capture posting, and the second is
worth knowing about: widening `apply_succeeded`'s `WHERE id = $1 AND state IN
(…)` to `WHERE id = $1` does **not** produce a double posting — the loser is
refused by `payment_intents::succeed_after_submission`'s `status IN
(SETTLEABLE_STATUSES)`, which does not contain `succeeded`, and the whole
transaction aborts. What the widening destroys is the _answer shape_: the loser
gets `DbError::WriteMatchedNoRow` instead of `Ok(None)`, which a worker retries
forever. The charge CAS is what makes the settlement **idempotent**; the intent
guard is what makes it **safe**. Both new cases assert the first, which is the
one that can silently degrade.

## Mutations run, with their numbers

| #   | Mutation                                                                   | Expected | Measured                                                                                                                                                                                                                                                                                           |
| --- | -------------------------------------------------------------------------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `reserve_refund_in_tx`: increment → `SET updated_at = now()`               | FAIL     | FAIL. `two_concurrent_refunds_race_and_the_database_refuses_the_second`: `left: 2, right: 1` — the reported numbers. 7 of 13 `postgres_smoke` cases and 5 of 11 `vpay-db` refund cases failed with it.                                                                                             |
| 2   | `post_capture`: `AccountKind` built from a literal `"merchant_2"`          | FAIL     | FAIL. The join reports `[("lt_…_1", "merchant_2", "merchant_1")]`.                                                                                                                                                                                                                                 |
| 3   | `apply_succeeded`: skip `post_capture` entirely                            | FAIL     | FAIL on the count assertion (`left: 1, right: 3`) — it is not decoration; it is what stops the join passing vacuously.                                                                                                                                                                             |
| 4   | `apply_succeeded`: charge CAS widened to `WHERE id = $1`                   | FAIL     | FAIL, both invariant-4 cases. See the correction above for what actually broke.                                                                                                                                                                                                                    |
| 5   | `apply_refund_succeeded`: swallow the invoice error (`.unwrap_or(None)`)   | FAIL     | **PASS** — absorbed by the next statement. Postgres has already aborted, so `post_refund`'s INSERT raises `25P02` and the settlement still fails closed. Recorded because it is the trap amendment item 4 names, and here it happens to be caught by the statement ordering rather than by a test. |
| 6   | `apply_refund_succeeded`: split into two commits after the intent counters | FAIL     | **PASS on every case the branch shipped.** The finding above.                                                                                                                                                                                                                                      |

## Gates run

`just ci` was **not** run — the brief forbids it. CI is the gate.

| Command                                                                                      | Result                           |
| -------------------------------------------------------------------------------------------- | -------------------------------- |
| `cargo nextest run -p vpay-db -p vpay-ledger -p vpay-core`                                   | 328 passed, 0 skipped, 0 ignored |
| `cargo nextest run -p vpay-tests-integration --test postgres_smoke`                          | 52 passed, 0 skipped, 0 ignored  |
| `cargo clippy -p vpay-db -p vpay-tests-integration --all-targets --all-features -D warnings` | clean                            |
| `cargo +nightly fmt --all`                                                                   | clean                            |

**Postgres tests ran; none skipped.** Every count is a real
`postgres:16-alpine` container over
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, and nextest reports
`0 skipped` on both runs. The counts are 328 and 52 against the branch's 241 +
51 because `vpay-ledger`/`vpay-core` are in the same invocation here and
because this review added two cases.

**What is NOT verified here, loudly.**
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` passed —
and it passed against **cratestack 0.11.1**, which is what this host's shared
`~/.cargo/bin` carries, while the repository pins **0.12.0**. The test printed
its own warning and ran the measurement in full, reporting
`drift detected in 25 table(s)/view(s) (194 change(s) total)` — the same 194
the branch pins. That agreement is between two runs of the _same_ off-pin
grammar and is therefore no evidence at all about the pinned one. Installing
0.12.0 was refused deliberately: `~/.cargo/bin` is shared with other agents on
this host. **CI is the arbiter of `EXPECTED_DRIFT_CHANGES`, and if CI disagrees
the constant is what moves.**

## One defect outside the money path, fixed because it was in the diff

`charges::id_for_intent_in_tx` arrived with no blank line between its doc
comment and `insert_for_intent`'s, so the two comments merged:
`insert_for_intent` — the write that opens the single charge for an intent —
was left with **no documentation at all**, and this read's rustdoc opened with
another function's `# Errors` section and then carried a second one. Two
references in it were also wrong (`crate::refunds::create_in_tx` does not
exist; it is `post_refund`, not `apply_refund_succeeded`, that names the charge
id). Separated, corrected, and the `fetch_optional`-is-safe argument written
down: `one_charge_per_intent` is a **full** unique index, so the read can match
at most one row.

## Files

- `backends/crates/vpay-db/tests/repositories.rs` — the restored atomicity case
  and the `intent_refund_figures` reader.
- `backends/tests/integration/tests/postgres_smoke.rs` — the invariant-4
  concurrency case.
- `backends/crates/vpay-db/src/charges.rs` — the doc comments separated.
