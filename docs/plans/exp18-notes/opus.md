# S3 — the outbox through CrateStack inside vpay's own transaction

Working notes for `model Event` / `model WebhookDelivery` and the move of
`create_in_tx` and `mark_fanned_out_in_tx` onto `run_in_tx`. Base: `06e27f9`
(master after PR #55, carrying S1, S2a and S2b — 43 test binaries, 1382 tests,
32 migrations, drift 84 / 16 / 17).

Everything below is a measurement taken on this branch on 2026-09-06 against
`postgres:16-alpine` and `cratestack-cli 0.11.1`, or a citation into the
pinned 0.11.1 crate sources. Conclusions live in
[docs/reference/vpay-db.md](../../reference/vpay-db.md) § CrateStack,
[docs/status.md](../../status.md) and
[docs/flows/webhooks.md](../../flows/webhooks.md). This file is the
transcript.

---

## 1. The headline: the brief's plan for `events` is impossible, and the reason is one `NOT NULL`

The brief said: model both tables **without** the JSONB columns, and "keep
reading/writing them with one hand-written statement each inside the same
transaction". For `webhook_deliveries` that is vacuous — migration 0022 has
no JSONB column at all. For `events` it cannot be done, and the reason is
mechanical:

`events.data` is `JSONB NOT NULL` with **no `DEFAULT`** (migration 0018). A
model that does not declare it generates

```text
INSERT INTO events (id, merchant_id, livemode, type, object_id)
VALUES ($1, $2, $3, $4, $5) RETURNING ...
```

which Postgres refuses:

```text
23502: null value in column "data" of relation "events" violates not-null constraint
```

There is no split of one `INSERT` into two statements. So `insert_in_tx` did
not move. The two writes that *could* move did.

### 1a. And "CrateStack cannot round-trip JSONB" is false, which changes the argument

The brief cites F5. Checked against the pinned sources rather than accepted:

| Claim | Source | Verdict |
|---|---|---|
| `Json` is a real `.cstack` scalar and emits `JSONB` | `cratestack-migrate/src/emit/postgres/columns.rs:275` | ✔ **exists** |
| `Json` generates `::cratestack::Json<::cratestack::Value>` in the row struct | `cratestack-macros/src/shared/types.rs:36` | ✔ **exists** |
| `SqlValue::Json` binds through sqlx's `Json` wrapper | `cratestack-sqlx/src/query/support/values.rs:41` | ✔ **exists** |
| `map_scalar` reads `jsonb` back | `cratestack-migrate/src/introspect/postgres/types.rs` | ✘ **not mapped** |

So a `data Json` field would *work at runtime*. It is still not declared, for
two measured reasons, and the second is the one that decides it:

1. **Drift gets worse.** An *undeclared* `jsonb` column is invisible to the
   comparison in both directions (§3 below: declaring both models moved
   `EXPECTED_UNMAPPABLE_COLUMNS` by zero and produced no `column data exists
   in the live database` line). Declaring `data Json` leaves the live column
   invisible while adding a `[blocking] column data is declared in the schema
   but does not exist in the live database` line — precisely what
   `currencies.exponent` did before migration 0032.

2. **`cratestack::Value` is not `serde_json::Value`, and the conversion is
   lossy on numbers.** `cratestack-core/src/value.rs:95`:

   ```rust
   serde_json::Value::Number(number) => match number.as_i64() {
       Some(value) => Value::Int(value),
       None => Value::Float(number.as_f64().unwrap_or_default()),
   },
   ```

   A `u64` above `i64::MAX` anywhere in the payload comes back as an `f64`.
   `events.data` is the wire object that is stored, **signed** and delivered
   to a merchant, and it embeds merchant-authored `metadata` — arbitrary JSON
   vpay does not choose. That is not a conversion to introduce to save one
   raw statement.

Pinned so an upstream fix is noticed rather than remembered:
`the_events_insert_cannot_move_until_a_json_column_can_be_modelled` (no
database; pins the rendered five-column `INSERT`) and
`a_generated_events_insert_is_refused_by_the_not_null_on_data` (runs exactly
that statement and asserts the `23502`).

---

## 2. The second blocker, and why it was closable when `Provider`'s was not

`webhook_deliveries.id` is `UUID PRIMARY KEY DEFAULT gen_random_uuid()`.
With `@default(gen_random_uuid())` in the model,
`cratestack-macros/src/model/inputs.rs:157` (`generate_upsert_input_struct`)
returns an **empty token stream**:

> Server-generated PKs get no upsert impl; calling `.upsert(...)` is a
> compile error.

`is_generated_on_create` is `has_default(field)`. Measured:
`error[E0599]: no method named 'upsert' found`.

`.upsert(..).do_nothing()` is **not** optional here. `create_in_tx` must
answer `Ok(None)` for a pair that already exists, because the drain is
at-least-once and a re-run must enqueue no second `deliver_webhook` job. A
bare `.create(..)` raises `23505` against
`webhook_deliveries_event_endpoint` — and a failed statement **aborts the
enclosing Postgres transaction**, so the error cannot be caught and mapped
back to `None` without a `SAVEPOINT` this layer does not have.

The fix: the model does not declare the default, and `vpay-db` mints the
`Uuid`. Cost: **one** `[safe] column id default value differs` line
(100 -> 101). The database default is untouched and now vestigial.

**This is the same shape as `model Provider`'s reserved decision, with the
opposite answer, and the difference is worth stating.** Unblocking
`Provider`'s upsert requires `ALTER TABLE providers ALTER COLUMN … DROP
DEFAULT` on five columns — a code generator's input-shaping rule deciding
vpay's DDL, which exp17 correctly reserved for the maintainer. Unblocking
this one required **no DDL at all**: nothing else writes `webhook_deliveries`
without supplying an id, so the column default has no remaining reader.
Dropping it in a migration would close the drift line and is deliberately not
done here (see §6).

---

## 3. The drift arithmetic, derived line by line

Driven with `cratestack migrate baseline --strict` against freshly built
databases (all 32 migrations applied with `psql`, plus a hand-created
`_sqlx_migrations` so the undeclared-table set matches the real run).

| Variant | Report |
|---|---|
| **A** — base, no new models | **84 / 16 / 17** (reproduces the pinned constants exactly) |
| **B** — both models, `@default("pending")` double-quoted | 102 / 16 / 17 |
| **C** — B with `@default('pending')` single-quoted | 100 / 16 / 17 |
| **D** — C with `webhook_deliveries.id`'s default dropped from the model | **101 / 16 / 17** — as delivered |

Two findings in that table:

- **B -> C is -2, and it is a quoting rule nobody would guess.**
  `convert/fields.rs::field_default` classifies the argument: `dbgenerated()`
  is a marker, anything ending in `)` is a `Function`, everything else is a
  bare `Literal`. The live default deparses to `'pending'::text`, so
  `@default('pending')` compares equal and `@default("pending")` does not.
  Two `default value differs` lines, on `events.fanout_state` and
  `webhook_deliveries.state`.
- **C -> D is +1**, and it is the price of §2.

### 3a. The +17, itemised

`events` (11 lines) and `webhook_deliveries` (8) replace two
`table … is not declared` lines. Every one of the 19 new lines:

```text
events:
  [safe] CHECK `data_is_object` ... not declared in the schema
  [safe] CHECK `fanout_attempts_is_not_negative` ...
  [safe] CHECK `fanout_state_is_known` ...
  [safe] CHECK `id_length` ...
  [safe] CHECK `merchant_id_length` ...
  [safe] CHECK `object_id_length` ...
  [safe] CHECK `type_is_a_documented_event` ...
  [safe] index `events_merchant_seq_idx` ...
  [safe] index `events_pending_idx` ...
  [safe] index `events_seq_key` ...
  [safe] column `seq` default value differs from the schema

webhook_deliveries:
  [safe] CHECK `attempt_is_not_negative` ...
  [safe] CHECK `endpoint_id_length` ...
  [safe] CHECK `excerpt_length` ...
  [safe] CHECK `state_is_known` ...
  [safe] CHECK `url_length` ...
  [safe] index `webhook_deliveries_event_endpoint` ...
  [safe] index `webhook_deliveries_live_idx` ...
  [safe] column `id` default value differs from the schema
