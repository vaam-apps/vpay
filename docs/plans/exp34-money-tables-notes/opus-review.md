# exp34 review — the money tables (S5), sabotage pass

Adversarial review of `claude/exp34-money-tables` at `faa8f7a` (base
`889d045`), 2026-09-07/08. Written the way
[opus.md](opus.md) is: every number below came from running something, and
the things that did **not** work are recorded beside the things that did.

Everything ran in `/home/selast/dev/vpay/.claude/worktrees/exp34-money-tables`
with `CARGO_BUILD_JOBS=4`, the `.nvmrc` Node (22.23.2), and the CrateStack
0.12.0 CLI from a private `--root` (the shared `~/.cargo/bin` carries 0.11.1
and was left alone). Containers ran on the rootless daemon; the user's
`vpay-demo` compose project was never touched.

---

## Verdict

**Not safe as delivered.** One finding is a false completion on the money
path — a guard the branch's own commit message, notes, `status.md` and
`reference/vpay-db.md` all describe, which did not exist in the tree — and
three more are operator-facing claims that measurement contradicts. The
design is sound and the migration is correct; the gap is between what the
branch says it checks and what it checks.

Seven commits fix every confirmed finding. Three of the four proofs the
implementer could not give are now given in full; the fourth — `just test-e2e`
as a recipe — is blocked by issue #78 on a port the user's own stack holds,
and its specs were run another way, all eleven passing, with the single
deviation stated.

---

## Findings

### 1. `latest_by_intent_query` and its test did not exist — GATE-HOLE (money path)

`faa8f7a`'s message says, in its own words:

> - The chain moves into `latest_by_intent_query`, so the test can preview
>   **the same builder the query runs**. […]
> - `the_latest_session_query_orders_by_seq_and_takes_one` asserts the
>   rendered SQL […] Red in ~1 ms under the mutation, no container.

`git show --stat faa8f7a` lists five files and
`backends/crates/vpay-db/src/checkout_sessions.rs` is not one of them.
Neither the function nor the test was ever written. They appear in prose in
five places — the commit message, [opus.md](opus.md)'s mutation table (M3′),
`docs/status.md`, `docs/reference/vpay-db.md`, and the doc comment of the
container test in `vpay-db/tests/repositories.rs`.

That last one is the damaging one. The implementer measured that deleting
`.order_by(checkout_session::seq().desc())` leaves that container test
**green** (`checkout_sessions_intent_seq_idx` is `(payment_intent_id, seq
DESC)`, so Postgres answers an unordered `LIMIT 1` in seq-descending order
anyway), corrected the test's doc comment to say so, pointed it at the
replacement — and did not write the replacement. Net effect: the branch
_documented away_ the only guard on the `ORDER BY` and shipped nothing in its
place. `find_latest_by_intent` is what the confirm path uses to decide which
session is current.

Fixed in `04bbf1e`, which lands what the message described plus a second test
that asserts the read policy reaches the statement.

### 2. "The reads are equally affected" — WRONG, and operator-facing

0037's header said the previous binary's reads break too. They do not.
`status::TEXT AS status` casts to a **built-in** type this migration does not
drop; the cast is a no-op, not a reference to a vanished type. Measured on
889d045's own `vpay-db` against a 0037 database:
`PaymentIntents::get_for_merchant` answered `Some("requires_payment_method")`
normally.

This points the opposite way from reassurance. During the rolling-deploy
window the system does **not** look broken: every `GET` answers, dashboards
render, and only the statements that move money fail. An operator told to
expect reads to go dark is waiting for a signal that never comes.

### 3. "unlike 0032 … this one breaks the money path at request time" — HALF TRUE

Measured, same harness. A previous-release process that **restarts** never
reaches a request:

```
migration 37 was previously applied but is missing in the resolved migrations
  -> Migrate(VersionMissing(37))
```

`Migrator::validate_applied_migrations` refuses any applied version the binary
does not carry, and `run_migrations` never sets `ignore_missing`
(`sqlx-core-0.9.0/src/migrate/migrator.rs:363-380`). That is boot, before
step 4 — which is what 0032 does on a restart too, so the contrast the
sentence draws is not the real difference. The real difference is the
**in-flight** window, which 0032 did not have.

