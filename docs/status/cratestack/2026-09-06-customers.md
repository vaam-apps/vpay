# CrateStack — `customers`, the first table born with a model (2026-09-06)

_Archived from [docs/status.md](../../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../../` because the file moved two directories down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](../README.md) is the index of them._

#### `customers`: the first table born with a model (2026-09-06, S4a)

Migration `0034` creates `customers`, and `schemas/vpay.cstack`'s
`model Customer` lands in the **same commit**. Every table before it acquired
its model afterwards, and migration `0033` exists because of what that costs:
`cratestack-macros` drops every `@default(...)` field from
`Create{Model}Input`, so `providers`' five capability booleans could not be
written through the generated input until their column `DEFAULT`s came off.
`0034` pays that up front — **no column CrateStack may write carries a
`DEFAULT`** — and the two that do, `created_at` and `updated_at`, are declared
`@default(now())` for `DisabledClient.disabled_at`'s measured reason.

**Two of seven repository methods run through the generated layer**, taking
the count to **eight queries over five tables**: `touch_last_used`
(`update_many`) and `delete` (`delete_many`). One column decides the split —
`metadata JSONB NOT NULL`, undeclared for the two costs `model Event.data`
already measured (`map_scalar` does not read `jsonb` back;
`Value::from_plain_json` demotes any number outside `i64` to `f64`) — and the
consequence is sharper here than there: an undeclared column is absent from
the generated model **struct** too, so a CrateStack _read_ could not render
the wire object at all. Create, update and every read are hand-written.

That is not a consolation prize: the two that moved are the two where being
wrong is irreversible, and **both policy failures are silent** —
`update_many` and `delete_many` compile the `@@allow` into the statement's own
`WHERE`, so a deleted arm matches zero rows and returns `Ok`. A missing
`update` freezes every customer's retention clock at creation and the sweep
deletes live customers twelve months later; a missing `delete` answers
`{deleted: true}` while the row stays.
`every_action_this_module_calls_has_an_allow_arm` asserts the compiled
descriptor with no container, and also asserts the **absence** of the three
arms this model deliberately does not grant.

`metadata` carries **no column `DEFAULT`** precisely so that a generated
`INSERT` omitting it is a loud `23502` rather than a silent `{}` over a
merchant's data. Two tests arm that: `vpay-db`'s
`a_generated_customer_insert_cannot_carry_metadata` pins the rendered
statement, and `postgres_smoke`'s
`customers_metadata_has_no_default_so_an_omitted_insert_is_loud` reads
`pg_attrdef` _before_ asserting the failure so it cannot go vacuous.

**Drift: 101 → 113 over 16 → 17 relations, unmappable 17 → 18.** Measured
against a freshly migrated database and accounted for line by line on
`EXPECTED_DRIFT_CHANGES`. The +12 is ten on `customers` (six undeclared
hand-named CHECKs, three undeclared indexes, one `seq` default line —
`model Event.seq`'s known trade) and two on `payment_intents` (the
`customer_id` column and its partial index). **`checkout_sessions` gained the
identical column and index and cost zero**, because it is an undeclared
_table_ and the report collapses the whole thing to one line — so the marginal
drift of a column depends on whether its table is modelled, which is the
opposite of the intuition that modelling reduces drift.
`at_least_one_identifier`, the eleventh multi-column CHECK, contributes
nothing in either direction, which is why `postgres_smoke` asserts it
directly.

**Drift again: 113 → 130 over 17 → 20 relations, unmappable unchanged at 18**
(2026-09-07, ADR-0017's migration `0035`). The +17 is nine on
`staff_members`, four on `staff_sessions` and four on
`oauth_authorization_codes`, **and every one of the seventeen is a hand-named
CHECK or an undeclared index**. Not one `column … type differs`, not one
`column … default value differs`, not one `column … is declared in the schema
but does not exist`, and no `table … is not declared` line for any of the
three. Every earlier modelled table carries at least one of those — `charges`
and `ledger_entries` carry enum-type lines **no migration can remove**, and
`customers` and `events` carry identity-default lines.

That is what "shaped so the data layer can write every column" buys, and the
constant is the evidence rather than the claim: no `bytea` (so the unmappable
count does not move either), no native enum, no `DEFAULT` on any column a
writer names, and no `seq` cursor. `staff_members_totp_is_paired`, the twelfth
multi-column CHECK, contributes nothing in either direction and is asserted
directly for `at_least_one_identifier`'s reason.

**Drift a third time: 130 → 156 over 20 → 23 relations, unmappable 18 → 19**
(2026-09-07, S4b's migration `0036`). The +26 is sixteen on `invoices` (eight
hand-named CHECKs, six undeclared indexes, one `seq` default line, one
`status` type line), nine on `invoice_items` (six CHECKs, two indexes, one
`seq`), and one for `invoice_number_sequences`, which is undeclared as a whole
table.

**One line a reader would expect is absent, and its absence is the result
worth recording: `invoices_status_enum_check` costs nothing.** Migration 0032
had to _rename_ `providers.flow`'s hand-named CHECK after the fact because
`diff/checks.rs` matches by name first; migration 0036 creates the constraint
under `naming.rs::check_name(table, column, "enum")`'s own spelling from the
start, so the declared constraint and the live one are one object and neither
side reports the other missing. It is the first enum column in this repository
to cost zero on the CHECK — the `[lossy] column status type differs (live:
Scalar("String"), schema: Enum("InvoiceStatus"))` line is still permanent, for
`introspect/postgres/enums.rs`' documented reason.

`invoice_number_sequences` is undeclared **deliberately** and is in
`postgres_smoke.rs`'s `tables_missing_from_the_schema` list rather than being
a gap somebody has to notice: its only write is `next_number =
invoice_number_sequences.next_number + 1`, a `SET` whose right-hand side names
the column being set, and `Update{Model}Input` carries values rather than
expressions — the same shape `Customers::touch_last_used` wanted `GREATEST`
for and could not have.

`invoice_items` adds **nothing** to the unmappable count, which is the
property that table was shaped for (no `jsonb`, no `bytea`, no native enum, no
`int4`); `invoices.metadata` adds the one, for `customers.metadata`'s reason.
The five new multi-column CHECKs contribute nothing in either direction, which
is why `postgres_smoke` asserts them directly.

**A naming fact with no gate behind it, found by a container test.** 0.11.1
derives a table name from the model name with
`pluralize(to_snake_case(model))` and has no `@@map`, so `model Staff` reads
and writes `staffs`. The first draft of `0035` created a table called `staff`;
`cargo build`, `just check-schema`, `clippy` and all ten `just verify` gates
stayed green, and the first thing to say anything was a Postgres-backed test
answering `relation "staffs" does not exist`. The model is `StaffMember`, the
table is `staff_members`, and the Rust trait is still `vpay_db::Staff`.

**`to_chrono` is the second chrono/time crossing in this crate and the first
in that direction.** CrateStack's inputs and filters take
`chrono::DateTime<Utc>`; every TIMESTAMPTZ this crate binds by hand is
`time::OffsetDateTime`. Its `unwrap_or` is unreachable — chrono's range is
~±262,000 years and `time`'s is −9999..=9999 — and
`the_chrono_conversion_is_total_over_every_instant_time_can_hold` proves it at
both extremes rather than in prose, so a future `time` with `large-dates`
turns the branch red instead of clamping a retention clock silently.

**`sql_audit` fired twice, unprompted, on the first draft.** Once correctly,
on a computed `NOT EXISTS` fragment (now `const UNREFERENCED`); once as a
false positive on a `format!` inside a `#[cfg(test)]` assertion, which the
textual scanner read as a statement because it looks for the word `sql` within
forty characters before a `format!` and the assertion's own message printed
`{sql}`. Recorded in [reference/vpay-db.md](../../reference/vpay-db.md) rather than
worked around: the scanner will do it again, and the answer is to avoid
`format!` in a test that mentions `sql`, never to widen the allowlist.
`EXPECTED_ASSERT_SITES` 37 → 43, with the audit re-done.
