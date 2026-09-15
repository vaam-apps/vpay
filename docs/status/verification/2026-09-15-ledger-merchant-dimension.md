# 2026-09-15 — the ledger gets a merchant dimension and a writer

RFC-0003 § 4, wave 1. Branch `refunds/w1-ledger`, off
`claude/orange-refund-transaction-2b95e5` at `c7f06382`.

## What this page is evidence for, and what it is not

It is evidence that `vpay_ledger::AccountKind` now carries the per-merchant
dimension invariant 2 needs, that `ledger_entries` mirrors it, and that a
writer exists and is proven against a real Postgres 16.

**It is not evidence that vpay keeps a ledger.** Nothing calls the writer.
`Settlement::apply_succeeded` and `Settlement::apply_refund_succeeded` do not
post, so every deployment's `ledger_transactions` and `ledger_entries` are
empty and no capture or refund has ever produced a row. That wiring is a
separate piece of work and was deliberately out of scope here.

## Gates run

`just ci` was **not** run — the brief for this work forbade it (five
concurrent local builds have OOM-killed this host). CI is the gate. What was
run, and its exact output:

| Command                                                               | Result                                |
| --------------------------------------------------------------------- | ------------------------------------- |
| `cargo nextest run -p vpay-ledger -p vpay-db`                         | 244 passed, 0 skipped, 0 ignored      |
| `cargo test --doc -p vpay-ledger -p vpay-db`                          | 6 + 8 passed, 0 ignored               |
| `cargo clippy -p vpay-ledger -p vpay-db --all-targets -- -D warnings` | clean                                 |
| `cargo xtask verify-migrations`                                       | ok — 45 files match `MANIFEST.sha256` |

The `vpay-db` half of that 244 is container-backed: `DOCKER_HOST=unix:///run/user/1000/docker.sock`
was set and every case ran against a real `postgres:16-alpine`. **None
skipped** — a suite that skips on a missing Docker socket still prints `ok`,
which is why the count and the zero are both recorded here.

Additionally, outside the two crates the brief named, because the tests it
asked for live in a third package:

| Command                                                                                               | Result                                         |
| ----------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `cargo nextest run -p vpay-tests-integration -E 'binary(postgres_smoke) and …'` (the six cases below) | 6 passed                                       |
| the same, `test(drift)`                                                                               | 1 passed, after `EXPECTED_DRIFT_CHANGES` moved |

## The six real-Postgres cases

All in `backends/tests/integration/tests/postgres_smoke.rs`. The review below
added four more in the same file, taking `postgres_smoke` from 44 to 48.

- `schema_migrates_cleanly_on_an_empty_database` — 45 migrations applied
  (44 → 45).
- `a_balanced_ledger_posting_satisfies_invariant_1_in_the_database` — the
  shipping writer, through the real `UnitOfWork`, records a 5 000 XAF capture
  with a 100 XAF fee; `SUM(debit) = SUM(credit) = 5 000` read back **out of
  Postgres**, and the three legs are the ones `docs/flows/ledger.md`
  § Postings tabulates.
- `an_unbalanced_ledger_posting_is_refused_and_writes_nothing` — a lopsided
  posting comes back as `DbError::Ledger(LedgerError::Unbalanced { currency:
Xaf, debits: 5_000, credits: 4_900 })` **and** both tables are still empty.
  Two assertions because neither implies the other: a writer that inserted
  the parent row and then noticed would satisfy the first. (The `currency`
  field was added by the review below; the case read `{ debits, credits }`
  when this section was first written.)
- `two_merchants_payable_balances_do_not_mix_in_the_database` — invariant 2,
  for two merchants at once. See the mutation below.
- `a_merchant_payable_entry_must_name_its_merchant` and
  `a_pooled_account_entry_must_not_name_a_merchant` — both rows
  `ledger_entries_merchant_id_iff_merchant_payable` refuses, written by hand
  because the writer **cannot produce either**: `AccountKind` makes both
  unrepresentable, so a case driven through it would pass whether or not the
  CHECK existed.

## The mutation, run in both the form the brief asked for and a sharper one

**As briefed — delete migration 0045's merchant column.** `merchant_id` and
its two constraints and its index were commented out of the migration and
`two_merchants_payable_balances_do_not_mix_in_the_database` re-run:

```
FAIL two_merchants_payable_balances_do_not_mix_in_the_database
  Error: every one of these postings balances and must commit
  Caused by: column "merchant_id" of relation "ledger_entries" does not exist
```

It fails. **But it fails at the INSERT, not at a number**, which is weaker
evidence than it looks: it says the column is load-bearing, not that the
assertion discriminates on the dimension. So a second, sharper mutation was
run.

**Sharper — keep the column, neutralise the tenant predicate.**
`merchant_payable_balance`'s `AND merchant_id = $1` was replaced by
`AND ($1 = $1)`:

```
FAIL two_merchants_payable_balances_do_not_mix_in_the_database
  assertion `left == right` failed: merchant 1's balance is its own capture
  net of fee, less its own refund
    left: 22500
   right: 2900
```

