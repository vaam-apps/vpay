# CrateStack — the money tables: migration 0037 (2026-09-07)

_Archived from [docs/status.md](../../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../../` because the file moved two directories down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](../README.md) is the index of them._

#### The money tables (2026-09-07, S5): migration `0037`, four models, four queries

**The last rung of the CrateStack ladder, and it is a smaller rung than the
plan expected.** `payment_intents`, `charges`, `refunds` and
`checkout_sessions` are all modelled now; **four queries moved, all four of
them reads, all four on `checkout_sessions`, and nothing at all moved on the
other three tables.** The asymmetry is the finding and it is stated first so
that nothing here reads as more finished than it is.

**Migration `0037`** converts the last four native enums on a money table —
`intent_status`, `charge_state`, `refund_status`, and the `failure_code`
shared by `charges.failure_code`, `payment_intents.last_payment_error_code`
and `refunds.failure_code` — to `TEXT` plus a `<table>_<column>_enum_check`.
`account_kind` and `direction`, on the ledger tables, are the two of vpay's
seven that remain. Three things it measured that the source does not give you:

- **A partial index blocks the `ALTER` outright.** `charges_live_idx` is
  `ON charges (state) WHERE state IN (…)`, and the conversion fails with
  `ERROR: operator does not exist: text = charge_state`. It is dropped and
  rebuilt. A plain index on the same column needs nothing.
- **It moves the drift count by ZERO** — 130 before, 130 after, measured on
  the same freshly migrated database with only the migration applied. That is
  what "the enum conversion no report can see" predicts, and this is the
  second table set to confirm it.
- **Applied to a populated database** carrying every label of every one of
  the four types, every row keeps exactly its value, and all four types then
  `DROP` — without `CASCADE`, so a dependency this migration did not find
  fails it loudly instead of silently dropping whatever depended on it.

**⚠️ It is not backward compatible with the previous binary, and it has two
failure modes rather than one.** Measured 2026-09-08 by building 889d045's
`vpay-db` and running it against real 0037 databases:

- a previous-release process that **restarts** does not serve at all — it
  fails at boot in `run_migrations()` with
  `migration 37 was previously applied but is missing in the resolved
migrations` (`sqlx::MigrateError::VersionMissing`), before boot step 4 and
  before the listener binds. This is what 0032 does on a restart too; it is
  **not** the difference between them, and this page said it was;
- a previous-release process that **keeps running** — the rolling-deploy
  window — fails money **writes** with `42704 type "intent_status" does not
exist` (`insert`, `transition`) and `42704 type "charge_state" does not
exist` (`set_live_state`). That in-flight window is what 0037 has and 0032
  did not.

**Reads are unaffected**, which is the quiet part: `status::TEXT AS status`
casts to a built-in type this migration does not drop, and
`PaymentIntents::get_for_merchant` answered normally on the previous binary.
The rolling-deploy window therefore does not look like an outage — `GET` keeps
serving and only the statements that move money fail. Deploy it with the
release that carries the matching code, drain the previous version first, and
do not roll that release back past it. The migration's own header carries the
rule and the measurement, which is where an operator looks.

**Why nothing on three of the four tables moved, and it is one reason rather
than a list.** Every read and every write in `vpay_db::payment_intents`,
`vpay_db::charges` and `vpay_db::refunds` returns a row struct carrying a
`jsonb` column — `payment_method_types` and `metadata`, `provider_ref_extra`,
`metadata` — and `Value::from_plain_json` demotes any non-`i64` number to
`f64`. `metadata` is arbitrary merchant-authored JSON, so that is the column
the conversion must not be lossy on. The models therefore do not declare
those columns, a generated read that omitted them would answer a struct the
repository cannot build, and a second statement to fetch them would be a
consistency window on the money path. **`model PaymentIntent`, `model Charge`
and `model Refund` carry no `@@allow` arm at all** — deny-by-default, which
is the honest declaration for a model nothing queries.

`refunds` has a second, independent blocker worth recording because the brief
that produced this work assumed otherwise: **there is no refund create to
move.** `vpay_db::refunds` is two reads and no write, because
`ProviderAdapter::refund` is `NotImplemented` on MTN and `Unsupported` on
Orange. Both reads are also merchant-scoped through a JOIN onto
`payment_intents` (the table carries no `merchant_id` of its own), and a
generated read filters columns of one table.

**`checkout_sessions` moved because migration 0028 gave it nothing to trip
on** — no `jsonb`, no `bytea`, no native enum. `get_for_merchant`,
`get_by_id_unscoped`, `find_open_by_intent` and `find_latest_by_intent` are
`find_unique`/`find_many` calls, and `EXPECTED_ASSERT_SITES` falls **45 → 41**
for the first time in its history: four places the compiler's `&'static str`
check was switched off are gone. Five statements on the same table did not
move, each for its own reason — `list_page`'s correlated sub-select cursor;
`expire`, `due_for_expiry`, `expire_due` and `settle_for_intent`, each of
which carries a `NOT EXISTS` over `charges` that a generated builder cannot
express and that is the whole point of the statement; and `create`, which is
expressible and **deliberately deferred**, because moving it needs `0037` to
drop two column defaults on a money table and that is 0033's trade in a place
where 0033's argument does not carry over.

**One of the four is pinned on its SQL rather than on its behaviour, and that
is a measurement rather than a preference.** Deleting the `ORDER BY seq DESC`
from `find_latest_by_intent` leaves a container test that seeds two sessions
and asserts _which_ one comes back **green**, because
`checkout_sessions_intent_seq_idx` is `(payment_intent_id, seq DESC)` and
Postgres answers an unordered `LIMIT 1` out of it in seq-descending order
anyway. The behaviour was right by accident.
`the_latest_session_query_orders_by_seq_and_takes_one` previews the same
builder the query runs and is red in a millisecond; the container test's own
doc comment now says plainly that it does not catch this.

**Drift: 130 → 141 over 20 relations, unmappable unchanged at 18**, measured
per table before and after — `payment_intents` 31 → 19, `charges` 23 → 16,
`checkout_sessions` 1 → 21, `refunds` 1 → 11, and nothing else by a line. The
two falls are the report describing **rot** rather than drift: `model
PaymentIntent` still declared `last_payment_error`, a column migration 0014
DROPPED, and neither it nor `model Charge` knew about `seq`, `description`,
`customer_id`, `client_secret_suffix`, `provider_txn_id`, `return_url` or
`updated_at`. Nothing read or wrote through either, so nothing noticed. After
the rewrite, **not one `column … is declared in the schema but does not exist`
and not one `column … exists in the live database but is not declared` line
survives on any of the four tables**. The two rises are `customers`' lesson
repeated: a table nobody declares collapses to one line however wrong it is.

**Ten of the 67 remaining lines are FALSE, and that is a fifth measured
upstream gap.** The report says ten foreign keys are "declared in the schema
but do not exist in the live database"; all ten exist.
`cratestack-migrate-0.12.0/src/introspect/postgres/mod.rs` says so itself
under "Known gaps": foreign keys are not introspected, so
`TableProjection::foreign_keys` is always empty and any table with declared
relations shows every one as missing. The relations are declared anyway —
they are true, `cratestack-parser` requires both sides, and removing a true
declaration to make a false line disappear would be optimising the constant
instead of the schema. The latent hazard is the mirror of the
undeclared-CHECK one: a generated `migrate diff` would emit
`ADD CONSTRAINT … FOREIGN KEY` for ten constraints that already exist.
Nothing runs `migrate diff` here.

**One behaviour changed and is not visible in any signature.**
`PaymentIntents::transition` and `Settlement::set_live_state` take an
`expected` label that used to be bound as an enum cast, so a label outside
the vocabulary was a Postgres error and a `500`. It is a plain `TEXT`
comparison now: such a label matches no row and answers `Ok(None)` /
`Ok(false)`, which a caller turns into a `409`. Every caller passes a label
from `vpay_core`'s state machine, so this is two spellings of "a vpay bug" —
recorded because a 409 is quieter than a 500 and nothing else would say so.

**`jobs`, `idempotency_keys` and `provider_requests` are out of scope
FOREVER unless a reason appears.** All three keep working raw `sqlx`
implementations and none of them is pending work: `Jobs::claim` needs
`FOR UPDATE SKIP LOCKED` and `FindMany::for_update()` emits a bare
`FOR UPDATE`; `idempotency_keys`' merchant-scoped `claim_id` semantics have
no delegate equivalent and three of its columns are unmapped types
(`bytea`, `int2`, `jsonb`); `provider_requests`' `latest_submit_attempt`
orders `sent_at DESC, id DESC` and pairs `status_code`/`responded_at`, and
both `attempt` and `status_code` are `int4`. They are listed here so that
neither this page nor the reference doc reads as though they were a gap
somebody forgot.

**Reviewed 2026-09-08, and the review moved things.**
[plans/exp34-money-tables-notes/opus-review.md](../../plans/exp34-money-tables-notes/opus-review.md)
is the sabotage pass. Four guards existed only in prose and now exist:
`the_latest_session_query_orders_by_seq_and_takes_one` (the `ORDER BY` on
`find_latest_by_intent`, which the commit that claimed to add it did not
touch the file for),
`migration_0037_keeps_every_stored_label_on_a_populated_database`,
`every_enum_check_0037_created_refuses_a_value_outside_it`,
`the_crash_recovery_sweep_is_still_served_by_the_rebuilt_partial_index`, and
in `schema.rs` `the_three_money_models_answer_no_rows_to_every_action` and
`no_generated_read_on_a_money_table_can_carry_its_jsonb_column` — the last two
because adding `@@allow("read", auth() != null)` to `model PaymentIntent`, or
declaring `metadata Json` on `model Refund`, was green through every gate.
The backward-compatibility note above was rewritten from a measurement rather
than reworded; the drift numbers, the per-table table and the ten false
foreign-key lines were all re-measured from scratch and hold exactly.

**What is NOT claimed.** `@@audit` was **not** enabled on any model and the
brief's decisive test for it was not attempted — not because it could not be
made to fail, but because it is not applicable yet: the audit hook is on
CrateStack's generated **write** path, and there is no CrateStack write on any
money table to audit. It becomes runnable the day `create` moves.
[plans/exp34-money-tables-notes/opus.md](../../plans/exp34-money-tables-notes/opus.md)
carries that reasoning and the full mutation table. No write on any money
table runs through CrateStack, so the settlement statement, the confirm path's
two-row transaction and every `jobs` insert are exactly the raw `sqlx` they
were.
