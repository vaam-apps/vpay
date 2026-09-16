# 2026-09-15 — the refunds write path, and the first ledger postings a call site ever made

RFC-0003 §§ 3 and 4, wave 2. Branch `refunds/w2-writepath`, off
`claude/orange-refund-transaction-2b95e5` at `be55ff6c`.

## What this page is evidence for, and what it is not

It is evidence that `vpay_db` can now create, cancel, settle and fail a refund
against a real Postgres 16; that the over-refund guard is the `UPDATE` and
fires under concurrency; and that `Settlement::apply_succeeded` and
`Settlement::apply_refund_succeeded` post to the ledger inside the
transactions that settle a charge and a refund.

**It is not evidence that vpay can refund anything.**
`ProviderAdapter::refund` is `NotImplemented` on both rails (RFC-0003 § 5) and
`POST /v1/refunds` is unrouted until wave 3, so nothing in a shipping binary
calls any of it. No deployment has produced a ledger row, because no
deployment has ever taken a payment. No refund event is emitted —
`charge.refunded` and `charge.refund.updated` remain documented types nothing
writes.

## Gates run

`just ci` was **not** run — the brief for this work forbade it (five
concurrent local builds have OOM-killed this host). CI is the gate. What was
run, and its exact output:

| Command                                                                         | Result                           |
| ------------------------------------------------------------------------------- | -------------------------------- |
| `cargo nextest run -p vpay-core -p vpay-ledger`                                 | 86 passed, 0 skipped, 0 ignored  |
| `cargo nextest run -p vpay-db`                                                  | 241 passed, 0 skipped, 0 ignored |
| `cargo nextest run -p vpay-tests-integration -E 'binary(postgres_smoke)'`       | 51 passed, 0 skipped, 0 ignored  |
| `cargo test --doc -p vpay-core -p vpay-ledger -p vpay-db`                       | 57 + 6 + 8 passed, 0 ignored     |
| `cargo clippy -p vpay-core -p vpay-ledger -p vpay-db --all-targets -D warnings` | clean                            |
| `cargo clippy -p vpay-tests-integration --all-targets -D warnings`              | clean                            |
| `cargo build --workspace --all-targets`                                         | clean                            |
| `cargo +nightly fmt --all --check`                                              | clean                            |

**Postgres tests ran; none skipped.** Every count above is a real container
(`postgres:16-alpine`, `DOCKER_HOST=unix:///run/user/1000/docker.sock`), and
nextest reports `0 skipped` on all three runs. The `vpay-db` run includes the
seven ledger cases that moved into that crate — see below — and takes ~204 s
because each starts its own container.

## The decisive mutation, run and reverted

The brief named one. It was run on 2026-09-15 and the result is recorded here
whichever way it went:

> Remove the `amount_refund_pending` increment from step 2 and re-run the
> concurrent over-refund test. It must FAIL.

**It failed.** With
`SET amount_refund_pending = amount_refund_pending + $3` replaced by
`SET updated_at = now()` in `vpay_db::payment_intents::reserve_refund_in_tx`
— the rest of the statement, the guard and the merchant scope untouched —
`two_concurrent_refunds_race_and_the_database_refuses_the_second` failed with:

```
assertion `left == right` failed: exactly one of two concurrent 3 000 refunds
against a 5 000 capture may commit; got [Ok(Some(RefundRow { id: "re_race_a",
… amount: 3000 … })), Ok(Some(RefundRow { id: "re_race_b", … amount: 3000 … }))]
  left: 2
 right: 1
```

Both refunds committed: 6 000 pending against a 5 000 capture, which is
exactly the state `no_over_refund` exists to make unreachable. The mutation
was reverted and the test re-run — `PASS [14.763s]`, and `git status` clean
against the commit it was taken from.

That is the whole argument for the test: it is reaching the constraint, and
the constraint is what refuses the second writer.

## What moved, and the two amendments that changed the shape of it

### The raw ledger write came off the public trait

`TxRepositories::post_ledger_transaction_in_tx` is gone.
`vpay_db::ledger::post_in_tx` is `pub(crate)` and on no public trait, which is
`vpay_db::refunds::settle_in_tx`'s shape exactly: what a consumer of the crate
can name is the business operation and never the raw double entry. The method
had let any caller holding a `PendingTransaction` post an arbitrary balanced
transaction against an arbitrary `charge_id` under an id of its choosing, with
no settlement anywhere near it.

