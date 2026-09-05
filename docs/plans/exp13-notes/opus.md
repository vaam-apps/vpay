# exp13 (opus): the measured drift between `schemas/vpay.cstack` and the live database

**Date:** 2026-09-05 · **Branch:** `claude/exp13-baseline-drift-opus` · **Base:** `d086084`
**Tool:** `cratestack migrate baseline --strict`, `cratestack-cli` **0.11.1** (the
`cratestack_version` pin in `justfile`) · **Database:** `postgres:16-alpine`, the
tag `vpay_testkit::containers::start_postgres_with_retry` pins.

## Headline

`schemas/vpay.cstack` differs from the database `backends/migrations/*.sql`
builds by **86 pending drift changes across 17 tables/views**. That is now
asserted by
`backends/tests/integration/tests/postgres_smoke.rs::the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`,
which starts a testcontainers Postgres, runs all 30 migrations with
`sqlx::migrate!`, shells out to the CLI and parses the report.

**And the headline finding is a negative one.** The two cross-column CHECK
constraints this repository holds up as the reason it needs `@@check(expr)` —
`providers.partial_refunds_imply_refunds` and `payment_intents.no_over_refund` —
**do not appear in the drift report at all.** Not as "declared in the schema but
missing from the database", not as "exists in the live database but is not
declared", not in any form. The task brief expected them to show up as things
the schema cannot express; they do not show up.

## The rule, measured

Every CHECK constraint `migrate baseline` reports is a **single-column** one.
Every **multi-column** CHECK in the database is invisible to it, in both
directions. Read straight out of `pg_constraint` on the live database
(`cardinality(conkey) > 1`), there are exactly ten:

| Table | Constraint | Columns | In the report? |
|---|---|---|---|
| `checkout_sessions` | `urls_match_ui_mode` | 4 | no |
| `idempotency_keys` | `complete_has_a_response` | 4 | no |
| `payment_intents` | `no_over_refund` | 3 | no |
| `jobs` | `lock_is_paired` | 2 | no |
| `oauth_signing_keys` | `active_key_has_no_expiry` | 2 | no |
| `oauth_signing_keys` | `expiry_after_creation` | 2 | no |
| `payment_intents` | `lpe_paired` | 2 | no |
| `provider_requests` | `response_is_paired` | 2 | no |
| `providers` | `partial_refunds_imply_refunds` | 2 | no |
| `refunds` | `failure_paired` | 2 | no |

Ten multi-column CHECKs, zero reported. Meanwhile single-column CHECKs on the
**same two modelled tables** are reported one line away:

```
providers:
  [safe] CHECK `code_length` exists in the live database but is not declared in the schema
  [safe] CHECK `display_name_length` exists in the live database but is not declared in the schema
```

so the absence is not "the tool skipped `providers`". `providers` and
`payment_intents` are both modelled in `schemas/vpay.cstack`; eight of the ten
sit on tables the schema does not model at all, but those two do not.

### Why this matters more than the `@@check` ask