Findings 2 and 3 fixed in `6f7d93a` (header, `status.md`, `crash-safety.md`).

`0032`'s own header attributes its restart failure to "boot step 4"; on the
evidence above it would fail one step earlier, at `run_migrations`. **Left
alone** — pre-existing, out of this branch's scope, and worth a maintainer's
glance rather than a drive-by edit.

### 4. Three claims about 0037 had no test — GATE-HOLE

The brief asked for the first of these in so many words ("Applied to a
populated database **in a test**"), and migration 0033 already had exactly
that test to copy. All three were done by hand in `psql` and recorded in
[opus.md](opus.md); none landed.

- a populated database survives the conversion;
- the six CHECKs that replace four dropped types actually refuse;
- the dropped-and-rebuilt partial index is still the one the recovery sweep
  plans against.

Fixed in `0c7ca86`. The second test's first doc comment claimed to be the
only guard; measured, the drift test catches both of its mutations too, and
the comment now says so and says what the test adds instead.

### 5. Deny-by-default was a claim, not an assertion — CORRECTNESS

`model PaymentIntent`, `model Charge` and `model Refund` carry no `@@allow`
arm, and the branch says repeatedly that this is deny-by-default and
deliberate. Nothing asserted it. Adding `@@allow("read", auth() != null)` to
`model PaymentIntent` — a two-line diff to a schema file, a standing read
permission over every merchant's intents — left `cargo build`, `just clippy`,
`just check-schema` and all twelve `just verify` gates **green** (measured).

The same for the blocker underneath: declaring `metadata Json` on `model
Refund` was green everywhere too, and that column is merchant-authored JSON
that `Value::from_plain_json` demotes.

Fixed in `b859ef3`, in `schema.rs` because the property spans three modules
and two of them have no test module at all.

### 6. Five sentences 0037 falsified — MISLEADING-CLAIM

`vpay-api/src/model.rs` still described `refunds.status` as a Postgres `ENUM`
in four places — and that description is the whole argument for
`RefundObject::status` being typed rather than a `String`.
`docs/reference/vpay-core.md` still called `intent_status` a Postgres enum.
Fixed in `65024cd`; historical plan notes (`docs/plans/issue-46-notes/`) left
as the dated records they are.

### 7. A table that was not a table — NIT

The four `checkout_sessions` rows S5 added to `reference/vpay-db.md`'s "What
runs through it today" were separated from it by a blank line, so they were a
second block with no header and no delimiter row: four lines of literal pipes.
Fixed in `65024cd`.

### 8. `charge_state` has six labels, not five — NIT

[opus.md](opus.md) §2.2 says the populated-database check seeded "5
`intent_status`, 5 `charge_state`, 4 `refund_status`, 11 `failure_code`".
`charge_state` carries **six** (`submitting`, `submitted`, `pending`,
`unresolved`, `succeeded`, `failed`) and 0037's CHECK correctly lists all six.
Recorded here rather than edited into the implementer's own notes; the new
test seeds all six.

### 9. `LIVE_CHARGE_STATES`' doc names the wrong index — OBSERVATION, pre-existing

The constant says the four labels are "exactly the set the partial index
`charges_live_idx` … is built over, so the `NOT EXISTS` in
`PaymentIntents::cancel` is an index lookup". It is an index lookup — but
`EXPLAIN` shows it served by `one_charge_per_intent`, not by
`charges_live_idx`, because the correlated sub-query leads on
`payment_intent_id`:

```
->  Index Scan using one_charge_per_intent on charges
      Index Cond: (payment_intent_id = …)
      Filter: (state = ANY ('{submitting,submitted,pending,unresolved}'::text[]))
```

`charges_live_idx` is genuinely reached by
`Settlement::live_charges_stale_since`, which filters on `state` alone — and
that is what the new EXPLAIN test pins. The sentence predates this branch and
was **not** edited.

---

## The four proofs

### `just ci`, end to end, clean

**Given.** Exit 0 on `faa8f7a` before any fix, with nothing else running:

```
verify: ok — the twelve gates above passed
Summary [899.742s] 1568 tests run: 1568 passed, 0 skipped
```

Doctests: 0 failed, 1 ignored (`sdks/rust/src/lib.rs - ReadmeDoctests`,
pre-existing). Web: 1116 vitest cases across nine packages, all passing.
`a_valid_config_lets_the_server_boot_and_serve_healthz` passed in that run, so
the failure [opus.md](opus.md) records at 1248/1568 did not reproduce; it was
load, not this branch. The second full run, on the final head, is in the
report.

### `just test-e2e`

**The recipe itself: not run.** `compose.e2e.yml` publishes the dashboard on a
hard-coded `3000:3000` and `justfile`'s `test-e2e` probes
`http://localhost:3000/`, also hard-coded — issue #78. The user's own
`vpay-demo` project held `127.0.0.1:3000` for the whole of this review
(polled, still held after a 15-minute wait). Running the recipe would have
required stopping the user's stack or editing the recipe, and neither is a
reviewer's to do.

**The specs: run, and all of them pass.** One deviation, stated so it can be
discounted: the dashboard was republished on `13000` by a one-service compose
override kept in the scratchpad, and `VPAY_DASHBOARD_URL` was set to match —
Cypress' own `baseUrl` is already `VPAY_DASHBOARD_URL ?? http://localhost:3000`,
so nothing else changed. Everything after that is the recipe's own command
(`pnpm --filter @vpay/e2e e2e`) with the recipe's own environment, on compose
project `exp34-review` with the branch's images built from this head:

```
✔  checkout.cy.ts       1  passing
✔  dashboard.cy.ts      3  passing
✔  shop-hosted.cy.ts    3  passing
✔  shop-embedded.cy.ts  4  passing   (the framed run, its own cypress run)
   11 passing, 0 failing, 0 pending, 0 skipped, exit 0
```

And from that stack's database afterwards: `payment_intents.status` 5
`succeeded` / 5 `requires_payment_method`, `charges.state` 5 `succeeded` / 1
`failed`, `checkout_sessions.status` 4 `complete` / 1 `expired` / 3 `open`,
and **0** of the four enum types alive. Browser-driven payments settling
through 0037's TEXT columns.

**The first attempt failed, and the reason was mine.** `checkout.cy.ts` and
`shop-hosted.cy.ts` failed with `invalid_client: Client authentication
failed`; the server logged `InvalidAudience`. Cause: a bare `just
gen-demo-keys` regenerated `.e2e/application-demo.yml` with
`public_base_url: http://localhost:8080` — the recipe bakes `demo_port` into
the overlay and I had not passed the override to _that_ invocation, only to
`demo-up`. Regenerated with `just demo_port=18080 … gen-demo-keys`, stack
recreated, all four specs green. Recorded because a first-attempt failure on
the money path that is silently dropped from a report is exactly the thing
this review exists to find.

### `just demo-up` + `just demo-walk`

**Given**, on compose project `exp34-review` with ports 18080/18082/18083/
13080/13001, the user's `vpay-demo` untouched, torn down with `demo-down`
afterwards.

```
✔ all six steps behaved as expected — 6 payments on 2 rails, every one settled
  by the worker asking the rail and evidenced by a signed webhook
```

And from that stack's own database, which is the point:

|                                  |                                                                 |
| -------------------------------- | --------------------------------------------------------------- |
| `charges.state`                  | `succeeded` 2, `failed` 4                                       |
| `payment_intents.status`         | `succeeded` 2, `requires_payment_method` 6                      |
| `events.type`                    | `payment_intent.succeeded` 2, `payment_intent.payment_failed` 4 |
| the four columns' `format_type`  | `text`, `text`, `text`, `text`                                  |
| surviving enum types of the four | **0**                                                           |

Six payments settled through 0037's TEXT columns, against real rails over
HTTP.

### 0037 on a populated database with every label

**Given twice** — by hand first, then as
`migration_0037_keeps_every_stored_label_on_a_populated_database`. By hand: 21
rows covering 5 `intent_status`, 6 `charge_state`, 4 `refund_status` and all
11 `failure_code` labels across the three tables that carry one; `diff` of the
before/after snapshots empty; all four types gone; both indexes back.

---

## The money invariants after the conversion

| Invariant                                                         | How it was checked                                                                                        | Result                                                                                                                                                |
| ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| No `sqlx::Type` derive or `type_name` for the four enums survives | `git grep` for `sqlx::Type` / `sqlx(type_name` across `backends/`                                         | none exist, and none ever did — the vocabularies were always carried as `String` (D4)                                                                 |
| No statement still names a dropped type                           | `git grep` of every `::intent_status`/`::charge_state`/`::refund_status`/`::failure_code`                 | 32 removed, 0 remaining in any statement; the 10 remaining occurrences are prose describing the history                                               |
| Each dropped cast is semantics-preserving                         | every statement diffed before/after; suites that cover them re-run                                        | the 32 enum casts and 10 `::TEXT` casts are no-ops on a `TEXT` column; `AS <col>` aliases dropped only where the bare column already carries the name |
| **Ordering** did not change                                       | `git grep` for `ORDER BY`/`MIN`/`MAX`/`<`/`>` on the six columns                                          | none. A native enum orders by declaration order and `TEXT` orders alphabetically — nothing in vpay depends on either                                  |
| A value outside each CHECK is refused                             | `every_enum_check_0037_created_refuses_a_value_outside_it`, six columns, SQLSTATE **and** constraint name | all six `23514`, each by its own constraint                                                                                                           |
| Every label vpay writes is still accepted                         | same test, the other direction                                                                            | all 6 + 5 + 11 + 4 accepted                                                                                                                           |
| The partial index still filters what it filtered                  | `EXPLAIN` on `Settlement::live_charges_stale_since` with `enable_seqscan=off`                             | `Index Scan using charges_live_idx on charges`                                                                                                        |

---

## Drift, re-measured from scratch

Hand-run (no `_sqlx_migrations`, so one relation and one line below the
container figures, exactly as [opus.md](opus.md) predicts):

| schema                        | database                    | report       |
| ----------------------------- | --------------------------- | ------------ |
| base `vpay.cstack` @`889d045` | migrated to 0035            | **129** / 19 |
| base `vpay.cstack` @`889d045` | migrated to 0035 **+ 0037** | **129** / 19 |
| branch `vpay.cstack`          | migrated to 0035 + 0037     | **140** / 19 |

**"0037 alone moves drift by zero" is confirmed**, and more strongly than
claimed: the two reports are identical but for the ordering of two lines. The
before-report does carry
`[safe] CHECK payment_intents_last_payment_error_code_enum_check exists in the
live database but is not declared` on a database where no such constraint
existed — the synthesised line [opus.md](opus.md) §2.4 describes.

Per table, before and after, exactly as the notes and
`EXPECTED_DRIFT_CHANGES`' comment claim:

| table               | before                   | after |
| ------------------- | ------------------------ | ----- |
| `payment_intents`   | 31                       | 19    |
| `charges`           | 23                       | 16    |
| `checkout_sessions` | 1                        | 21    |
| `refunds`           | 1                        | 11    |
| every other table   | unchanged, line for line |       |

The 67 lines on the four tables break down exactly as claimed: **37** CHECKs,
**12** indexes, **6** `type differs`, **2** `seq` default differs, **10**
foreign keys.

---

## The foreign-key claim, and the decision

**Confirmed, twice over.** The tool documents it —
`cratestack-migrate-0.12.0/src/introspect/postgres/mod.rs:22-27`, "Known
gaps": _"Foreign keys are not introspected … `TableProjection::foreign_keys`
is always empty here"_ — and `mod.rs:109` is the literal
`foreign_keys: Vec::new()`. And the database disagrees with the report: all
ten named constraints exist (`SELECT conname FROM pg_constraint WHERE
contype = 'f'` returns all ten).

**Decision: keep the relations.** The alternative — deleting ten true
`@relation` declarations so a false report line disappears — optimises
`EXPECTED_DRIFT_CHANGES` instead of the schema, which is the move that
constant's own assertion message warns about; the declarations are true
statements about the database; and `cratestack-parser` requires both sides of
a relation, so the back-references cannot be trimmed independently. The brief
asked that, if kept, the drift test's accounting comment say the ten are false
and why: **it already does**, in `EXPECTED_DRIFT_CHANGES`' doc comment, naming
the upstream file and quoting its "Known gaps". Nothing to add.

The latent hazard the notes record is real and unchanged: a generated `migrate
diff` would emit `ADD CONSTRAINT … FOREIGN KEY` for ten constraints that
exist. Nothing in this repository runs `migrate diff`.

---

## The JSON blocker

**Confirmed.** `cratestack-core-0.12.0/src/value.rs:99-102` routes every JSON
number through `Number::as_i64()` and falls back to
`as_f64().unwrap_or_default()`. Pinned on this branch by
`no_generated_read_on_a_money_table_can_carry_its_jsonb_column`, which asserts
both halves: the generated projections omit `metadata`,
`payment_method_types` and `provider_ref_extra`, **and** the round trip still
loses a number outside `i64`. The second half is the one that matters — an
upstream `map_scalar` fix would make the first assertion pass while merchant
`metadata` was still being demoted.

**The three models really do answer zero rows.** `push_allow_policy_query`
emits the literal `FALSE` for an empty allow list
(`cratestack-sqlx-0.12.0/src/query/support/policy.rs:52-55`), and
`the_three_money_models_answer_no_rows_to_every_action` asserts it of the
rendered statement under `system_context()`, not only of the descriptor.

---

## `@@audit`

The implementer's reasoning is **accepted**: `descriptor.audit_enabled` is
consulted by generated _writes_, there is no CrateStack write on any money
table, so the decisive test could not be made to fail for a reason that is
about applicability rather than about effort. Enabling it would arm
`ensure_audit_table` for a path that does not exist. Re-checked against
`cratestack-sqlx-0.12.0`'s write paths; nothing to change.

---

## Mutations

Every mutation was applied to the committed tree, the named test run, and the
tree restored (`git diff --stat` checked empty afterwards, because a `git
checkout --` during this review did once wipe an uncommitted fix — the lesson
is to commit before mutating, and it is why every fix below is its own commit).

| #    | Mutation                                                      | Test                                                                | Result                                                                                              |
| ---- | ------------------------------------------------------------- | ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| M3   | `latest_by_intent_query` loses `.order_by(seq().desc())`      | `the_latest_session_query_orders_by_seq_and_takes_one`              | **RED**, no container, message names the confirm path                                               |
| M-A  | 0037 loses `charges_state_enum_check`                         | `every_enum_check_0037_created_refuses_a_value_outside_it`          | **RED**                                                                                             |
| M-A′ | same                                                          | `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` | **RED** too — measured, and the reason the new test's doc comment does not claim to be the only net |
| M-B  | 0037 loses `DROP INDEX charges_live_idx`                      | `migration_0037_keeps_every_stored_label_on_a_populated_database`   | **RED**: `operator does not exist: text = charge_state`                                             |
| M-C  | one label mistyped inside `charges_state_enum_check`          | both of the above                                                   | **RED** in both                                                                                     |
| M-D  | `model PaymentIntent` grows `@@allow("read", auth() != null)` | `the_three_money_models_answer_no_rows_to_every_action`             | **RED**                                                                                             |
| M-E  | `model Refund` declares `metadata Json`                       | `no_generated_read_on_a_money_table_can_carry_its_jsonb_column`     | **RED**                                                                                             |

The implementer's own M1, M2, M4 and M5 were not re-run: their subjects
(`get_for_merchant`'s merchant predicate, `find_open_by_intent`'s status
predicate, `CheckoutSession`'s read arm, `no_over_refund`) are unchanged by
this review and the tests that refuse them are unchanged too. M3′ could not be
re-run because it did not exist; it does now.

**The abandon/transaction property is moot for these four moves**, and should
be said rather than left as a silent omission: all four are reads, all four
ran on `self.pool` before the swap and run on the CrateStack runtime's pool
after it, and none is called while a `for_update` lock is held. There is no
`run_in_tx` variant to lose, so the "run it on `runtime.pool()` instead" family
of mutations has no subject here. It acquires one the day a _write_ on a money
table moves.