**Seven tests moved with it** (this page said six until the conventions review
counted them), out of `postgres_smoke.rs` and into
`vpay_db::ledger`'s own `#[cfg(test)]` module: the balanced capture read back
out of Postgres, the unbalanced posting, the mixed-currency posting, the two
merchants' balances, the replayed transaction id, the prefix-shared entry ids,
and the swallowed duplicate. Every one of them needs the raw writer — their
subject is what it refuses or what it derives, and neither settlement call
site can be made to build a bad posting. The precedent for putting a container
test inside the crate is `config_reconcile.rs`'s
`a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx`, which
states the rule: adding a public method purely to give a test a door is
publishing a capability vpay does not have.

Nothing outside `vpay-db` broke. The only consumers were those tests.

**One test was added rather than moved.**
`swallowing_a_duplicate_write_inside_a_transaction_discards_the_whole_transaction`
in `postgres_smoke.rs` pins the same trap for a duplicate `event_id` through
the real `UnitOfWork` seam, because the trap is a property of every write in
`vpay-db` and not of the ledger — which is the claim the ledger-only version
could no longer make from outside the crate.

### The merchant attribution is discharged at the call site

Migration `0045` and `docs/flows/ledger.md` § Status both record that nothing
constrains `ledger_entries.merchant_id` to agree with the merchant of the
charge its transaction names, and that no SQL constraint can: the fact is
three tables away and a row-level CHECK sees one row.

`settlement::post_capture` and `settlement::post_refund` build the
`AccountKind::MerchantPayable` from the `merchant_id` of the intent row _the
same transaction_ wrote.`apply_succeeded` and `apply_refund_succeeded` take no
merchant argument at all, so threading a caller's merchant through would be a
change to those signatures rather than a value someone could quietly pass.

`a_posting_is_attributed_to_the_intents_own_merchant_and_not_to_a_caller`
captures and refunds for two merchants in one database and joins **every**
`merchant_payable` row back through `ledger_transactions -> charges ->
payment_intents`, asserting that no row disagrees — plus a count assertion, so
an empty ledger cannot satisfy it vacuously. The balances alone would not
catch a consistent mix-up; the join is what does.

### Ledger ids are minted, and both tables are bounded

`vpay_core::ids::ledger_transaction_id` (`lt_`) joins the id vocabulary, and
migration `0046` adds `CHECK (char_length(id) BETWEEN 1 AND 64)` to
`ledger_transactions` and `ledger_entries` — the two halves of the gap
migration `0045`'s own header named and left open. Both settlement call sites
mint through it.

**Added by the conventions review:** the two CHECKs were pinned only by
`EXPECTED_DRIFT_CHANGES` going 192 → 194, which is a count and is satisfied by
any two single-column CHECKs anywhere in the schema.
`an_over_long_ledger_id_is_refused_by_the_database` in `postgres_smoke.rs` now
proves each one fires, by name — a 65-character id into each table, 64
admitted so the bound is inclusive, and the 66-character entry id a hand-built
64-character transaction id would derive, which is the case `0046`'s header
predicted in prose. Measured to fail with `ALTER TABLE ledger_transactions
DROP CONSTRAINT id_length` applied first (`rows_affected: 1`).

## What was left open, deliberately, and is not claimed as done

- **RETRACTED by the conventions review, 2026-09-15: the
  `{transaction_id}_{index}` ambiguity does not exist.** This entry said the
  derivation was "not injective in general", that `x` and `x_0` both derive
  `x_0_0`, and that closing it needed a shape check in `post_in_tx` plus a new
  `DbError` variant. All three are wrong. `x` derives `x_0, x_1, …` and `x_0`
  derives `x_0_0, x_0_1, …`; the derivation is injective for **every** pair of
  transaction ids, because the index is a `usize` rendered in decimal and
  decimal contains no `_`, so the last `_` of an entry id recovers exactly one
  pair. The claim was inherited from RFC-0003 § 4's amendment and repeated
  into five files without being tested. It is now tested:
  `vpay_db::ledger::tests::the_entry_id_derivation_is_injective` — no
  container, 340 adversarial ids × 13 indices, and it fails naming a colliding
  pair if the separator is removed — and
  `entry_ids_do_not_collide_between_ids_that_share_a_prefix` against the real
  `ledger_entries_pkey`. Migration `0046`'s header still carries the false
  sentence; migrations here are forward-only, so `vpay_db::ledger::entry_id`'s
  doc and `docs/flows/ledger.md` § Status are the correction of record.