`--strict`'s documented purpose is "proving in CI that a database already
matches the schema exactly"
(https://cratestack.dev/tooling/migrate-baseline). On this database, a
**green `--strict` run would say nothing whatever about the over-refund guard or
the refund-capability coherence rule** — the two constraints the migrations
added precisely *because* the grammar could not express them are exactly the two
a clean drift report cannot vouch for. Anyone who later wires `migrate baseline
--strict` in as a schema gate should know that before trusting it.

This is the evidence for the `@@check(expr)` ask, and it is stronger evidence
than the report naming them would have been: the grammar cannot express them
*and* the drift tool cannot see them, so nothing in the CrateStack toolchain
would ever notice their absence.

## How the report represents the things it *can* see

`Destructiveness`-derived severities, as documented: `[safe]`, `[lossy]`,
`[blocking]`. Some observations worth recording, since they explain the shape of
the 86:

- **A Postgres enum column introspects as `Scalar("String")`.** Every enum-typed
  column the schema models is reported as a type difference —
  `column `status` type differs (live: Scalar("String"), schema:
  Enum("IntentStatus"))` — for `status`, `flow`, `state`, `failure_code`,
  `account`, `direction`. Six of the 86. The schema is not wrong here; the
  introspection does not round-trip a `CREATE TYPE ... AS ENUM`.
- **A synthetic CHECK name is minted for an enum column the schema does not
  declare**: `payment_intents_last_payment_error_code_enum_check`. There is no
  such constraint in `pg_constraint`; `last_payment_error_code` is of type
  `failure_code`.
- **`@default(dbgenerated())` never matches a concrete Postgres default.** Every
  column carrying it reports `default value differs from the schema`
  (`charges.created_at`, `payment_intents.created_at`,
  `ledger_transactions.created_at`, `ledger_transactions.id`,
  `ledger_entries.id`). Measured directly: switching a model's `DateTime
  @default(dbgenerated())` to `@default(now())` against a `DEFAULT now()` column
  makes the difference disappear. `@default(dbgenerated("now()"))` is rejected —
  "cratestack's `dbgenerated()` takes no argument".
- **18 columns are excluded from the comparison entirely.** The report's own
  trailing block: `18 column(s) have a Postgres type cratestack could not
  confidently map to a `.cstack` scalar — excluded from the comparison above,
  review manually`. Every one is `jsonb`, `int2`/`int4` or `bytea`. This is a
  blind spot *in the measurement*, so the test pins it too: if it grows, the 86
  can fall for a reason that has nothing to do with the schema improving.
- **The `authkestra.*` tables are invisible**, because baseline introspects the
  connection's own schema and they live in `authkestra`. `disabled_clients`,
  `oauth_signing_keys` and `oauth_client_assertion_jtis` are *not* in that
  category — they are `public` tables, so the schema header's "and the
  authkestra tables" does not account for them. Measuring rather than copying
  the header's list is what surfaced that.

## The tables the migrations build and the schema does not declare

Eleven, measured, sorted — a **larger** set than `schemas/vpay.cstack`'s header
claims to omit:

```
_sqlx_migrations              (sqlx's own bookkeeping; not a vpay design object)
checkout_sessions
disabled_clients              ] not covered by the header's
oauth_client_assertion_jtis   ] "and the authkestra tables" —
oauth_signing_keys            ] these three are `public`
events
idempotency_keys
jobs
provider_requests
refunds
webhook_deliveries
```

## `--strict` wrote nothing

Asserted, not assumed. The out-dir (a `std::env::temp_dir()` path outside the
checkout) is created empty and is still empty afterwards; no `migrations/`
directory appears at the repository root, so the `--out-dir` flag was honoured
rather than the default used; `schemas/vpay.cstack` is byte-identical after the
run. Separately, after four `--strict` runs against the same database,
`information_schema.tables` had **no `cratestack_migrations` table** — the
synthetic baseline row really is not recorded. Exit code 1, and stderr:

```
Error: migrate baseline: --strict refuses to baseline with 86 pending drift change(s); resolve the drift above (or drop --strict) and try again. No snapshot was written and no baseline row was recorded.
```

## Full CLI output

Verbatim, from the run the test itself performed (`cratestack CLI under test:
0.11.1`):

```text
drift detected in 17 table(s)/view(s) (86 change(s) total):

_sqlx_migrations:
  [lossy] table `_sqlx_migrations` exists in the live database but is not declared in the schema