22 500 is both merchants' postings summed together, which is exactly what a
balance blind to the merchant dimension would answer. The same mutation in
Rust — `vpay_ledger::balance`'s `entry.account == *account` replaced by a
`std::mem::discriminant` comparison — fails the unit test
`two_merchants_payable_balances_do_not_mix` with the same two numbers.

Both mutations were reverted and `git status` is clean; the tree these
numbers were taken on is the tree that was pushed.

## Drift: 190 → 192, measured — on cratestack **0.11.1**, not the pinned 0.12.0

**Read the caveat before the number.** The host had `cratestack 0.11.1` on
`PATH`; this repository pins `0.12.0`, and `EXPECTED_DRIFT_CHANGES`' previous
value was measured at 0.12.0. The drift test only _warns_ about a mismatch, so
it is easy to read past. Both numbers below were therefore taken with the
**same** binary: with migration `0045` withdrawn and
`model LedgerEntry.merchant_id` commented out, 0.11.1 reported
`drift detected in 25 table(s)/view(s) (190 change(s) total)` — exactly the
0.12.0-measured constant — and with both restored, 192. So the delta is +2
under one binary, and the two releases agree on the absolute number for this
schema. Installing 0.12.0 was declined deliberately: it would have written
into a `~/.cargo/bin` shared with other agents on this host. **CI runs 0.12.0;
if it reports anything other than 192, CI is the evidence and this constant is
what moves.**

`cratestack migrate baseline --strict` against a freshly migrated Postgres 16:
`drift detected in 25 table(s)/view(s) (192 change(s) total)`, 19 unmappable
columns. `EXPECTED_DRIFTED_RELATIONS` (25) and `EXPECTED_UNMAPPABLE_COLUMNS`
(19) did not move. `ledger_entries` goes 6 → 8:

```
[safe] CHECK `ledger_entries_merchant_id_length` exists in the live database but is not declared
[safe] index `ledger_entries_merchant_payable_idx` exists in the live database but is not declared
```

The `merchant_id` **column** costs zero, because `model LedgerEntry` declares
it in the same commit and the migration gave it no DB `DEFAULT`; the pair
CHECK costs zero because it is multi-column and introspection filters
`array_length(c.conkey, 1) = 1`. The full account is on
`EXPECTED_DRIFT_CHANGES`' own doc comment. See also
[status/cratestack.md](../cratestack.md).

## What was NOT done

- **No call site.** The postings are not wired into
  `Settlement::apply_succeeded` or `apply_refund_succeeded`. Deliberate split.
- **No refund fee posting.** Issue #46's decision stands, unchanged by
  RFC-0003 § 4: the fee is reported on the object and posted to no account.
- **No route, no adapter, no `POST /v1/refunds`.**
- ~~**Invariant 1 is still not per currency.**~~ **Closed by the review
  below on the same day.** The bullet said the gap was pre-existing and
  unreachable, and the first half was right. The second was not: the writer
  this page documents is a `pub` method taking a `Transaction` whose
  `entries` field is `pub`, and a hand-built mixed-currency posting was
  measured committing to Postgres. See "Adversarial review" below.
- **Invariants 3 and 4** (`amount_refunded` equals the sum of succeeded
  refunds; every succeeded charge has exactly one capture transaction) remain
  unstarted.

## Adversarial review — correctness and the money invariants (2026-09-15)

Branch `review/w1-ledger-invariants`, off `refunds/w1-ledger` at `7b75503b`.
Lens: the money invariants only; schema/migration blast radius and conventions
were a separate reviewer's.

### Claims re-run, and what they came back as