- **No `le_` prefix was added.** Entry ids are _derived_, not minted, and a
  vocabulary entry with no minter behind it would be a claim about code nobody
  has written. The review confirms this: with the derivation injective, a
  minter for entry ids would buy nothing at all.
- **Found by the conventions review and left for the maintainer: a refund
  posting's currency comes from the intent, while `refunds.currency_code` is a
  column of its own that nothing ties to it.** `settlement::post_refund` calls
  `money_from_row(refund.amount, &intent.currency_code, "payment_intents")`
  because `SettledRefund` carries `id`, `payment_intent_id` and `amount` and no
  currency. The two agree today by construction — `Refunds::create` reads the
  currency off the intent it has locked and that is the only writer — and
  nothing in the schema enforces it: migration `0017` gives
  `refunds.currency_code` a foreign key to `currencies` and no relation to the
  intent's. So a second writer, or a hand-written `INSERT`, could produce a
  refund whose object renders one currency while its ledger legs are posted in
  another, and `Transaction::validate` would not notice: it balances each
  currency on its own book, and a posting whose every leg is in the same
  _wrong_ currency balances perfectly. The fix is small — add `currency_code`
  to `SettledRefund` and read it off the row being settled, so the posting's
  currency comes from the refund rather than from a row that merely ought to
  agree — but it changes a money-path type and is one statement's worth of
  behaviour, so the review reports it rather than taking it. This is the only
  place left on the path where "unreachable from today's one call site" is
  doing load-bearing work.
- **Invariant 4 has one guard, not two.** `apply_succeeded`'s compare-and-swap
  on the charge still being live is what stops a second capture transaction; a
  randomly minted id cannot make `ledger_transactions_pkey` a second,
  independent guard. A deterministic id, or a `kind` column and a partial
  unique index on `charge_id`, would add one — recorded in
  `ledger_transaction_id`'s doc as a maintainer decision rather than taken here.
- **Nothing schedules the nightly assertion of any invariant.**

## `EXPECTED_DRIFT_CHANGES`: 192 → 194, measured off-pin

Measured against a freshly migrated Postgres 16 with `cratestack migrate
baseline --strict`, which reported `drift detected in 25 table(s)/view(s) (194
change(s) total)`. `EXPECTED_DRIFTED_RELATIONS` is unmoved at 25 — both ledger
tables were already on the list.

**The measurement was taken on `cratestack-cli` 0.11.1 and this repository
pins 0.12.0.** The host has 0.11.1 on `PATH`, and `cargo install`ing 0.12.0
into a shared `~/.cargo/bin` while other work is running on this machine was
refused. That is the same limitation the 190 → 192 measurement records, and the
same mitigation applies: the delta was taken with a single binary. **If CI
(which has 0.12.0) disagrees, CI is the evidence and the constant is what
moves.**

## Two other constants moved, and both are audits rather than counters

- `vpay_db::sql_audit`'s `EXPECTED_ASSERT_SITES`: **61 → 66.** Five new
  `AssertSqlSafe` sites, three on `payment_intents` and two on `refunds`. The
  line-by-line audit is in
  [../../reference/vpay-db/dynamic-sql.md](../../reference/vpay-db/dynamic-sql.md);
  `refunds::fail_in_tx` landed beside them and adds **no** site, because it
  needs no constant and is a plain `&'static str`.
- `postgres_smoke.rs`'s migration count: **45 → 46**, for `0046`, with
  `MANIFEST.sha256` appended in the same commit.

## Tests changed rather than added, and why

Five cases in `backends/crates/vpay-db/tests/repositories.rs` failed after the
counters landed, and the failures were correct: their fixture wrote a
`refunds` row with a raw `INSERT` and no matching reservation on the intent,
which is a state the write path cannot produce and which
`apply_refund_succeeded` is now right to refuse. The helper goes through
`Refunds::create`, and its doc comment records what it used to say and why that
stopped being true.

Two of those cases changed meaning, which is stated rather than smoothed over:

- `two_refunds_against_one_invoice_add_up_and_an_over_refund_is_refused` — the
  over-refund is now refused **at create**, by the intent's reservation, one
  statement earlier than it used to be. Migration `0042`'s
  `refunded_at_most_paid` on the invoice becomes a backstop the write path
  cannot reach, so the case asserts it directly in raw SQL. Without that,
  moving the guard forward would have quietly left `0042`'s CHECK with no test
  at all.
- `two_refunds_settling_concurrently_add_up_and_the_over_refund_still_loses` —
  the first pair (two refunds that fit, settling concurrently) is unchanged and
  still measures that the invoice increment is an expression over the row's own
  column. The second pair now races at _create_, for the same reason.

## Files

- `backends/crates/vpay-core/src/ids.rs` — `lt_` and its minter.
- `backends/crates/vpay-db/src/refunds.rs` — `NewRefund`, `create`, `cancel`,
  `fail_in_tx`.
- `backends/crates/vpay-db/src/payment_intents.rs` — the reservation and its
  two releases.
- `backends/crates/vpay-db/src/settlement.rs` — `post_capture`, `post_refund`,
  the refund counters, `apply_refund_failed`.
- `backends/crates/vpay-db/src/ledger.rs` — the seven moved tests, plus
  `entry_id` and its injectivity test from the review.
- `backends/crates/vpay-db/src/error.rs` — `OverRefund`, `UnknownCurrency`.
- `backends/crates/vpay-db/src/repository.rs` — the trait method removed.
- `backends/crates/vpay-db/src/charges.rs` — `id_for_intent_in_tx`.
- `backends/migrations/0046_ledger-id-length.sql`.

## 2026-09-15 — the conventions, schema and blast-radius review

A second, adversarial pass over the branch, on `review/w2c-conventions`. Its
lens was conventions, schema and blast radius; money correctness and
concurrency were a separate reviewer's and are not re-litigated here.

### What it confirmed

- **Amendment 1 stayed discharged.** `TxRepositories::post_ledger_transaction_in_tx`
  exists nowhere. A tree-wide grep for both names finds only `pub(crate) async
fn post_in_tx`, its two call sites in `vpay_db::settlement`, its own tests,
  and prose. `pub mod ledger` exports the `Ledger` **read** trait and nothing
  else, and `cargo xtask verify-repositories` passes.
- **The moved tests still prove what they proved.** Diffed case by case
  against `be55ff6c`; the assertions are carried over verbatim, including the
  `LedgerError::Unbalanced { currency, debits, credits }` match and the
  `(0, 0)` both-tables count. Nine tests in `vpay_db::ledger::tests`, seven of
  them on a real container, all passing.
- **`OverRefund` is classified as claimed.** `Category::Conflict` → `409` →
  `Retry::Never` (`vpay_core::error`), code `over_refund` rather than
  `resource_conflict` so a merchant can tell "not that much left" from "you
  already did this". It carries `#[source] sqlx::Error`, **not** `#[from]`, so
  the delegation rule `verify-errors` enforces does not apply to it — and
  could not, since `sqlx::Error` implements no `Classify`. Matching migration
  `0003`'s constraint by **name** rather than by SQLSTATE `23514` is right:
  `payment_intents` carries four other CHECKs on the same columns and every
  one of them is a vpay bug. `cargo xtask verify-errors` passes: 19 error
  types, 17 `#[from]` variants.
- **Migration `0046` meets `0045`'s header standard** — `DEPLOY ORDERING` in
  both directions (forward-compatible because a CHECK only narrows and the
  tables are empty; not backward, because `run_migrations()` never sets
  `ignore_missing`) and a measured `DRIFT` section. `verify-migrations` passes
  at 46 files and `MANIFEST.sha256` gained exactly one line.
- **`EXPECTED_DRIFT_CHANGES`' off-pin note is good enough to make a CI
  disagreement cheap.** It names the binary (0.11.1), the exact report line,
  that `EXPECTED_DRIFTED_RELATIONS` is unmoved at 25, that the delta was taken
  under a single binary, and that CI is the arbiter. Nothing was `cargo
install`ed into the shared `~/.cargo/bin`.
- **`NewRefund` carries no `currency_code`, `merchant_id` or `charge_id`, and
  nothing else on the path takes them from caller input either.** The
  currency and the charge come off the intent row the transaction has locked
  (`reserve_refund_in_tx` → `charges::id_for_intent_in_tx`); `post_capture`
  and `post_refund` build `AccountKind::MerchantPayable` from
  `intent.merchant_id`. `merchant_id` on `Refunds::create` and `cancel` is a
  tenant **filter** in the `WHERE` clause, never a written value.

