# exp34 — the money tables through CrateStack (S5)

Working notes for migration `0037`, the four money models, and the four
queries that moved. Written as evidence, not as a summary: every number here
was produced by running something, and the things that did **not** work are
recorded beside the things that did.

Branch `claude/exp34-money-tables`, base `889d045`. CrateStack CLI 0.12.0
installed into a private `--root` (the shared `~/.cargo/bin` had 0.11.1 and
was left alone, because another agent was using it).

---

## 1. The headline, stated before the evidence

**Four queries moved. All four are reads. All four are on
`checkout_sessions`. Nothing at all moved on `payment_intents`, `charges` or
`refunds`.**

The plan (§4 S5) expected a swap across all four tables. It is not one, and
the reason is a single measured blocker rather than a list of small ones:
every read and every write on those three tables returns a row struct that
carries a `jsonb` column, and `Value::from_plain_json` demotes any non-`i64`
number to `f64` (`cratestack-core-0.12.0/src/value.rs:95-106`). On
`payment_intents.metadata` and `refunds.metadata` that JSON is
**merchant-authored**, so it is the column the conversion must not be lossy
on.

The brief also asked for "refunds' create/get" to move. **There is no refund
create.** `vpay_db::refunds` is two reads and no write, because
`ProviderAdapter::refund` is `NotImplemented` on MTN and `Unsupported` on
Orange. Both reads are additionally merchant-scoped through a JOIN onto
`payment_intents`, which a generated read cannot express.

---

## 2. Migration 0037, and three things measuring it taught

### 2.1 A partial index blocks the `ALTER` outright

```
$ ALTER TABLE charges ALTER COLUMN state TYPE TEXT USING state::TEXT;
ERROR:  operator does not exist: text = charge_state
HINT:  No operator matches the given name and argument types.
```

`charges_live_idx` (migration 0014) is
`ON charges (state) WHERE state IN ('submitting','submitted','pending','unresolved')`,
and Postgres will not alter a column a **partial** index predicate depends on.
It is dropped and rebuilt in `0037`. `charges_state_idx`, a plain index on the
same column, is rebuilt automatically and needs nothing — both halves measured
in the same psql session.

### 2.2 Applied to a populated database, every label survives

A database migrated to `0035`, seeded with one row per label of every one of
the four types (5 `intent_status`, 5 `charge_state`, 4 `refund_status`, 11
`failure_code` spread across three tables), then `0037` applied:

```
 status                  | pg_typeof | last_payment_error_code
-------------------------+-----------+-------------------------
 canceled                | text      | provider_error
 processing              | text      | payer_declined
 requires_action         | text      | payer_timeout
 requires_payment_method | text      | insufficient_funds
 succeeded               | text      | invalid_payer

 SELECT typname FROM pg_type WHERE typname IN
   ('intent_status','charge_state','refund_status','failure_code');
 (0 rows)
```

`DROP TYPE` without `CASCADE`, so a dependency the migration did not find
fails it loudly rather than silently dropping whatever depended on it.

### 2.3 A nullable enum column takes a bare `IN (...)`

The first draft wrote `CHECK (failure_code IS NULL OR failure_code IN (...))`,
which is redundant (`NULL IN (…)` is NULL, and a CHECK fails only on FALSE)
**and wrong for drift**: it deparses to
`failure_code IS NULL OR (failure_code = ANY (ARRAY[…]))`, which
`introspect/postgres/check_pattern.rs::reconstruct_enum` cannot read back — it
matches `<column> = ANY (ARRAY[…])` and nothing else. The constraint would
have introspected as `CheckKind::Raw` and diffed against the generated
`CheckKind::Enum` as a kind mismatch. Corrected before the migration was run
against anything. `emit/postgres/checks.rs` relies on the same fact and says
so in its own comment.

### 2.4 The conversion moves the drift count by ZERO

| Schema                     | Database           | Report                                 |
| -------------------------- | ------------------ | -------------------------------------- |
| base `schemas/vpay.cstack` | migrated to `0035` | **130** / 20 relations / 18 unmappable |
| base `schemas/vpay.cstack` | migrated to `0037` | **130** / 20 / 18                      |