| Claim on this page                                                                                        | Verdict                                             | Measured                                                                                                                                                                                                                                                   |
| --------------------------------------------------------------------------------------------------------- | --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo nextest run -p vpay-ledger -p vpay-db` → 244 passed, 0 skipped, 0 ignored                          | **TRUE**                                            | `244 tests run: 244 passed, 0 skipped` in 224 s, `DOCKER_HOST` set. Run without it first, deliberately: `vpay-db` **fails** rather than skipping (`config_reconcile … failed to create a container`), so this suite cannot print a green it has not earned |
| `postgres_smoke` → 44 passed, 0 skipped                                                                   | **TRUE**                                            | `44 tests run: 44 passed, 0 skipped` in 84 s                                                                                                                                                                                                               |
| the sharper mutation, `AND merchant_id = $1` → `AND ($1 = $1)`, fails on a number: left 22500, right 2900 | **TRUE, reproduced exactly**                        | `left: 22500 / right: 2900` at `postgres_smoke.rs:3020`                                                                                                                                                                                                    |
| `ledger_entries_merchant_id_iff_merchant_payable` fires in both directions                                | **TRUE, and mutation-proven**                       | With the `ADD CONSTRAINT` commented out of `0045`, **both** cases fail, and fail on the INSERT _succeeding_: `PgQueryResult { rows_affected: 1 }` for the `merchant_payable`-with-no-merchant row and for both pooled-account-with-a-merchant rows         |
| the currency half of `merchant_payable_balance` is tested                                                 | **TRUE** (not claimed on this page; checked anyway) | `AND currency_code = $2` → `AND ($2 = $2)` fails `two_merchants_payable_balances_do_not_mix_in_the_database` on the EUR assertion: `left: 2900 / right: 0`                                                                                                 |
| "nothing that reaches the database today can trip" the per-currency gap                                   | **FALSE**                                           | see below                                                                                                                                                                                                                                                  |

The drift constant (192) was **not** independently re-verified: this host still
has `cratestack 0.11.1` and installing `0.12.0` into the shared `~/.cargo/bin`
was declined for the same reason the implementer declined it. CI is the
evidence for that number.

### The per-currency gap was reachable, and is fixed

`a_mixed_currency_ledger_posting_is_refused_and_writes_nothing` was written
first and run against the unfixed tree. It did not merely fail to raise — the
posting **committed**:

```
panicked at postgres_smoke.rs: a franc is not a euro and this posting
balances in neither: Commit(())
```

100 XAF debited and 100 EUR credited went into `ledger_entries` through the
shipping writer, through the real `UnitOfWork`. The disclosure on this page
was half right: the gap is pre-existing in `validate()`, but "unreachable" was
a property of there being **no writer**, and this branch is the writer. The
two constructors cannot build such a posting; nothing requires a caller to use
them, `Transaction::entries` is `pub`, and the branch's own
`an_unbalanced_ledger_posting_is_refused_and_writes_nothing` hand-builds a
`Transaction` — so hand-building is the established idiom at exactly this
seam. The database cannot catch it either: `currency_code` is per row and
invariant 1 is deliberately not a constraint.

`Transaction::validate()` now iterates `Currency::ALL` and balances each
currency's book on its own; `LedgerError::Unbalanced` gained a `currency`
field, because the error pages and "debits 100, credits 0" does not say which
book is short. Four new `vpay-ledger` cases and one real-Postgres case pin it,
including `each_currency_balances_on_its_own_book` — a transaction carrying
two currencies that both balance is still valid, so the fix rejects mismatched
legs and not the presence of a second currency.

### Idempotency of `post_in_tx` — attacked, and one trap found

`a_replayed_ledger_transaction_id_is_refused_and_adds_no_legs` confirms the
module docs: a replay is `UniqueViolation { constraint:
"ledger_transactions_pkey" }`, a _different_ posting reusing a spent id is
refused by the same key, and the capture's three legs
(`ltx_replay_0/_1/_2`) are all that is in the table afterwards.
`entry_ids_do_not_collide_between_transactions_whose_ids_share_a_prefix` posts
`ltx_p`, `ltx_p_0` and `ltx_p_0_0` and gets six distinct entry ids —
`{transaction_id}_{index}` is injective because the index carries no
underscore, so the last `_` always splits an entry id back into its
transaction.

The trap is what a caller does with the violation.
`swallowing_a_duplicate_posting_inside_a_transaction_discards_the_whole_transaction`
posts, posts the same id again, catches the `UniqueViolation` the way an
"already posted, carry on" branch would, and commits. `UnitOfWork::transaction`
returns `TxOutcome::Commit(())` **and `ledger_entries` is empty** — Postgres
aborted the transaction at the failed statement and turned the `COMMIT` into a
`ROLLBACK` without raising. This is a property of every write in `vpay-db`,
not of this writer, but it is load-bearing for the arm that wires
`Settlement::apply_succeeded`, and it is now pinned beside the posting instead
of waiting to be rediscovered in production.

### Left open, deliberately

**Nothing ties `ledger_entries.merchant_id` to the charge's merchant.** The
column has a length CHECK and the pair CHECK and no foreign key; there is no
relation to `ledger_transactions -> charges -> payment_intents.merchant_id`.
A posting crediting `merchant_2` against `merchant_1`'s charge is storable,
and invariant 2 would then answer confidently and wrongly for both merchants.
It is a call-site obligation today. Closing it in the schema means either
denormalising the merchant onto `ledger_transactions` or adding a trigger, and
which is right is a maintainer decision, so it is recorded in
`docs/flows/ledger.md` § Status rather than picked here.

### Gates run by the review

| Command                                                                                                                    | Result                                                                 |
| -------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| `cargo nextest run -p vpay-ledger -p vpay-db`                                                                              | 247 passed, 0 skipped, 0 ignored (244 + three new `vpay-ledger` cases) |
| `cargo nextest run -p vpay-tests-integration -E 'binary(postgres_smoke)'`                                                  | 48 passed, 0 skipped                                                   |
| `cargo test --doc -p vpay-ledger -p vpay-db`                                                                               | 6 + 8 passed, 0 ignored                                                |
| `cargo clippy -p vpay-ledger -p vpay-db -p vpay-api -p vpay-worker -p vpay-tests-integration --all-targets -- -D warnings` | clean                                                                  |
| `cargo +nightly fmt --all --check`                                                                                         | clean                                                                  |

`just ci` was **not** run; the brief forbade it and CI is the gate.
