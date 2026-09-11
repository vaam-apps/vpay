# `vpay-db` — dynamic SQL strings and sqlx 0.9

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## Dynamic SQL strings and sqlx 0.9

Every statement in this crate is built with `format!` and then wrapped in
`sqlx::AssertSqlSafe`. Both halves need explaining, because the wrapper's name
is a promise and a promise nobody re-reads is worth nothing.

sqlx 0.9 (sqlx#3723) changed `query`, `query_as` and `query_scalar` to take
`impl SqlSafeStr`, which is implemented for `&'static str` and for the explicit
`AssertSqlSafe` wrapper — and for nothing else. A `String` built by `format!`
therefore no longer compiles as a statement. Under 0.8 this crate passed
`&sql` at 36 call sites; under 0.9 it passes `AssertSqlSafe(sql)` at the same
36 — **37 since 2026-09-05**, when `refunds::get_for_merchant` landed with
issue #45, **39 since 2026-09-06**, when `refunds::list_for_intent` and
`events::list_for_objects` landed with the `/dash/v1` payment detail (exp23),
and **45 since the same day**, when `customers` landed with S4a
(`insert_in_tx`, `get_for_merchant`, `update_in_tx`, `list_page`,
`idle_since`, `erase_idle` and, since migration `0041`, `erase_in_tx`'s
branch query, its anonymising `UPDATE` and two of its five redaction
statements — the other three are plain `&'static str` and need no wrapper;
`touch_last_used` and the hard-delete branch go through CrateStack and build
no string at all).
`EXPECTED_ASSERT_SITES` went **56 -> 60** in that change, a net +4 over five
additions and one removal, and its own doc comment enumerates them. Taking the
`String` **by value** rather than `AssertSqlSafe(&sql)` is
deliberate: the borrowed form goes through `AssertSqlSafe<&str>`, which sqlx's
own docs describe as copying the string.

The `format!` predates 0.9 and is not what that change is about. Each of these
statements ends in a column list — `RETURNING {COLUMNS}`, `SELECT {COLUMNS}` —
and that list is a `const … : &str` declared once per module so that a column
added to a `SELECT` cannot drift from the column read out of the `PgRow`. That
is the entire reason a statement here is not a literal.

### The audit, done rather than asserted

`AssertSqlSafe`'s contract is that the caller audited the string. Here is the
audit, re-done on 2026-09-05 from the source rather than inherited:

All 45 statements interpolate exactly two kinds of value. Re-done on
2026-09-06 for the six `customers` statements S4a added.

- **A `const … : &str` declared in this crate.** Thirteen of them:
  `charges::COLUMNS`, `checkout_sessions::COLUMNS`, `customers::COLUMNS`,
  `events::COLUMNS`,
  `payment_intents::COLUMNS`, `refunds::COLUMNS`,
  `webhook_deliveries::COLUMNS`,
  `checkout_sessions::OPEN`, `customers::UNREFERENCED`,
  `payment_intents::LIVE_CHARGE_STATES`,
  `payment_intents::SETTLEABLE_STATUSES`, `jobs::CLAIM_RETURNING` and
  `settlement::PREVIOUS_STATE`. A `const` cannot carry a caller's value.

  `customers::UNREFERENCED` is the one that had to _become_ a constant: it is
  the `NOT EXISTS` pair the retention sweep's read and its write must both
  carry, and it was first written as `fn unreferenced(alias: &str) -> String`
  so the two call sites could name the table differently. They do not — both
  spell it `customers` — and the audit is what said so, by failing on
  `{guard}`. A computed fragment fails this gate by construction however fixed
  its inputs are, which is the rule working rather than an inconvenience.

- **`direction`**, which is
  `let direction = if backwards { "ASC" } else { "DESC" };` — a `bool`
  choosing between two literals written in the same function
  (`events.rs`, `payment_intents.rs`, `checkout_sessions.rs`, in each case
  inside `list_page`). Postgres has no bind parameter for a sort direction,
  which is why it is interpolated at all.

**Re-checked 2026-09-06 for the two sites exp23 added, and for the one it
did not.** `refunds::list_for_intent` and `events::list_for_objects` are each
`SELECT {COLUMNS} … WHERE …` with `COLUMNS` the module's own `const` and every
caller value bound — `merchant_id`, `payment_intent_id`, and, in the events
read, a whole `&[String]` of object ids passed to `= ANY($2)` as one bound
array rather than a generated `IN (…)` list, precisely so the statement's
_text_ does not depend on the number of arguments. `payment_intents::list_page_filtered`
adds three predicates and **no** site: it is the existing `list_page`
statement with `($5::TEXT IS NULL OR status::TEXT = $5)` and two timestamp
bounds written the same way, so an absent filter sends the byte-identical
statement `/v1` sends and a filter value can never reach the SQL text.

**Re-done 2026-09-10 for issues #57 and #66, where the count moved 55 → 56 and
the net of +1 hides four additions and three removals.** Four terminal writes
left the pool for the caller's transaction, because each now commits with the
`events` row that reports it: `payment_intents::cancel` → `cancel_in_tx`
(interpolates `COLUMNS` and `LIVE_CHARGE_STATES`), `customers::create` →
`insert_in_tx` and `customers::update` → `update_in_tx` (both `COLUMNS`), and
the new `customers::lock_for_update` (`COLUMNS`), the `SELECT … FOR UPDATE`
that makes the update's `metadata` merge definite. The three pooled originals
were **deleted** rather than kept beside the transactional ones — which is
what makes "write the row and tell nobody" inexpressible — so the count moved
by one. Every one of the four interpolates crate constants and binds every
caller value, so the paragraph below is unchanged.

**Re-done 2026-09-10 for issue #91's D5, where the count moved 56 → 57 — one
addition, not a net.** `invoices::add_refund_for_intent_in_tx` is
`UPDATE invoices SET amount_refunded = amount_refunded + $2, updated_at = $3
WHERE payment_intent_id = $1 AND status = 'paid' RETURNING {COLUMNS}`. It
interpolates `invoices::COLUMNS` and nothing else; all three caller-supplied
values are bound, and the increment is an _expression over the row's own
column_ rather than a computed total, so there is not even an arithmetic
result to interpolate.

The other statement that landed with it, `refunds::settle_in_tx`, adds **no
site at all**, and that is worth a sentence rather than silence: it needs no
constant, so it is written as a plain `&'static str` and the compiler's own
check is never switched off for it. A statement that does not have to be a
`format!` should not be one — the count above is the budget, and this is what
spending nothing looks like.

**No caller-supplied value reaches a statement string anywhere in this crate.**
Every merchant id, intent id, cursor, limit, status, timestamp and payload is
already a bind parameter — the `.bind(..)` calls immediately below each
statement are the whole argument list.

That audit is a claim about a file that people edit, so it is also a test:
`vpay_db::sql_audit` (test-only, `backends/crates/vpay-db/src/sql_audit.rs`)
reads this crate's own sources and fails if a `format!` bound to `sql`
interpolates anything that is not one of the constants above or one of the two
named exceptions. It was proven to fire by three mutations on 2026-09-05, each
reverted — and it fired **unprompted** on 2026-09-06, twice, against S4a's
first draft of `customers.rs`: once correctly, on the computed `{guard}`
described above, and once as a false positive on a `format!("{column} = ")`
_inside a `#[cfg(test)]` assertion_, which the scanner read as a statement
because it looks for the word `sql` in the forty characters before a `format!`
and the assertion's own message printed `{sql}`. The test now builds that
needle with `concat` and says why. Worth recording rather than quietly working
around: the scanner is textual, so it will do this again, and the answer is to
avoid `format!` in a test that mentions `sql` — never to widen the
allowlist.

The three 2026-09-05 mutations:

- interpolating `{payment_intent_id}` into `charges::get_for_intent` →
  `every_interpolation_into_a_statement_is_a_crate_constant` fails, naming the
  file and the capture;
- redefining `direction` as anything other than the two-literal `if` →
  `the_audited_non_constants_are_still_what_the_audit_says_they_are` fails;
- wrapping a fresh `format!` in `AssertSqlSafe` instead of the audited `sql`
  variable → `every_assert_sql_safe_wraps_the_variable_the_audit_covers`
  fails.

The third is the one that matters most: without it the audit could be bypassed
by not using the variable the audit looks at.

**A fourth mutation, added by review on 2026-09-05, is why the list above was
not enough.** All three mutations above spell the interpolation by _name_.
Written positionally —

```rust
let sql = format!("SELECT {COLUMNS} FROM charges WHERE payment_intent_id = '{}'", payment_intent_id);
```

— the same injection passed all five tests, because the scanner discarded a
capture with no name. Since a positional capture's value comes from the
argument list, which this module does not resolve, it is now reported as a
violation on sight (`sql_audit::POSITIONAL_CAPTURE`), and
`a_positional_capture_is_reported_and_is_neither_a_constant_nor_allowed` pins
the scanner behaviour over synthetic text. **Every statement in this crate
captures its constants by name; `{}` is never the right spelling here**, which
is what makes a blanket refusal the correct rule rather than a heuristic.

### Why not `QueryBuilder`

sqlx's own suggested alternative. It was considered and rejected: it would
rewrite 45 working, reviewed statements to remove a risk the audit above shows
is not present, and it would replace SQL that reads as SQL with SQL assembled
by method calls — in a crate where the statement text _is_ the design
(`FOR UPDATE SKIP LOCKED`, `UPDATE … WHERE state = $2 RETURNING`, the
`NOT EXISTS` guards that make cancellation atomic). `QueryBuilder` earns its
place where the _shape_ of a statement varies with input. Nothing here has
that shape: the only variability is a sort direction and a fixed column list.

### The two interpolations that are not constants

`sql_audit`'s allowlist has exactly two entries and both are checked rather
than merely permitted:

- `direction` — the file must still contain the literal two-branch `if`.
- `columns` — `settlement.rs` writes `columns = crate::charges::COLUMNS` as a
  named argument, because it interpolates another module's constant and the
  implicit-capture form cannot name a path.

A third entry is a deliberate edit to that file, which is the review this
arrangement exists to force.