Exactly what `docs/reference/vpay-db.md` § "The enum conversion no report can
see" predicts. `introspect/postgres/enums.rs` was already synthesising an
`AddCheck { name: "payment_intents_last_payment_error_code_enum_check", kind: Enum }`
out of `pg_enum` for the native column — the same name `0037` then created for
real, which is visible in the before-report as a
`[safe] CHECK payment_intents_last_payment_error_code_enum_check exists in the
live database but is not declared` line on a database where no such constraint
existed.

(The counts above include `_sqlx_migrations`, which `sqlx::migrate!` creates
and a hand-run `psql` loop does not. Measuring by hand gives 129/19/17; the
container-backed test gives 130/20/18. Worth knowing before comparing a
hand-run report against the constant.)

---

## 3. Drift after the four models: 130 → 141

Per table, before and after, same database, same CLI:

| table               | before    | after     |
| ------------------- | --------- | --------- |
| `payment_intents`   | 31        | 19        |
| `charges`           | 23        | 16        |
| `checkout_sessions` | 1         | 21        |
| `refunds`           | 1         | 11        |
| everything else     | unchanged | unchanged |

`EXPECTED_DRIFTED_RELATIONS` does not move (20). `EXPECTED_UNMAPPABLE_COLUMNS`
does not move (18) — which is the prediction "an undeclared `jsonb` column is
invisible in both directions" tested rather than repeated.

The two falls are the report describing **rot**: `model PaymentIntent` still
declared `last_payment_error`, a column migration 0014 dropped, and neither it
nor `model Charge` knew about seven columns added since. After the rewrite, no
`column … is declared in the schema but does not exist` and no
`column … exists in the live database but is not declared` line survives on
any of the four tables.

The 67 remaining lines are five kinds — 37 hand-named CHECKs, 12 undeclared
indexes, 6 permanent enum `type differs`, 2 identity `seq` defaults, and 10
foreign keys.

### The fifth upstream gap, found here

**Ten of those lines are false.** The report says ten foreign keys are
"declared in the schema but do not exist in the live database"; all ten exist
and `psql` shows them. `cratestack-migrate-0.12.0/src/introspect/postgres/mod.rs`
documents it under "Known gaps":

> **Foreign keys are not introspected.** Neither the issue nor the design
> doc's §5.2 query list mentions `pg_constraint`'s `contype = 'f'` rows, so
> `TableProjection::foreign_keys` is always empty here. A table with
> `.cstack`-declared relations will show every foreign key as "missing" drift
> until a follow-up phase adds this.

Added to the upstream-gap table in `docs/reference/vpay-db.md` as the fifth
row. The relations are kept: they are true, `cratestack-parser` requires both
sides, and deleting a true declaration to make a false report line disappear
would be optimising the constant instead of the schema.

---

## 4. Mutations

Every mutation was applied to the committed tree, the named test run, and the
tree restored with `git checkout --`. Two rounds, because the first round had
a scaffolding bug and one genuine finding.

| #   | Mutation                                                 | Test                                                                            | Result                             |
| --- | -------------------------------------------------------- | ------------------------------------------------------------------------------- | ---------------------------------- |
| M1  | `get_for_merchant` loses `.where_(merchant_id)`          | `a_session_read_for_the_wrong_merchant_is_indistinguishable_from_a_missing_one` | **RED**                            |
| M2  | `find_open_by_intent` loses `.where_(status = 'open')`   | `the_open_session_read_filters_by_status_and_the_latest_read_orders_by_seq`     | **RED**                            |
| M3  | `latest_by_intent_query` loses `.order_by(seq().desc())` | the container test above                                                        | **GREEN — not caught** (see below) |
| M3′ | same mutation, after the fix                             | `the_latest_session_query_orders_by_seq_and_takes_one`                          | **RED**, ~1 ms, no container       |
| M4  | `model CheckoutSession` loses `@@allow("read", …)`       | the unit test **and** both container tests                                      | **RED** (all three)                |
| M5  | migration 0003 loses `CONSTRAINT no_over_refund`         | `over_refund_is_rejected_by_the_database`                                       | **RED**                            |

### M3 is the finding worth carrying

Deleting the `ORDER BY` from `find_latest_by_intent` left a container test
that seeds two sessions and asserts **which** one comes back **green**. The
live `checkout_sessions_intent_seq_idx` is `(payment_intent_id, seq DESC)`, so
Postgres answers an unordered `LIMIT 1` out of that index in seq-descending
order anyway. The behaviour was right by accident — dependent on a planner
choice, on that index continuing to exist, and on the row count staying small.