charges:
  [safe] CHECK `amount_non_negative` exists in the live database but is not declared in the schema
  [safe] CHECK `failure_raw_length` exists in the live database but is not declared in the schema
  [safe] CHECK `id_length` exists in the live database but is not declared in the schema
  [safe] CHECK `payment_intent_id_length` exists in the live database but is not declared in the schema
  [safe] CHECK `provider_txn_id_length` exists in the live database but is not declared in the schema
  [safe] CHECK `redirect_url_is_a_bounded_web_url` exists in the live database but is not declared in the schema
  [safe] CHECK `return_url_is_a_web_url` exists in the live database but is not declared in the schema
  [safe] CHECK `return_url_length` exists in the live database but is not declared in the schema
  [safe] index `charges_live_idx` exists in the live database but is not declared in the schema
  [safe] index `charges_provider_reference_idx` exists in the live database but is not declared in the schema
  [safe] index `one_charge_per_intent` exists in the live database but is not declared in the schema
  [lossy] column `provider_txn_id` exists in the live database but is not declared in the schema
  [lossy] column `return_url` exists in the live database but is not declared in the schema
  [lossy] column `updated_at` exists in the live database but is not declared in the schema
  [safe] column `provider_ref_extra` is declared in the schema but does not exist in the live database
  [safe] column `created_at` default value differs from the schema
  [lossy] column `failure_code` type differs (live: Scalar("String"), schema: Enum("FailureCode"))
  [lossy] column `state` type differs (live: Scalar("String"), schema: Enum("ChargeState"))
  [safe] index `charges_payment_intent_id_key` is declared in the schema but does not exist in the live database
  [blocking] CHECK `charges_amount_range_check` is declared in the schema but does not exist in the live database
  [safe] foreign key `charges_payment_intent_id_fkey` is declared in the schema but does not exist in the live database
  [safe] foreign key `charges_provider_code_fkey` is declared in the schema but does not exist in the live database
  [safe] foreign key `charges_currency_code_fkey` is declared in the schema but does not exist in the live database

checkout_sessions:
  [lossy] table `checkout_sessions` exists in the live database but is not declared in the schema

currencies:
  [safe] CHECK `code_is_iso4217_shape` exists in the live database but is not declared in the schema
  [safe] CHECK `exponent_in_range` exists in the live database but is not declared in the schema
  [blocking] column `exponent` is declared in the schema but does not exist in the live database
  [blocking] CHECK `currencies_code_iso4217_check` is declared in the schema but does not exist in the live database
  [blocking] CHECK `currencies_exponent_range_check` is declared in the schema but does not exist in the live database

disabled_clients:
  [lossy] table `disabled_clients` exists in the live database but is not declared in the schema

events:
  [lossy] table `events` exists in the live database but is not declared in the schema

idempotency_keys:
  [lossy] table `idempotency_keys` exists in the live database but is not declared in the schema

jobs:
  [lossy] table `jobs` exists in the live database but is not declared in the schema

ledger_entries:
  [safe] CHECK `amount_non_negative` exists in the live database but is not declared in the schema
  [safe] index `ledger_entries_transaction_id_idx` exists in the live database but is not declared in the schema
  [lossy] column `account` type differs (live: Scalar("String"), schema: Enum("AccountKind"))
  [lossy] column `direction` type differs (live: Scalar("String"), schema: Enum("Direction"))
  [lossy] column `id` type differs (live: Scalar("String"), schema: Scalar("Cuid"))
  [safe] column `id` default value differs from the schema
  [lossy] column `transaction_id` type differs (live: Scalar("String"), schema: Scalar("Cuid"))
  [blocking] CHECK `ledger_entries_amount_range_check` is declared in the schema but does not exist in the live database
  [safe] foreign key `ledger_entries_transaction_id_fkey` is declared in the schema but does not exist in the live database
  [safe] foreign key `ledger_entries_currency_code_fkey` is declared in the schema but does not exist in the live database

ledger_transactions:
  [safe] column `created_at` default value differs from the schema
  [lossy] column `id` type differs (live: Scalar("String"), schema: Scalar("Cuid"))
  [safe] column `id` default value differs from the schema
  [safe] foreign key `ledger_transactions_charge_id_fkey` is declared in the schema but does not exist in the live database

oauth_client_assertion_jtis:
  [lossy] table `oauth_client_assertion_jtis` exists in the live database but is not declared in the schema

oauth_signing_keys:
  [lossy] table `oauth_signing_keys` exists in the live database but is not declared in the schema

