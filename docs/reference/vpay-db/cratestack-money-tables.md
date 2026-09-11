# `vpay-db` — CrateStack: the money tables

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

### The money tables: what moved, and what stays raw forever

S5 (2026-09-07) is the last rung of the CrateStack ladder and the one the
whole adoption was pointed at: `payment_intents`, `charges`, `refunds` and
`checkout_sessions`. It landed **migration 0037**, four models, and **four
queries** — all four on `checkout_sessions`, and none at all on the other
three tables. That asymmetry is the finding, so it is stated before anything
else.

#### One blocker decides three tables, and it is `jsonb`

Every read and every write in `vpay_db::payment_intents`, `vpay_db::charges`
and `vpay_db::refunds` returns a row struct — `PaymentIntentRow`,
`ChargeRow`, `RefundRow` — and each of those carries a `jsonb` column:

| Table             | Column(s)                          | What is in them                                                       |
| ----------------- | ---------------------------------- | --------------------------------------------------------------------- |
| `payment_intents` | `payment_method_types`, `metadata` | a JSON array of rail codes, and **merchant-authored** key/value pairs |
| `charges`         | `provider_ref_extra`               | rail key material (Orange's `pay_token`)                              |
| `refunds`         | `metadata`                         | merchant-authored, as above                                           |

`Value::from_plain_json` routes every JSON number through `Number::as_i64()`
with an `as_f64().unwrap_or_default()` fallback
(`cratestack-core-0.12.0/src/value.rs:95-106`), so a `u64` above `i64::MAX`
anywhere inside a merchant's `metadata` comes back as a float. That is the
same argument `events.data` has carried since 2026-09-06, and on `metadata`
it is stronger rather than weaker: `events.data` is at least vpay-shaped
around a merchant payload, while `metadata` **is** the merchant payload.

So the models do not declare those columns, and a generated read that omitted
them would answer a struct the repository cannot build. The two ways around
that were both considered and both rejected:

- **A second statement for the `jsonb` columns.** Two round trips and a
  consistency window, on the money path, to replace one statement that works.
- **Declaring `metadata Json`.** Strictly worse for drift, in the direction
  the report cannot help with: an _undeclared_ `jsonb` column is invisible to
  the comparison in both directions, so it costs nothing; a _declared_ one
  adds a `[blocking] column metadata is declared in the schema but does not
exist in the live database` line while leaving the live column just as
  invisible. `EXPECTED_UNMAPPABLE_COLUMNS` did not move at all across S5,
  which is that prediction tested.

**Nothing on those three tables moved, and nothing will until upstream maps
`jsonb` in both directions.** The three models carry no `@@allow` arm at all
— deny-by-default — because an arm for a call site that does not exist is a
standing permission nobody asked for.

`refunds` has a second, independent blocker worth recording because the brief
that produced this work assumed otherwise: **there is no refund create to
move.** `vpay_db::refunds` is two reads and no write, because
`ProviderAdapter::refund` is `NotImplemented` on MTN and `Unsupported` on
Orange (`docs/status.md`). Both reads are also merchant-scoped through a JOIN
onto `payment_intents` — the table carries no `merchant_id` of its own, and
migration 0017 argues why it should not — and a generated read filters
columns of one table.

#### `checkout_sessions` moved because migration 0028 gave it nothing to trip on

No `jsonb`, no `bytea`, no native enum. Four of its nine repository methods
are now `find_unique`/`find_many` calls, and `EXPECTED_ASSERT_SITES` in
`sql_audit.rs` falls 45 → 41 for the first time in its history. The other five
did not move, and each has its own reason:

| Statement                                                     | Why it stays raw                                                                                                                                                                                                                                                  |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `list_page`                                                   | the cursor is a correlated sub-select (`seq < (SELECT seq FROM checkout_sessions WHERE id = $2 AND merchant_id = $1)`); no delegate expresses it, exactly as for `Events::list_page`                                                                              |
| `expire`, `due_for_expiry`, `expire_due`, `settle_for_intent` | each carries `NOT EXISTS (SELECT 1 FROM charges …)` over a **second** table, and a generated builder filters columns of one. That guard is the statement's whole point: it is what stops a session being expired out from under a payer who has already confirmed |
| `create`                                                      | expressible, and deliberately deferred — see below                                                                                                                                                                                                                |

**`create` is the one that could have moved and did not.** Moving it needs
migration 0037 to `DROP DEFAULT` on `created_at` and `updated_at`, because
`cratestack-macros` filters every `@default(...)` field out of
`Create{Model}Input` (`model/inputs.rs:20-23`) and the writer supplies
`created_at` today. That is migration 0033's trade applied to a money table,
and 0033's own argument does not carry over cleanly: on `providers` the
defaults were doing nothing for the only writer there was, whereas here
`updated_at`'s default is what `create` relies on. It is one migration and
one struct field away, and it is written down rather than done, because a
column default dropped on a money table is a `23502` waiting for the writer
that forgets. `model CheckoutSession` has no `@@allow("create", …)` arm, and
`every_action_this_module_calls_has_an_allow_arm` asserts that slot is
**empty** — so adding one is a deliberate edit rather than a silent one.

**One of the four reads is pinned on its rendered SQL rather than on its
behaviour, and the reason is a measurement.** `find_latest_by_intent` asks for
`ORDER BY seq DESC LIMIT 1`. Deleting that `order_by` left a container test
that seeds two sessions and asserts _which_ one comes back **green**: the live
`checkout_sessions_intent_seq_idx` is `(payment_intent_id, seq DESC)`, so
Postgres answers an unordered `LIMIT 1` out of that index in seq-descending
order anyway. The behaviour was right by accident — dependent on a planner
choice, on that index continuing to exist, and on the row count staying small.
The chain therefore lives in `latest_by_intent_query` so that
`the_latest_session_query_orders_by_seq_and_takes_one` can preview **the same
builder the query runs**; a test that rebuilt the chain itself would assert
the generator back to itself and catch nothing.

#### Migration 0037, and the two things it deliberately did not do

It converts the last four native enums on a money table — `intent_status`,
`charge_state`, `refund_status`, and the `failure_code` shared by
`charges.failure_code`, `payment_intents.last_payment_error_code` and
`refunds.failure_code` — to `TEXT` plus a `<table>_<column>_enum_check`.
`account_kind` and `direction`, on the ledger tables, are the two that remain.

Three measurements from writing it, none of them guessable from the source:

1. **A partial index blocks the `ALTER` outright.** `charges_live_idx` is
   `ON charges (state) WHERE state IN (…four labels…)`, and
   `ALTER COLUMN state TYPE TEXT USING state::TEXT` fails with
   `ERROR: operator does not exist: text = charge_state`. The migration drops
   and rebuilds it. `charges_state_idx`, a plain index on the same column, is
   rebuilt by Postgres on its own and needs nothing.
2. **The drift count moved by zero**: 130 before, 130 after, measured against
   the same freshly migrated database with only the migration applied. That is
   what "The enum conversion no report can see" predicts, and this is the
   second table set to confirm it. `introspect/postgres/enums.rs` had already
   been synthesising an `AddCheck { name:
"payment_intents_last_payment_error_code_enum_check", kind: Enum }` out of
   `pg_enum` for the native column — the _same name_ migration 0037 then
   created for real, which is visible in the before-report as a `[safe] CHECK
… exists in the live database but is not declared` line on a database where
   no such constraint existed.
3. **A nullable enum column takes a bare `IN (...)`, not `IS NULL OR … IN
(...)`.** `NULL IN (…)` is NULL and a CHECK fails only on FALSE, so the
   NULL case needs no spelling out — and spelling it out would deparse to
   something `reconstruct_enum` cannot read back (it matches
   `<column> = ANY (ARRAY[…])` and nothing else), turning a `CheckKind::Enum`
   into a `CheckKind::Raw` and a converged constraint into a kind mismatch.
   The generator relies on the same fact and says so
   (`emit/postgres/checks.rs`: "NULL passes … Nullable enum columns therefore
   need no special casing").

What it did **not** do, both deliberate:

- **It drops no column default.** 0033's argument applies to a column a
  CrateStack write must name, and no write on these four tables moved.
  `PaymentIntents::insert` does not name `amount_received`, `amount_refunded`,
  `amount_refund_pending` or `updated_at`; dropping those defaults would turn
  every intent creation into a `23502` and buy nothing. The models declare the
  defaults instead, so the two sides agree and the columns cost no drift.
- **It renames no hand-written CHECK.** `checkout_sessions`' three closed
  vocabularies were never native enums — migration 0028 created them as `TEXT`
  plus `ui_mode_is_known`, `status_is_known` and `payment_status_is_known` —
  and a rename moves the count by zero (0032's measurement). The model
  declares those three columns as plain `String` rather than `.cstack` enums,
  so there is no generated name to converge on, and `vpay-db` carries all
  three as `String` regardless.

#### One behaviour changed, and it is not visible in any signature

`PaymentIntents::transition` and `Settlement::set_live_state` both take an
`expected` label. It used to be bound as `$3::intent_status` /
`$2::charge_state`, so a label outside the vocabulary was a Postgres error and
reached a caller as a `500`. It is a plain `TEXT` comparison now, so such a
label matches no row and answers `Ok(None)` / `Ok(false)` — the same answer a
genuine lost race gives, which a caller turns into a `409`.

Every caller passes a label from `vpay_core`'s state machine, so the
difference is between two spellings of "a vpay bug". It is recorded because a
`409` is quieter than a `500` and nothing else in the system would say so. The
`new` label is unaffected: outside the vocabulary it is a `23514` rather than a
`22P02`, and `classify_write` leaves both as `DbError::Query`.

#### What the four models cost, and the ten report lines that are false

Drift moves **130 → 141**, per table: `payment_intents` 31 → 19, `charges`
23 → 16, `checkout_sessions` 1 → 21, `refunds` 1 → 11, and nothing else by a
line. `EXPECTED_DRIFTED_RELATIONS` does not move and neither does
`EXPECTED_UNMAPPABLE_COLUMNS`.

The two falls are the report describing rot rather than drift: `model
PaymentIntent` still declared `last_payment_error`, a column migration 0014
**dropped**, and neither it nor `model Charge` knew about `seq`,
`description`, `customer_id`, `client_secret_suffix`, `provider_txn_id`,
`return_url` or `updated_at`. After the rewrite, **not one `column … is
declared in the schema but does not exist` and not one `column … exists in the
live database but is not declared` line survives on any of the four tables.**

The two rises are `customers`' lesson repeated: a column's marginal drift
depends on whether its table is modelled at all, and a table nobody declares
collapses to one line however wrong it is.

The 67 lines the four tables carry are five kinds, and every one is a kind
0.12.0 structurally cannot close — 37 hand-named CHECKs, 12 undeclared
indexes, 6 permanent enum `type differs`, 2 identity-column `seq` defaults,
and **10 foreign keys that the report says do not exist and that all do**.

That last group is the fifth upstream gap in the table at the top of this
section, and it is documented by the tool rather than inferred from its
output: `introspect/postgres/mod.rs`'s own "Known gaps" says "**Foreign keys
are not introspected.** … so `TableProjection::foreign_keys` is always empty
here. A table with `.cstack`-declared relations will show every foreign key as
'missing' drift until a follow-up phase adds this."

The relations are declared anyway. All ten constraints exist,
`cratestack-parser` requires both sides of a relation to be declared, and
removing a true declaration to make a false report line disappear would be
optimising `EXPECTED_DRIFT_CHANGES` instead of the schema — the exact move
that constant's own assertion message warns about. What it leaves is the
mirror of the undeclared-CHECK hazard recorded above: a generated
`migrate diff` would emit `ADD CONSTRAINT … FOREIGN KEY` for ten constraints
that already exist, and Postgres would refuse each one. Nothing runs
`migrate diff` in this repository, so it is latent rather than live.

#### Three tables are out of scope forever unless a reason appears

`jobs`, `idempotency_keys` and `provider_requests` keep working raw `sqlx`
implementations and are **not** pending work. `docs/status.md` says so in the
same words, so that neither list reads as a gap:

- **`jobs`** — `Jobs::claim` needs `FOR UPDATE SKIP LOCKED` and
  `FindMany::for_update()` emits a bare `FOR UPDATE`. A lease that silently
  lost `SKIP LOCKED` would turn every worker's claim into a queue behind every
  other worker's. `jobs.payload` is `jsonb` and `jobs.attempts` is `int4`
  besides.
- **`idempotency_keys`** — the merchant-scoped `claim_id` semantics have no
  delegate equivalent, and three of its columns (`request_hash` `bytea`,
  `response_status` `int2`, `response_body` `jsonb`) are unmapped types.
- **`provider_requests`** — `latest_submit_attempt` orders `sent_at DESC, id
DESC` and pairs `status_code`/`responded_at`; both `attempt` and
  `status_code` are `int4`.