### What it changed

- **A doc comment had come unattached from its function.**
  `charges::id_for_intent_in_tx` was inserted between `insert_for_intent`'s
  doc block and `insert_for_intent`, so a `SELECT` was documented as "opens
  the single charge for an intent" and as raising
  `UniqueViolation { constraint: "one_charge_per_intent" }`, and the INSERT
  was left undocumented. Reordered. Two stale paths in the new doc
  (`crate::refunds::create_in_tx`, which never existed) corrected.
- **The `{transaction_id}_{index}` collision was retracted.** See "What was
  left open" above; the short version is that the derivation is injective for
  every pair of transaction ids, the counterexample in the amendment is not
  one, and it had been repeated into five files untested.
  `vpay_db::ledger::entry_id` is now a named function carrying the argument,
  and `tests::the_entry_id_derivation_is_injective` asserts it.
- **Migration `0046`'s two CHECKs now have a test that proves they fire**, not
  just a drift count that moved. See "Ledger ids are minted" above.
- **Three counts corrected**: `vpay_db::ledger::tests` says seven container
  tests, not six; `postgres_smoke.rs`'s § 0045 banner lists all seven that
  moved (it had listed six, omitting the entry-id case); this page says seven.

### Gates re-run on `review/w2c-conventions`

| Command                                                                         | Result                           |
| ------------------------------------------------------------------------------- | -------------------------------- |
| `cargo nextest run -p vpay-core -p vpay-ledger`                                 | 86 passed, 0 skipped, 0 ignored  |
| `cargo nextest run -p vpay-db`                                                  | 242 passed, 0 skipped, 0 ignored |
| `cargo nextest run -p vpay-tests-integration -E 'binary(postgres_smoke)'`       | 52 passed, 0 skipped, 0 ignored  |
| `cargo test --doc -p vpay-core -p vpay-ledger -p vpay-db`                       | 57 + 6 + 8 passed, 0 ignored     |
| `cargo clippy -p vpay-core -p vpay-ledger -p vpay-db --all-targets -D warnings` | clean                            |
| `cargo clippy -p vpay-tests-integration --all-targets -D warnings`              | clean                            |
| `cargo +nightly fmt --all --check`                                              | clean                            |
| `cargo xtask verify-errors`                                                     | ok — 19 types, 17 `#[from]`      |
| `cargo xtask verify-migrations`                                                 | ok — 46 files                    |
| `cargo xtask verify-repositories`                                               | ok — 4 implementations           |
| `cargo xtask verify-status`                                                     | ok — 1 unimplemented item        |
| `cargo xtask verify-links`                                                      | ok — 1 650 links                 |
| `just check-schema`                                                             | ok (0.11.1 on `PATH`, warned)    |

`just ci` was **not** run, for the branch's reason. The counts moved by +1 in
`vpay-db` and +1 in `postgres_smoke` because the review added one test to
each; nothing was removed.

### Two mutations, run and reverted

- `format!("{transaction_id}_{index}")` → `format!("{transaction_id}{index}")`
  in `vpay_db::ledger::entry_id`. `the_entry_id_derivation_is_injective`
  **failed**: `_10 is derived by both ("_", 10) and ("_1", 0)`. Reverted and
  re-run green.
- `ALTER TABLE ledger_transactions DROP CONSTRAINT id_length` applied before
  `an_over_long_ledger_id_is_refused_by_the_database`. It **failed**:
  `a 65-character ledger transaction id must be refused: PgQueryResult {
rows_affected: 1 }`. Reverted and re-run green.

### What the review deliberately did NOT do

- It did not edit migration `0046`, which still carries the sentence "its ids
  are exactly the ones that can collide with each other". Migrations here are
  forward-only and immutable (issue #76), and adding a migration whose only
  content is a corrected comment is a maintainer's call, not a reviewer's.
  `vpay_db::ledger::entry_id`'s doc and `docs/flows/ledger.md` § Status are
  the correction of record, and the test `0046`'s header cites still exists
  under the name it cites.
- It did not change `SettledRefund` to carry `currency_code`. See the note
  under "What was left open".
