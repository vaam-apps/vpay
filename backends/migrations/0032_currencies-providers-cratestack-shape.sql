-- Bring `currencies` and `providers` into the shape `schemas/vpay.cstack`
-- projects, so `cratestack migrate baseline` can compare them instead of
-- reporting them, and so `vpay-db` can read and write both through
-- CrateStack. Nothing about what the database *enforces* changes here: every
-- predicate below is semantically the one it replaces. What changes is the
-- spelling, and the spelling is what the diff engine matches on.
--
-- WHAT DOES CHANGE, and is not about enforcement: **this migration is not
-- backward compatible with the binary of the previous release.** It is the
-- first one here to alter a column *type* that shipping code binds. Measured
-- on 2026-09-06 against a real database with rows already in it:
--
--   * the pre-0032 binary's boot step 4 issues
--     `INSERT INTO providers (..., flow, ...) VALUES ($1, $2, $3::provider_flow, ...)`,
--     which after this migration is
--     `ERROR: type "provider_flow" does not exist` (SQLSTATE 42704);
--   * that same binary reads `currencies.exponent` as `i32`, which sqlx
--     refuses against `int8` rather than narrowing.
--
-- Migrations here are forward-only (no `down.sql`), and both binaries run
-- `run_migrations()` and then `ConfigReconcile::reconcile` before serving
-- anything. So during a rolling deploy, or after a rollback to the previous
-- image, any old-version process that *restarts* once this has landed
-- crash-loops at boot step 4. Deploy it with the release that carries the
-- matching code, and do not roll that release back past it. Splitting this
-- into an expand/contract pair across two releases (add a TEXT column,
-- dual-write, drop the enum a release later) would remove the constraint and
-- is a maintainer's call — see docs/plans/exp17-notes/opus-review.md,
-- finding 3.
--
-- Why each statement, and what it costs, is in
-- docs/reference/vpay-db.md § CrateStack. The short version:
--
--   * `Int` in a `.cstack` model always emits `int8`
--     (cratestack-migrate 0.11.1 `emit/postgres/columns.rs::scalar_to_postgres`),
--     and the introspector deliberately refuses to map `int4` back onto it
--     (`introspect/postgres/types.rs`: "an `int4` column silently mapped to
--     `Int` would make a live table that actually differs from the schema
--     look identical to it"). So an `INT` column is not a narrower `Int` —
--     it is a column the comparison cannot see at all.
--   * CrateStack names a generated CHECK `<table>_<column>_<validator>_check`
--     (`naming.rs::check_name`) and the diff matches checks **by name first**
--     (`diff/checks.rs`). A hand-named CHECK is therefore proposed for DROP
--     however correct its predicate is.
--   * CrateStack never emits a native Postgres enum. Every `.cstack` enum
--     field is `TEXT` plus a membership CHECK, because the generated row
--     decoders read an enum column with `try_get::<String>()` and `.parse()`
--     — a native enum column fails to decode on *every* read
--     (`emit/postgres/columns.rs`, citing upstream issue #228). This is the
--     first of vpay's seven native enums to be converted; the other six
--     (`intent_status`, `charge_state`, `failure_code`, `account_kind`,
--     `direction`, and `payment_intents.last_payment_error_code`) are on
--     tables no CrateStack query touches yet. See docs/status.md.

-- --- currencies -------------------------------------------------------------

-- INT -> BIGINT. Widening, so no row can fail it and no value changes.
-- `vpay-db` binds this column as `i64` from here on; `vpay_config`'s own
-- 0..=4 bound (Config::validate_all) is unchanged and still upstream of it.
ALTER TABLE currencies ALTER COLUMN exponent TYPE BIGINT;

-- `code_is_iso4217_shape` -> `currencies_code_iso4217_check`. The predicate
-- is byte-identical to what `@iso4217` renders
-- (`emit/postgres/checks.rs`: `format!("{c} ~ '^[A-Z]{{3}}$'")`), which is
-- the reason `schemas/vpay.cstack` could carry the `@db_enforce` in the
-- first place. Dropped and re-added rather than `ALTER ... RENAME
-- CONSTRAINT` so the predicate is written out where a reader can compare it
-- against the generator's, rather than being inherited invisibly.
ALTER TABLE currencies DROP CONSTRAINT code_is_iso4217_shape;
ALTER TABLE currencies ADD CONSTRAINT currencies_code_iso4217_check
    CHECK (code ~ '^[A-Z]{3}$');

-- `exponent_in_range` -> `currencies_exponent_range_check`. `BETWEEN 0 AND 4`
-- becomes `>= 0 AND <= 4`: the same set of accepted values, spelled the way
-- `@range(min: 0, max: 4)` renders it
-- (`emit/postgres/checks.rs`: `"{c} >= {min} AND {c} <= {max}"`).
ALTER TABLE currencies DROP CONSTRAINT exponent_in_range;
ALTER TABLE currencies ADD CONSTRAINT currencies_exponent_range_check
    CHECK (exponent >= 0 AND exponent <= 4);

-- --- providers --------------------------------------------------------------

-- The native `provider_flow` enum becomes TEXT. `USING flow::TEXT` is the
-- enum's own label text, so every existing row keeps exactly the value it
-- had ('push' / 'redirect').
ALTER TABLE providers ALTER COLUMN flow TYPE TEXT USING flow::TEXT;

-- What the native type used to enforce, restored as the constraint
-- CrateStack generates for an enum-typed field, under the name it generates
-- (`check_name(table, column, "enum")`). Postgres deparses `IN (...)` as
-- `= ANY (ARRAY[...])`, which is the exact shape
-- `introspect/postgres/check_pattern.rs::reconstruct_enum` reads back — so
-- this hand-written constraint and a cratestack-emitted one are the same
-- constraint to the diff engine.
ALTER TABLE providers ADD CONSTRAINT providers_flow_enum_check
    CHECK (flow IN ('push', 'redirect'));

-- Nothing references it any more. Left behind it would be a type no column
-- uses and no code names, which is the kind of leftover a later reader has
-- to prove is dead before touching anything.
DROP TYPE provider_flow;

-- `partial_refunds_imply_refunds` (migration 0002) is deliberately NOT
-- touched. It is a multi-column CHECK, and `migrate baseline` cannot see a
-- multi-column CHECK in either direction — `introspect_checks` selects
-- `array_length(c.conkey, 1) = 1`, and `AddCheck` ties to exactly one
-- column, which upstream lists under Known gaps. So it contributes nothing
-- to the drift this migration reduces, cannot be proposed for DROP, and
-- cannot be expressed in `schemas/vpay.cstack` (there is no `@@check(expr)`
-- in the 0.11.1 grammar). Its only guard is
-- `partial_refunds_without_refunds_is_rejected_by_the_database` in
-- `backends/tests/integration/tests/postgres_smoke.rs`, which reads the
-- constraint out of `pg_constraint` and then tries to violate it. Deleting
-- the constraint from 0002 fails that test and moves no drift count at all;
-- that mutation was run for this migration and is recorded in
-- docs/plans/exp17-notes/opus.md.