payment_intents:
  [safe] CHECK `amount_non_negative` exists in the live database but is not declared in the schema
  [safe] CHECK `amount_received_non_negative` exists in the live database but is not declared in the schema
  [safe] CHECK `amount_refund_pending_non_negative` exists in the live database but is not declared in the schema
  [safe] CHECK `amount_refunded_non_negative` exists in the live database but is not declared in the schema
  [safe] CHECK `client_secret_suffix_length` exists in the live database but is not declared in the schema
  [safe] CHECK `description_length` exists in the live database but is not declared in the schema
  [safe] CHECK `id_length` exists in the live database but is not declared in the schema
  [safe] CHECK `lpe_message_length` exists in the live database but is not declared in the schema
  [safe] CHECK `merchant_id_length` exists in the live database but is not declared in the schema
  [safe] CHECK `metadata_is_object` exists in the live database but is not declared in the schema
  [safe] CHECK `pmt_is_array` exists in the live database but is not declared in the schema
  [safe] CHECK `payment_intents_last_payment_error_code_enum_check` exists in the live database but is not declared in the schema
  [safe] index `payment_intents_merchant_seq_idx` exists in the live database but is not declared in the schema
  [safe] index `payment_intents_seq_key` exists in the live database but is not declared in the schema
  [lossy] column `client_secret_suffix` exists in the live database but is not declared in the schema
  [lossy] column `description` exists in the live database but is not declared in the schema
  [lossy] column `last_payment_error_code` exists in the live database but is not declared in the schema
  [lossy] column `last_payment_error_message` exists in the live database but is not declared in the schema
  [lossy] column `seq` exists in the live database but is not declared in the schema
  [lossy] column `updated_at` exists in the live database but is not declared in the schema
  [safe] column `last_payment_error` is declared in the schema but does not exist in the live database
  [blocking] column `payment_method_types` is declared in the schema but does not exist in the live database
  [safe] column `created_at` default value differs from the schema
  [lossy] column `status` type differs (live: Scalar("String"), schema: Enum("IntentStatus"))
  [blocking] CHECK `payment_intents_amount_range_check` is declared in the schema but does not exist in the live database
  [blocking] CHECK `payment_intents_amount_received_range_check` is declared in the schema but does not exist in the live database
  [blocking] CHECK `payment_intents_amount_refund_pending_range_check` is declared in the schema but does not exist in the live database
  [blocking] CHECK `payment_intents_amount_refunded_range_check` is declared in the schema but does not exist in the live database
  [safe] foreign key `payment_intents_currency_code_fkey` is declared in the schema but does not exist in the live database

provider_requests:
  [lossy] table `provider_requests` exists in the live database but is not declared in the schema

providers:
  [safe] CHECK `code_length` exists in the live database but is not declared in the schema
  [safe] CHECK `display_name_length` exists in the live database but is not declared in the schema
  [lossy] column `flow` type differs (live: Scalar("String"), schema: Enum("ProviderFlow"))
  [blocking] CHECK `providers_code_length_check` is declared in the schema but does not exist in the live database

refunds:
  [lossy] table `refunds` exists in the live database but is not declared in the schema

webhook_deliveries:
  [lossy] table `webhook_deliveries` exists in the live database but is not declared in the schema

18 column(s) have a Postgres type cratestack could not confidently map to a `.cstack` scalar — excluded from the comparison above, review manually:
  _sqlx_migrations.checksum: bytea
  charges.provider_ref_extra: jsonb
  currencies.exponent: int4
  events.data: jsonb
  events.fanout_attempts: int4
  idempotency_keys.request_hash: bytea
  idempotency_keys.response_status: int2
  idempotency_keys.response_body: jsonb
  jobs.payload: jsonb
  jobs.attempts: int4
  oauth_signing_keys.public_jwk: jsonb
  payment_intents.payment_method_types: jsonb
  payment_intents.metadata: jsonb
  provider_requests.attempt: int4
  provider_requests.status_code: int4
  refunds.metadata: jsonb
  webhook_deliveries.attempt: int4
  webhook_deliveries.status_code: int4