```

**`@db_enforce` would make this worse, not better.** It promotes a validator
to a CHECK named `naming.rs::check_name` -> `<table>_<column>_<validator>_check`,
and the live names are hand-written — so each of the four length CHECKs would
become a drop-and-add *pair*: two lines where there is now one. And
`ir/checks.rs` guarantees the kinds could never match anyway, since
introspection reports every validator-derived CHECK as `CheckKind::Raw`.
That is exp17 §1a applied in the only direction available without a rename
migration.

The validators are still declared **without** `@db_enforce`, and that is not
a no-op: `inputs.rs::validate_impl_tokens` generates `input.validate()` from
the validator list regardless, and every write path calls it before any SQL
runs. Consequence measured in §5.

### 3b. `seq`: one line, deliberately

`events.seq` is `GENERATED ALWAYS AS IDENTITY`. `information_schema` reports
`column_default` NULL and `is_identity = YES`; introspection reads only the
former. So `@default(dbgenerated())` costs one permanent line.

Dropping it closes the line — and makes `seq` a **required** field of
`CreateEventInput`, i.e. an explicit value for a `GENERATED ALWAYS` column,
which Postgres refuses outright. One honest drift line beats a create input
that cannot be filled in correctly.

### 3c. Zero lines for four columns, and why that is worth a constant

`events.data`, `events.fanout_attempts`, `webhook_deliveries.attempt` and
`webhook_deliveries.status_code` are `jsonb`/`int4`, on `map_scalar`'s
deliberate unmapped list. They contribute **nothing** in either direction:
`EXPECTED_UNMAPPABLE_COLUMNS` stays at 17, and no `exists in the live
database` line appears. Declaring them as `Int`/`Json` would have *added*
blocking lines.

Four of the seventeen unmappable columns now sit inside tables the schema
models **fully**, which is a sharper version of that constant's own point:
unmeasured drift can hide inside a table the report otherwise compares column
by column. Recorded on the constant.

---

## 4. What the two moved statements render to

`preview_sql`, no I/O:

```text
INSERT INTO webhook_deliveries (id, event_id, endpoint_id, url, response_excerpt,
                                payload_sha256, sent_at, responded_at, next_attempt_at)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
ON CONFLICT (event_id, endpoint_id) DO NOTHING
RETURNING id AS "id", event_id AS "event_id", ..., next_attempt_at AS "next_attempt_at"
```

`state`, `created_at` and `attempt` are absent from the INSERT list because
they carry `@default(...)` and `create_input_fields` filters them out — they
take their column defaults, exactly as the hand-written three-column INSERT
relied on. `status_code` is absent because the model does not declare it.
`attempt`/`status_code` are absent from the `RETURNING` for the same reason.

```text
UPDATE events SET fanout_state = $1 WHERE <filters> AND <update_policy>
RETURNING id AS "id", seq AS "seq", merchant_id AS "merchant_id", livemode AS "livemode",
          type AS "type", object_id AS "object_id", fanout_state AS "fanout_state",
          created_at AS "created_at"
