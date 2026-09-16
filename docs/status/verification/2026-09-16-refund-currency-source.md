# 2026-09-16 — the refund posting's currency comes off the refund, and a mismatch is refused

Branch `fix/settled-refund-currency`, off
`claude/orange-refund-transaction-2b95e5` at `7e2e5381`. Scope: one decided
money-path defect on the refund settlement path. Nothing else was touched.

## The defect

`vpay_db::settlement::post_refund` built the ledger legs with
`money_from_row(refund.amount, &intent.currency_code, …)` — the **intent's**
currency — because `SettledRefund` (`vpay-db/src/refunds.rs`) carried no
currency of its own.

`refunds.currency_code` is a real column. Its only constraint is the foreign
key onto `currencies` (migration `0017`):

```sql
currency_code TEXT NOT NULL REFERENCES currencies (code),
```

**Nothing ties it to the intent's.** The two agree today solely because
`Refunds::create` is the only writer and derives it from the intent inside the
same transaction.

`money_from_row`'s own doc states the rule this broke: both halves of a `Money`
come off the same row, because an amount and the currency it is denominated in
are one fact. A divergent writer would render one currency on the refund object
and post the legs in another, and **`Transaction::validate` would not notice** —
it balances each currency on its own book, and every leg in the same _wrong_
currency balances perfectly. The refund object and the ledger would disagree
with no error anywhere.

"Unreachable from today's call sites" is the reasoning that produced the
mixed-currency ledger defect earlier on this same branch, where it was true
right up until the same branch added the writer.

## What changed

| File                                    | Change                                                                                                                    |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `vpay-db/src/refunds.rs`                | `SettledRefund` gains `currency_code`; `settle_in_tx` and `fail_in_tx` add it to `RETURNING`, read off the row they flip. |
| `vpay-db/src/settlement.rs`             | `post_refund` builds its `Money` from `refund.currency_code` (`table: "refunds"`), and refuses a mismatch first.          |
| `vpay-db/src/error.rs`                  | New `DbError::RefundCurrencyMismatch`, `Category::Internal`, code `refund_currency_mismatch`.                             |
| `tests/integration/…/postgres_smoke.rs` | One real-Postgres case, below.                                                                                            |

`fail_in_tx` gets the column too, for symmetry with its twin — the failure path
posts nothing, so it has no use for it today.

## The decision the brief reserved: yes, refuse a mismatch

The brief asked whether, once the refund's currency is available at the call
site, to compare it to the intent's and refuse a disagreement. **It is
refused**, on a ground that is not in the brief's own two arguments and that
settles it:

> Reading the refund's own currency does not make a mismatched settlement
> correct. It only makes the **ledger** internally consistent with the refund
> object.

By the time `post_refund` runs, `Settlement::apply_refund_succeeded` has
already executed two currency-blind additions in the same transaction:

- `payment_intents::settle_refund_in_tx` — `amount_refunded = amount_refunded + $2`
- `invoices::add_refund_for_intent_in_tx` — the same, on the invoice

Both totals are denominated in the **intent's** currency and neither statement
looks at a currency at all. So on a mismatch there is no correct posting to
make in _either_ code: whichever the legs carried, those two totals are already
counting minor units of one currency into a total denominated in another.
Choosing a currency and committing would be choosing which of two wrong ledgers
to write.

Refusing rolls the whole transaction back — the flip to `succeeded`, both
counters, the invoice and the posting — and leaves the refund `pending` for a
human. That is the fail-closed direction every other error on this path already
takes.

The brief's counter-argument ("a check that can never fire is a check nobody
tests") is answered by the case below, which fires it against a real Postgres.
It is `Category::Internal` and not `Conflict`: `NewRefund` has no
`currency_code` field, so no request a merchant can send produces this, and a
`409` would blame the only party who could not have caused it.

**The cost of that decision, stated rather than buried — see the mutation
table.** With the guard in place, `refund.currency_code` and
`intent.currency_code` are provably equal on the `money_from_row` line, so the
brief's named decisive mutation does not fire. This is inherent, not an
oversight: a guard that proves two values equal makes the choice between them
unobservable. What remains testable, and is tested, is the guard itself and the
`RETURNING` clause the guard reads from.

## The test

`a_refund_whose_currency_disagrees_with_its_intent_is_refused_and_posts_nothing`
in `postgres_smoke.rs`. It is a **second writer of `refunds`**, deliberately:
the row is inserted with hand-written SQL because `vpay_db::NewRefund` has no
`currency_code` field to diverge with, and the reservation is set by hand for
the same reason. A `EUR` refund for 2 000 against an `XAF` intent for 5 000.