Error: migrate baseline: --strict refuses to baseline with 86 pending drift change(s); resolve the drift above (or drop --strict) and try again. No snapshot was written and no baseline row was recorded.
```

## Mutations

Three, each reverted; `git status` clean afterwards.

### 1. Delete `CONSTRAINT no_over_refund` from `backends/migrations/0003_create-payment-intents.sql`

The brief predicted both the existing constraint test and the drift assertion
would fail. **As first written, only the constraint test failed** — and that is
the finding, not a defect in the mutation:

```
FAIL [  23.870s] (1/2) vpay-tests-integration::postgres_smoke over_refund_is_rejected_by_the_database
  amount_refunded + amount_refund_pending > amount must be rejected: PgQueryResult { rows_affected: 1 }
PASS [   3.577s] (2/2) vpay-tests-integration::postgres_smoke the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount
```

Re-run with `--no-capture` to be sure it was a real run and not a fluke: the
full report printed, `drift detected in 17 table(s)/view(s) (86 change(s)
total)`, `no_over_refund` mentioned nowhere,
`amount_refund_pending_non_negative` (single-column, same table) still listed.
**The count does not move when the over-refund guard is deleted, because the
tool cannot see it.**

That exposed a real weakness in the test as first drafted:
`assert!(!stdout.contains("no_over_refund"))` is satisfied *both* when the
report is blind and when the constraint has been deleted. So the test was
changed to read `pg_constraint` first and assert the full set of ten
multi-column CHECKs exists in the live database, and only then that none of
them reaches the report. With that in place the same mutation fails both:

```
Summary [  11.452s] 2 tests run: 0 passed, 2 failed, 162 skipped
  FAIL over_refund_is_rejected_by_the_database
  FAIL the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount

assertion `left == right` failed: the multi-column CHECK constraints backends/migrations builds. …
  left:  [… ("payment_intents", "lpe_paired"), ("provider_requests", "response_is_paired"), …]
  right: [… ("payment_intents", "lpe_paired"), ("payment_intents", "no_over_refund"), ("provider_requests", "response_is_paired"), …]
```

Note what the *same* failing run still printed: `--strict refuses to baseline
with 86 pending drift change(s)`. The count is unmoved. The `pg_constraint`
assertion is what notices.

### 2. Add a model for a table that already exists

**2a — `DisabledClient` with `disabled_at DateTime @default(dbgenerated())`.**
The count **did not move**: still `drift detected in 17 table(s)/view(s) (86
change(s) total)`. The table's one change was swapped for another —

```
disabled_clients:
  [lossy] table `disabled_clients` exists in the live database but is not declared in the schema
```
became
```
disabled_clients:
  [safe] column `disabled_at` default value differs from the schema
```

— so a whole table entering the schema was invisible to the change count. The
test failed anyway, on the exact-table-set assertion:

```
assertion `left == right` failed: the set of tables the migrations build and the schema does not declare
  left:  ["_sqlx_migrations", "checkout_sessions", "events", …]
  right: ["_sqlx_migrations", "checkout_sessions", "disabled_clients", "events", …]
```

This is why the test asserts the set and not only the number, and the comment
above that assertion now says so.

**2b — the same model with `@default(now())`,** which matches the column's real
`DEFAULT now()` exactly. `disabled_clients` then drops out of the report
altogether and the count moves as the brief predicted:

```
drift detected in 16 table(s)/view(s) (85 change(s) total):
…
assertion `left == right` failed: the drift between schemas/vpay.cstack and backends/migrations changed: the report counts 85 pending change(s), this test pins 86. …
  left: 85
  right: 86
```

`just check-schema` passed with the extra model in place both times, so the
mutation was a legal schema, not a broken one.

## Timings

Measured on a host running three other agents' container suites concurrently.

| Step | Wall clock |
|---|---|
| `cargo build --tests -p vpay-tests-integration` (cold) | 1m 04s |
| The new test, first run (cold container image state) | 42.99s |
| The new test, warm | 1.4s – 2.0s |
| `migrate baseline --strict` itself, by hand | well under 1s |

## What this does not establish

- **Nothing about `authkestra.*`.** Those tables are in another Postgres schema
  and baseline never looked at them.
- **Nothing about the 18 unmapped columns.** Their drift, whatever it is, is
  unmeasured — the tool excluded them and the test only pins how many there are.
- **The schema is still excluded from the build graph and drives no migration.**
  Measuring the gap does not close it and does not wire the file into anything.
