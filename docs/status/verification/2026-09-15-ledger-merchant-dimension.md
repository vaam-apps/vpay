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

| Command | Result |
| --- | --- |
| `cargo nextest run -p vpay-ledger -p vpay-db` | 244 passed, 0 skipped, 0 ignored |
| `cargo test --doc -p vpay-ledger -p vpay-db` | 6 + 8 passed, 0 ignored |
| `cargo clippy -p vpay-ledger -p vpay-db --all-targets -- -D warnings` | clean |
| `cargo xtask verify-migrations` | ok — 45 files match `MANIFEST.sha256` |

The `vpay-db` half of that 244 is container-backed: `DOCKER_HOST=unix:///run/user/1000/docker.sock`
was set and every case ran against a real `postgres:16-alpine`. **None
skipped** — a suite that skips on a missing Docker socket still prints `ok`,
which is why the count and the zero are both recorded here.

Additionally, outside the two crates the brief named, because the tests it
asked for live in a third package:

| Command | Result |
| --- | --- |
| `cargo nextest run -p vpay-tests-integration -E 'binary(postgres_smoke) and …'` (the six cases below) | 6 passed |
| the same, `test(drift)` | 1 passed, after `EXPECTED_DRIFT_CHANGES` moved |

## The six real-Postgres cases

All in `backends/tests/integration/tests/postgres_smoke.rs`.

- `schema_migrates_cleanly_on_an_empty_database` — 45 migrations applied
  (44 → 45).
- `a_balanced_ledger_posting_satisfies_invariant_1_in_the_database` — the
  shipping writer, through the real `UnitOfWork`, records a 5 000 XAF capture
  with a 100 XAF fee; `SUM(debit) = SUM(credit) = 5 000` read back **out of
  Postgres**, and the three legs are the ones `docs/flows/ledger.md`
  § Postings tabulates.
- `an_unbalanced_ledger_posting_is_refused_and_writes_nothing` — a lopsided
  posting comes back as `DbError::Ledger(LedgerError::Unbalanced { debits:
  5_000, credits: 4_900 })` **and** both tables are still empty. Two
  assertions because neither implies the other: a writer that inserted the
  parent row and then noticed would satisfy the first.
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
value was measured at 0.12.0. The drift test only *warns* about a mismatch, so
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
- **Invariant 1 is still not per currency.** `Transaction::validate()` sums
  minor units across every leg whatever currency it is in — a pre-existing gap,
  now written down on the function rather than left implicit. Neither
  `Transaction::capture` nor `Transaction::refund` can produce a mixed-currency
  posting, so nothing that reaches the database today can trip it, and it was
  left alone rather than changed under a brief that did not ask for it.
- **Invariants 3 and 4** (`amount_refunded` equals the sum of succeeded
  refunds; every succeeded charge has exactly one capture transaction) remain
  unstarted.