It asserts, each separately because a partial rollback satisfies any single
one:

- the insert is accepted — the premise, asserted rather than assumed, so that a
  future constraint tying the two columns together breaks this case loudly
  rather than making it vacuous;
- `apply_refund_succeeded` returns `Err(RefundCurrencyMismatch { … })` naming
  both objects and both codes, with `refund_currency == "EUR"` — which is the
  value read off the refund row;
- `category() == Internal`, `code() == "refund_currency_mismatch"`;
- `ledger_entries` gained no row, and there is no `EUR` leg — a compensating
  pair that netted to zero would satisfy invariant 1 and still be wrong;
- the refund is still `pending`;
- the intent is still `(5 000, 0, 2 000)` — `amount_refunded` did not absorb
  2 000 EUR into an XAF total, and the reservation is still held.

## Mutations run, with their numbers

| #   | Mutation                                                                                                            | Expected | Measured                                                                                                                                                                                                            |
| --- | ------------------------------------------------------------------------------------------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | The brief's: `post_refund` reverted to `money_from_row(refund.amount, &intent.currency_code, "payment_intents")`    | FAIL     | **PASS — 12 of 12 refund cases green.** Reported rather than worked around. The guard two lines above proves the two codes equal, so the argument is unobservable by construction. See "the cost of that decision". |
| 2   | The guard deleted from `post_refund`                                                                                | FAIL     | FAIL, as intended. `a refund denominated in a currency that is not its intent's must be refused: Some((SettledRefund { id: "re_mixed", … currency_code: "EUR" }, None))` — the settlement committed and posted.     |
| 3   | `settle_in_tx`'s `RETURNING` reads the currency off the intent via a correlated sub-select instead of off `refunds` | FAIL     | FAIL, as intended. `SettledRefund { … currency_code: "XAF" }` — the guard never sees the divergence. **This is what pins the currency's source**, and it is the mutation that matters now that mutation 1 cannot.   |

Mutation 1 is the honest answer to the brief's question, not a passing gate.
The property "the legs carry the refund's currency" is, after this change,
unobservable through any behaviour — because the settlement that would
distinguish it is refused. The property that replaced it, "a settlement whose
two currencies disagree is refused and writes nothing", is strictly stronger
and mutations 2 and 3 both catch its removal.

## Gates run

`just ci` was **not** run — the brief forbids it. CI is the gate.

| Command                                                                                      | Result                               |
| -------------------------------------------------------------------------------------------- | ------------------------------------ |
| `cargo nextest run -p vpay-db -p vpay-ledger`                                                | **256 passed, 0 skipped, 0 ignored** |
| `cargo nextest run -p vpay-tests-integration -E 'binary(postgres_smoke)'`                    | **54 passed, 0 skipped, 0 ignored**  |
| `cargo clippy -p vpay-db -p vpay-tests-integration --all-targets --all-features -D warnings` | clean                                |
| `cargo +nightly fmt --all --check`                                                           | clean                                |
| `cargo test --doc -p vpay-db`                                                                | 8 passed, 0 ignored                  |
| `cargo doc -p vpay-db --no-deps --document-private-items`                                    | 43 warnings, the same 43 as the base |

**Postgres tests ran; none skipped.** Every count above is a real
`postgres:16-alpine` container over
`DOCKER_HOST=unix:///run/user/1000/docker.sock`; nextest reports `0 skipped` on
both runs, and `cargo nextest list --run-ignored ignored-only` lists nothing in
either scope, so the "0 ignored" is measured rather than inferred. `postgres_smoke`
is 54 rather than 53 because this change adds one case.

The rustdoc count was measured both ways — 43 on this branch's head and 43 with
`backends/crates/vpay-db/src` and `postgres_smoke.rs` checked back out to the
base — so this change adds no new warning. All 43 are pre-existing
private-item-link and unresolved-link warnings across the crate.

## Not done

- `just ci` was not run, per the brief.
- No adapter, route or worker change: `POST /v1/refunds` is still unrouted and
  no rail has ever executed a refund. This change is reachable from tests and
  from nothing else, exactly as the rest of the refund write path is.
- `vpay-api` does not map `RefundCurrencyMismatch` to a wire error, because no
  route reaches it. `Category::Internal` is what the generic mapping already
  gives it.
- No migration. A CHECK or trigger tying `refunds.currency_code` to its
  intent's would make the guard redundant at the database rather than in Rust;
  that is a schema decision this fix deliberately does not take.