```

Three things this render settles:

- **`<update_policy>` is in the `WHERE`.** That is the evidence for "a
  missing `@@allow("update", …)` on `model Event` is silent", not an
  inference from the docs. Mutation 4 confirms the behaviour.
- **`data` is in neither statement**, on the read side as well as the write
  side — the JSONB column never reaches `cratestack::Value` at all.
- `update_many` `RETURNING`s the model projection and `fetch_all`s it;
  `BatchSummary::ok` is `updated.len()` (`update_many_exec.rs:128`), which is
  the same number `rows_affected()` gave the statement it replaced.

`preview_sql` renders filters and the policy as the literal placeholders
`<filters>` / `<update_policy>` rather than expanding them, so the unit test
cannot assert the guard's *contents*; the container tests do that.

---

## 5. Mutations

Every one applied to a clean tree, run, and reverted; `git status` clean
afterwards each time.

| # | Mutation | Measured result |
|---|---|---|
| 1 | `create_in_tx`: `.run_in_tx(&mut *tx, &ctx)` -> `.run(&ctx)` | `an_abandoned_fan_out_leaves_no_delivery_and_the_event_still_pending` **FAILS in 2.3 s** — the delivery row survives the abandoned transaction. `a_committed_fan_out_keeps_both_cratestack_writes` still passes |
| 2 | `mark_fanned_out_in_tx`: the same swap | the same test **FAILS in 1.3 s** on the second assertion — `fanout_state` is `done` on an event whose deliveries were rolled back |
| 3 | Delete `@@allow("create", …)` from `model WebhookDelivery` | `every_action_this_module_calls_has_an_allow_arm` **FAILS with no container, in milliseconds**. Runtime: **5 of 6** delivery-touching container tests red — LOUD on every path that creates a delivery |
| 4 | Delete `@@allow("update", …)` from `model Event` | no-container test **FAILS**. Runtime: `a_committed_fan_out_keeps_both_cratestack_writes` and the abandon test fail; **`a_second_delivery_for_one_event_and_endpoint_is_not_created` still PASSES**, because it never reaches the flip. Silent in the error channel, total in effect |
| 5 | Delete `@@allow("update", …)` from `model WebhookDelivery` | no-container test **FAILS**. Runtime: **exactly one** container test red — `a_second_delivery_for_one_event_and_endpoint_is_not_created`. The first fan-out of an event succeeds; only a **re-run** fails. That is the crash-recovery path |
| 6 | Drop `CONSTRAINT type_is_a_documented_event` (migration 0029's re-add) | `an_undocumented_event_type_is_refused_by_the_database` **FAILS** (`PgQueryResult { rows_affected: 1 }` — the invented type was accepted). The drift report goes 101 -> 100 — and **the drift assertion FAILS on that too**, which this row originally denied. See §5b, corrected by the review |

### 5a. Neither `run_in_tx` mutation hangs, and that is not luck

exp17's equivalent mutation on the currency upsert did not fail — it
**hung** (`SLOW [>480.000s]`), because that transaction held `FOR UPDATE` on
the row the second connection's conflict probe then waited for.

Here both mutations fail loudly in about a second, and the reason is the lock
shape: this transaction takes no `FOR UPDATE` on a row it then asks a second
connection to probe. `do_nothing()`'s conflict probe finds no existing
delivery, so it locks nothing; the `events` row is held only by the
`FOR KEY SHARE` the delivery's foreign key takes, which does not conflict
with the `FOR NO KEY UPDATE` a non-key `UPDATE` wants.

So the two seams have *different* failure signatures under the same mistake,
and only measurement distinguishes them.

### 5b. Mutation 6 is the most important one in this file

`events.type_is_a_documented_event` is cited by migration `0018`'s own
comment ("the closure is enforced here rather than trusted to whoever writes
the emitting code later"), by `docs/api/README.md`, by
`docs/flows/webhooks.md` and by three Rust doc comments that call it "the
eight types `type_is_a_documented_event` allows".

**Nothing asserted it.** A grep for the constraint name across
`backends/tests` and `backends/crates/*/tests` returned nothing before this
commit. Four documents were resting on a constraint whose deletion no test
would have noticed.

That matters more now, because `model Event` cannot declare it: 0.11.1 has no
validator for "one of these eight strings" on a `String` column, and a
`.cstack` enum would match only under the generated name
`events_type_enum_check` while `diff/checks.rs` matches by name first.
So the constraint stays undeclared, and a dropped CHECK simply removes one
`[safe] … not declared in the schema` line and **lowers** the count:
101 -> 100, measured.

**Corrected by the review (see [opus-review.md](opus-review.md) F1).** This
section went on to claim that "every assertion in
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` [is]
still green" and that the new test "is the only thing that fails". That was
wrong, and it was the load-bearing sentence of this whole section.
`EXPECTED_DRIFT_CHANGES` is an exact `assert_eq!`, not a floor, so the drift
test fails on the *lower* count too:

```
assertion `left == right` failed: ... the report counts 100 pending change(s),
this test pins 101. ... If it shrank without an edit to the schema, find out
what the report stopped seeing before moving anything
  left: 100
 right: 101
```

The measurement recorded above (101 -> 100) was right; the inference drawn
from it — that the report is *structurally incapable* of complaining — was
not, and it understated a defence this repository already had.

`an_undocumented_event_type_is_refused_by_the_database` is still worth having,
for a reason that has to be stated differently: the count says *a constraint
is gone*, and cannot say which or whether it mattered; the test says *the
vocabulary is still closed*, which is what four documents rest on, and it is
the signal that survives someone re-pinning the constant. It also covers
`fanout_state_is_known`, the other multi-value CHECK on the same table and the
one the compare-and-swap depends on.

The hazard generalises: it applies to **every** `[safe] CHECK … exists in the
live database but is not declared in the schema` line in the report, of which
there are now many more than there were. Nothing in this repository runs
`cratestack migrate diff`, so it is latent rather than live.

---

## 6. Consequences elsewhere

- **`PendingTransaction` grew a field.** It carries a clone of the
  `Cratestack` handle beside its `Transaction<'static, Postgres>`, because a
  delegate borrows from that value and a `TxRepositories` method has no other
  reference in scope. `Cratestack` is `Clone` over an `Arc`-shaped
  `SqlxRuntime`, so no connection is added. The accessor returns
  `(&Cratestack, &mut Transaction)` in one call because the borrow checker
  will not allow two `&mut self` methods held across each other.
- **`PersistenceError::Invalid` is new, and it is a real fix rather than
  bookkeeping.** The generated `input.validate()` runs before any SQL, and
  `CratestackError::Validation` was falling into the `Backend` wildcard —
  `Category::Storage`, which is **retryable**. A 65-character endpoint id is
  exactly as long on every retry, so a worker would have retried it forever.
  It is `Category::Internal` / `Retry::Never` now, publishing the same `code`
  a CHECK violation does so a merchant cannot tell which layer refused.
- **Two existing tests changed their expected variant, and both got
  stronger rather than weaker.**
  `a_delivery_outside_the_length_checks_is_refused_by_the_database` now
  asserts *both* layers: the generated validator refusing non-retryably, and
  — via raw `sqlx` that goes around CrateStack entirely — the CHECKs
  themselves still firing in the database, which is what still binds `psql`
  and every future writer. `migration_0022_reopens_the_job_kinds_and_closes_the_delivery_states`
  asserts the FK violation arrives as `PersistenceError::ForeignKey` **and**
  that it classifies identically to the `DbError::ForeignKeyViolation` it
  replaced, compared against that path's own answer rather than a literal.
- **`crate::sql_audit` refused two of the new tests**, correctly: a `format!`
  interpolating a loop variable next to statement-shaped text is exactly what
  it scans for. Rewritten to slice the rendered column list and compare
  tokens, rather than adding an audit exception — exp17 §5 made the same
  call. The tokenisation also caught a real bug in the first draft:
  `next_attempt_at` contains the substring `attempt`, so a `contains` check
  was passing on the wrong column.

---

## 7. Not done

- **`insert_in_tx` did not move** (§1). It is the one write the brief named
  first.
- **`jobs` is untouched and is not next.** `Jobs::claim` needs `FOR UPDATE
  SKIP LOCKED`; `FindMany::for_update()` emits a bare `FOR UPDATE`. A lease
  mechanism that silently lost `SKIP LOCKED` would turn every worker's claim
  into a queue behind every other worker's. `jobs.payload` is `jsonb` and
  `jobs.attempts` is `int4` besides.
- **No money table moved.** `charges`, `payment_intents`, `ledger_*`,
  `refunds`, `settlement` are all untouched. `apply_succeeded` keeps its raw
  `UPDATE … RETURNING` with the `PREVIOUS_STATE` correlated sub-select, which
  no delegate expresses.
- **Every read on both tables stays raw `sqlx`.** `Events::{pending_page,
  list_page, get_by_id, get_unscoped}` all project `data`, and `list_page`'s
  cursor is a correlated sub-select (`seq < (SELECT seq FROM events WHERE id
  = $2 AND merchant_id = $1)`) with no delegate. `WebhookDeliveries::{get,
  pending_due, for_event, record_success, record_attempt}` read or write
  `attempt`/`status_code`, both `int4` and both undeclared.
- **`Events::record_fanout_failure` stays on the pool**, deliberately and
  unchanged: it counts the failure of a transaction that has rolled back.
- **No migration.** The count stays at 32. Three candidates were identified
  and all three left to the maintainer (§8).
- **The `@length` validators are app-layer only.** They do not create,
  rename or verify any database CHECK.
- Nothing about `cratestack` beyond the files cited above was verified.

---

## 8. Reserved for the maintainer

Three, none of them decided here.

1. **Whether to rename `type_is_a_documented_event` and
   `fanout_state_is_known`** to `events_type_enum_check` /
   `events_fanout_state_enum_check` in a migration, and declare them as
   `.cstack` enums. That would make them visible to the diff engine and close
   the §5b hazard for those two — at the cost of the vocabulary living in two
   places, and of exp17's finding 3 (a migration that alters a live table
   breaks the previous release's binary in a rolling deploy).
2. **Whether to `ALTER TABLE webhook_deliveries ALTER COLUMN id DROP
   DEFAULT`**, closing one drift line. Cheap and safe today — the column
   default has no remaining reader — but it is still a DDL change bought by a
   code generator's input-shaping rule, which is the trade `model Provider`'s
   GAP note reserves.
3. **Whether to widen `webhook_deliveries.attempt`/`status_code` and
   `events.fanout_attempts` to `BIGINT`**, the way 0032 widened
   `currencies.exponent`. It is what would let `record_attempt` and
   `record_success` move, and it carries exp17's measured rolling-deploy
   cost.
