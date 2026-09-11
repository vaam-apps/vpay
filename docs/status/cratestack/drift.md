# CrateStack — the measured drift

_Archived from [docs/status.md](../../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../../` because the file moved two directories down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](../README.md) is the index of them._

#### The measured drift (2026-09-05; **101 / 16 since `model Event` and `model WebhookDelivery`, 2026-09-06**)

**`schemas/vpay.cstack` differs from the database `backends/migrations/*.sql`
builds by 101 pending drift changes across 16 tables/views** — 86 across 17
when first measured on 2026-09-05, 85 across 16 once `model DisabledClient`
landed, 84 across 16 once migration 0032 widened `currencies.exponent`.
Measured, not estimated: `cratestack migrate baseline --strict` at the pinned
CLI 0.12.0 (0.11.1 through 2026-09-06; re-derived at 0.12.0 on 2026-09-07 and
unchanged), against a `postgres:16-alpine` testcontainer with all 32
migrations applied by `sqlx::migrate!`. **It went UP for the first time on
2026-09-06 — 84 -> 101 — and that is the good direction here**, because the
17 new lines are `events` and `webhook_deliveries` being compared column by
column instead of being dismissed as one `table … is not declared` line each.
See "The outbox through CrateStack" below for the line-by-line accounting.
It is asserted by
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` in
`backends/tests/integration/tests/postgres_smoke.rs` — the exact count, the
exact sixteen relations, and the exact ten tables the report names as
present in the database and absent from the schema. The full transcripts are
in [docs/plans/exp13-notes/opus.md](../../plans/exp13-notes/opus.md) (the original
measurement) and [docs/plans/exp14-notes/opus.md](../../plans/exp14-notes/opus.md)
(the move).

**Why it moved, and the near-miss worth recording.** `model DisabledClient`
landed on 2026-09-06 for the existing `disabled_clients` table, and that
table left the report entirely — one `table … is not declared in the schema`
line went away with nothing replacing it. That outcome depended on a single
attribute. With `disabled_at DateTime @default(dbgenerated())`, `migrate
baseline` reads the live default as `ColumnDefault::Function("now()")` and
the schema's as `ColumnDefault::DbGenerated`, which never compare equal — so
the missing-table line is _swapped_ for a `column disabled_at default value
differs` line and **the total stays at exactly 86**. A whole table entering
the schema would have been invisible to the count. `@default(now())`
converts to the same `Function("now()")` and compares clean. Both spellings
were run; the exact-set assertion beside the count is what would have caught
the first, and is the reason that assertion exists.

**85 -> 84 on 2026-09-06, and two of migration 0032's three changes moved
nothing.** This is the entry worth reading before planning the next table,
because the intuition it corrects is the obvious one.

| Migration 0032 change                                                                  | Drift effect                                                              |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `currencies.exponent` `INT` -> `BIGINT`                                                | **-1 change, and -1 in the "could not confidently map" block (18 -> 17)** |
| The two hand-named `currencies` CHECKs renamed to `<table>_<column>_<validator>_check` | **0**                                                                     |
| `providers.flow` native enum -> `TEXT` + `providers_flow_enum_check`                   | **0**                                                                     |

The widening is the whole of the -1, and it is the _good_ kind: the column was
excluded from the comparison entirely (`Int` emits `int8` and the introspector
deliberately refuses to map `int4` back onto it), so the report is now
comparing **more** and finding no drift on what it gained.

The **rename moved nothing** because introspection reports every
validator-derived CHECK as `CheckKind::Raw(<deparsed text>)` and reconstructs
only `CheckKind::Enum` — never `Iso4217`, `Range` or `Length`. `diff/checks.rs`
matches by name and then compares kinds, so a matching name turns two
unrelated lines into a same-named drop-and-add pair: a clearer report, the
same number. The rename is still right (the database now carries the name a
generated `migrate diff` would emit DDL against, and doing it later means
doing it on a table with rows), but **do not expect a CHECK rename to move
this constant.**

The **enum conversion moved nothing, and the report is structurally blind to
it** — the mirror image of the multi-column-CHECK finding below.
`introspect/postgres/enums.rs` already synthesised `providers_flow_enum_check`
from `pg_enum` for the native column, and `resolve_column` projects a native
enum and a `TEXT` column onto the same `Scalar("String")`, so `providers`
reports the identical four lines before and after. The conversion is real and
load-bearing all the same: CrateStack's generated row decoders read an enum
column with `try_get::<String>()`, so a native enum column fails to decode on
**every** read through that layer, and no CrateStack query could have touched
`providers` before it. What proves it worked is
`a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx` in
`vpay-db`'s own tests, and reverting the `ALTER COLUMN flow TYPE TEXT` is what
makes that test red — measured, in
[docs/plans/exp17-notes/opus.md](../../plans/exp17-notes/opus.md). One of the four
`providers` lines, `column flow type differs (live: Scalar("String"), schema:
Enum("ProviderFlow"))`, is **permanent at 0.12.0**: the enum's _name_ has no
catalog representation to recover it from, which `enums.rs`'s own doc comment
calls documented lossiness. Every enum-typed column in the schema carries one.

Until this test, "content remains a design sketch" was a sentence written from
reading both files, and nothing ran that could have contradicted it. **The
number must move when the schema grows**, and it must never be asserted as 0:
this file still drives no migration, so a 0 would mean the report stopped
finding things rather than that the gap closed. `--strict` writes nothing —
the out-dir is outside the checkout and is asserted empty, no
`cratestack_migrations` row is recorded, and the schema file is byte-identical
afterwards.

**The two cross-column CHECKs do not appear in the drift report at all.** This
is the finding, and it strengthens the `@@check(expr)` ask above rather than
merely illustrating it. Every CHECK `migrate baseline` reports is a
single-column one; read out of `pg_constraint`, the live database has **ten**
multi-column CHECKs — including `providers.partial_refunds_imply_refunds` and
`payment_intents.no_over_refund` — and **none of the ten reaches the report**,
in either direction. Both of those two sit on tables the schema _does_ model,
and single-column CHECKs on those same tables are reported one line away, so
the absence is not the tables being skipped. The consequence worth writing
down: `--strict`'s documented purpose is proving in CI that a database already
matches the schema, and **on this database a green `--strict` run would say
nothing whatever about the over-refund guard or the refund-capability rule** —
the two constraints raw SQL added _because_ the grammar could not express them
are exactly the two the drift tool cannot vouch for. Deleting `CONSTRAINT
no_over_refund` from migration 0003 leaves the count where it was, measured;
the test
catches it by reading `pg_constraint` directly, and
`over_refund_is_rejected_by_the_database` catches it behaviourally.

**It is CrateStack's own documented gap, not an inference from ten samples**
(added 2026-09-05 by review, from the pinned crate's sources rather than from
the measurement alone). `cratestack-migrate-0.12.0/src/introspect/postgres/
mod.rs` lists it under "Known gaps": _"Multi-column and zero-column CHECK
constraints are skipped. `crate::ir::AddCheck` ties to exactly one column —
there's no IR shape for `CHECK (a < b)` — so `constraints::introspect_checks`
only considers `contype = 'c'` rows with `array_length(conkey, 1) = 1`;
anything else is silently absent from the result rather than mis-attributed to
one of its columns."_ The query in `constraints.rs` carries that filter
verbatim. Three things follow that the black-box measurement could not have
told us:

- **The ask is bigger than `@@check(expr)`.** Grammar alone would not fix
  this: the IR has no shape a cross-column CHECK could occupy, so an
  `@@check(expr)` that parsed would still be invisible to `migrate baseline`
  and `migrate diff`. The ask to CrateStack is `@@check(expr)` **and** an IR
  op that is not tied to a single column **and** introspection that reads
  `array_length(conkey, 1) > 1`.
- **The skip is deliberate and defensible**, which is worth saying plainly:
  mis-attributing `CHECK (a < b)` to column `a` would be worse. The defect is
  that a `--strict` run reports success without saying what it could not
  look at — the report's trailing "N column(s) … review manually" block has
  no CHECK equivalent.
- **Zero-column CHECKs are skipped too**, and the test's `pg_constraint`
  query (`cardinality(conkey) > 1`) does not enumerate them. Measured on this
  database: there are none on a `public` table — the only two,
  `cardinal_number_domain_check` and `yes_or_no_check`, are
  `information_schema` domain constraints — so nothing is missed today, and a
  future `CHECK (current_setting(…) = 'x')`-shaped constraint would be
  invisible to both the tool and that assertion.

Two further blind spots, both pinned by the same test so they cannot move
unnoticed: **18 columns are excluded from the comparison entirely** (`jsonb`,
`int2`/`int4`, `bytea` — the report says so itself and asks for a manual
review), and the `authkestra.*` tables are never introspected, because baseline
reads the connection's own schema. `oauth_signing_keys` and
`oauth_client_assertion_jtis` are **not** in that second category — they are
`public` tables, so the schema header's "and the authkestra tables" does not
account for them, and the measured set of undeclared tables is larger than the
header claims. Measuring rather than copying that list is what surfaced it.
`disabled_clients` was a third until 2026-09-06, when it became the first
table this file models _and_ `vpay-db` reads through CrateStack.

What was **not** done, as of the 2026-09-05 measurement: `cratestack migrate
diff` was still never run, no snapshot was ever written, nothing was
reconciled, and none of the 86 changes was closed. **One of them has been
closed since** — `disabled_clients` — and `migrate diff` is still never run,
no snapshot is still ever written, and the remaining 85 stand.
