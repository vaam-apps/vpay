-- Drop the column defaults on `providers`' five capability booleans, so that
-- `ConfigReconcile::reconcile`'s provider pass can be written through
-- CrateStack the way its currency pass already is (migration 0032).
--
-- WHY A CODE GENERATOR GETS TO DECIDE A PIECE OF DDL, WHICH IS THE PART THAT
-- DESERVES AN ARGUMENT. `cratestack-macros` drops every field carrying a
-- `@default(...)` from both `Create{Model}Input` and `upsert_update_columns`
-- (`model/inputs.rs::create_input_fields`, `model/descriptor/columns.rs`) —
-- its stated reason is that those are server-owned bindings and letting an
-- upsert clobber them would turn `upsert` into "take ownership of any row I
-- name". `model Provider` carried a `@default(...)` on all five capability
-- booleans purely because this table did, so the generated upsert could
-- write only `code`, `display_name` and `flow`. Boot step 4 would have
-- inserted every rail with `supports_refunds = false … enabled = true`
-- whatever the deployment configured, and would never have carried a
-- capability change to an existing row: a rail an operator had just disabled
-- would come back enabled. That was measured, not feared —
-- docs/plans/exp17-notes/opus.md § 3 — and it is why the provider pass
-- stayed hand-written until now.
--
-- The alternative to this migration was to leave the defaults in place and
-- leave the provider pass on hand-written SQL forever. The maintainer's
-- decision (D7, 2026-09-06) was to drop them, on the ground that the ONLY
-- writer of these five columns is `reconcile`, which always writes all five
-- from configuration. A column default here cannot help that writer; it can
-- only invent a capability for some *other* writer that forgot one — and a
-- rail silently recorded as "does not refund", or silently recorded as
-- enabled, is worse than an INSERT that refuses. See
-- docs/reference/vpay-db.md § "`providers` is written through CrateStack
-- (D7, resolved 2026-09-06)".
--
-- OPERATOR NOTE — THIS CHANGES WHAT A HAND-WRITTEN INSERT MUST SAY. All five
-- columns are `NOT NULL` and, from here on, have no default. An
-- `INSERT INTO providers (code, display_name, flow) VALUES (…)` typed at a
-- psql prompt used to succeed and now fails with
--
--   ERROR: null value in column "supports_refunds" of relation "providers"
--   violates not-null constraint (SQLSTATE 23502)
--
-- Name all eight columns. This is a refusal, not a silent difference, which
-- is the whole point: the previous behaviour was to accept the row and
-- invent the four `false`s and the `true`. Proven by
-- `a_hand_written_provider_insert_must_now_name_every_capability_column` in
-- backends/tests/integration/tests/postgres_smoke.rs, and by the three
-- fixtures in that file which had to grow the columns in this migration's
-- own commit.
--
-- BACKWARD COMPATIBILITY, unlike migration 0032: this one is safe for the
-- previous release's binary. Dropping a default changes nothing about a
-- statement that names every column, and the pre-0033 `reconcile` names all
-- eight (its hand-written `INSERT … VALUES ($1 … $8)`). Nothing else in the
-- tree writes this table. So a rolling deploy and a rollback are both fine
-- here; 0032's own header still applies to 0032.
--
-- NO DATA CHANGES. `ALTER COLUMN … DROP DEFAULT` touches `pg_attrdef` only:
-- it does not rewrite the table, does not take a lock any longer than a
-- catalog update needs, and cannot change a stored value. Every existing row
-- keeps exactly the capabilities it had.
--
-- DRIFT: zero, in both directions, and that is the measured claim rather
-- than the expected one. While the five `@default(...)` were in
-- `schemas/vpay.cstack` AND the five `DEFAULT`s were here, the two agreed
-- and the drift report said nothing about them (measured 2026-09-06,
-- exp17). With both halves gone they still agree and it still says nothing.
-- Removing only the schema half takes the report from 84 changes to 89 —
-- five `column … default value differs` lines on `providers` — which is what
-- makes this migration the *required* other half rather than tidying.
-- `EXPECTED_DRIFT_CHANGES` therefore does not move; see
-- docs/plans/exp20-provider-defaults-notes/opus.md for the four-variant
-- measurement.

ALTER TABLE providers ALTER COLUMN supports_refunds DROP DEFAULT;
ALTER TABLE providers ALTER COLUMN supports_partial_refunds DROP DEFAULT;
ALTER TABLE providers ALTER COLUMN delivers_callbacks DROP DEFAULT;
ALTER TABLE providers ALTER COLUMN requires_ip_allowlist DROP DEFAULT;
ALTER TABLE providers ALTER COLUMN enabled DROP DEFAULT;

-- Migration 0002's table comment has said "reconciliation from YAML is not
-- implemented yet" since before boot step 4 existed. It is now not merely
-- implemented but the only writer of this table, and this migration is the
-- one that makes "the only writer" load-bearing: without the defaults, a
-- writer that is not `reconcile` cannot half-fill a row. A comment in the
-- database is what an operator reads at a psql prompt with no checkout, so
-- it is worth the one statement.
COMMENT ON TABLE providers IS
    'Mirrors vpay_provider::Capabilities. Reference data, reconciled from configuration at boot step 4 by vpay_db::ConfigReconcile::reconcile (docs/flows/configuration.md). The five capability booleans have NO column default as of migration 0033: a hand-written INSERT must name all eight columns.';