Fixed by extracting the chain into `latest_by_intent_query` so the test can
preview **the same builder the query runs**, and asserting the rendered SQL.
A test that rebuilt the chain itself would have proved nothing. The container
test's doc comment now says plainly that it does not catch this, rather than
continuing to claim it does.

### M5 was a scaffolding bug before it was a result

The first run reported M5 GREEN. It was not: the script's `str.replace` looked
for `",\n    CONSTRAINT no_over_refund"` and the two are separated by a 30-line
comment block, so **the edit silently did not apply** and the test ran against
an unmutated tree. `git diff --stat` was logged before each run and M5's was
empty, which is how it was caught. Re-run with a correct edit, the mutation is
RED — and it fails **twice**, which is more than the brief expected:

```
over_refund_is_rejected_by_the_database
  amount_refunded + amount_refund_pending > amount must be rejected:
  PgQueryResult { rows_affected: 1 }

the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount
  the multi-column CHECK constraints backends/migrations builds ...
  left:  [... ("payment_intents", "lpe_paired"), ("provider_requests", ...
  right: [... ("payment_intents", "lpe_paired"),
          ("payment_intents", "no_over_refund"), ("provider_requests", ...
```

The **count** was unmoved, as the brief predicted — what failed in the drift
test is the multi-column-CHECK list read from `pg_constraint`, the safety net
`docs/reference/vpay-db.md` records for exactly this case. So the invisible
constraint has two independent guards, not one.

---

## 5. Decisive test 2 (`@@audit`): NOT DONE, and why

The brief's decisive test 2 was to enable `@@audit` on `model Charge`, install
a recording `AuditSink`, delete the `dispatch_audit_sink` call from
`UnitOfWork::transaction`'s Commit arm, and confirm a test fails — with the
stopping rule "if it cannot be made to fail, do not enable `@@audit`".

**It was not attempted, and `@@audit` is not enabled.** The reason is upstream
of the stopping rule: `descriptor.audit_enabled` is consulted by CrateStack's
generated **writes**, and there is no CrateStack write on `charges` — or on any
money table — to audit. Enabling `@@audit` on `model Charge` would arm
`ensure_audit_table` for a code path that does not exist, and the test could
not be made to fail because nothing would ever dispatch. Enabling it on `model
CheckoutSession`, the one model with live queries, would be no better: all four
are reads, and the audit hook is on the write path.

This is a "not applicable yet" rather than a "could not make it fail", and the
difference matters: the decisive test becomes runnable the day a write on a
money table moves, which is the day `create` moves and needs its own migration.

---

## 6. Maintainer decisions surfaced, not taken

1. **The expand/contract alternative for `0037`.** Adding a `TEXT` column
   beside each enum, dual-writing for a release and dropping the enum a
   release later removes the deploy-ordering constraint entirely. Not taken:
   it would put a second column claiming to be the state of a charge in the
   table for the length of a release, which is a worse hazard than an ordering
   rule. 0032 recorded the same open question and it stays open.
2. **Whether `CheckoutSessions::create` should move**, which needs `0037` to
   `DROP DEFAULT` on `checkout_sessions.created_at` and `updated_at`. That is
   0033's trade on a money table, and 0033's argument ("the default was never
   doing anything for the only writer there is") does not carry over — here the
   default is what `create` relies on for `updated_at`.
3. **Whether to keep declaring relations** while the upstream FK gap is open.
   Kept, with the reasoning above; the alternative buys 10 drift lines and
   costs a true statement about the database.
4. **`account_kind` and `direction`** — the two native enums left, on the
   ledger tables. Not converted: no CrateStack query touches those tables and
   converting a money column's type buys nothing until one does.

---

## 7. What is not claimed

- No **write** on any money table runs through CrateStack. The settlement
  `UPDATE … RETURNING` with its correlated sub-select, the confirm path's
  two-row transaction, and every `jobs` insert are the raw `sqlx` they were.
- `jobs`, `idempotency_keys` and `provider_requests` are untouched and are
  **out of scope forever unless a reason appears** — recorded in
  `docs/status.md` in those words so neither list reads as a gap.
- `just test-e2e`, `just demo-up`/`demo-walk` and `just ci`'s web half were
  run as recorded in the final report; anything not recorded there was not run.
