-- Bring the four money tables — `payment_intents`, `charges`, `refunds` and
-- `checkout_sessions` — into the shape `schemas/vpay.cstack` can project, so
-- `cratestack migrate baseline` compares them column by column instead of
-- reporting each as a table it cannot see. This is migration 0032's work
-- (`currencies`, `providers`) applied to the tables money actually moves
-- through, and it is the prerequisite the data-layer plan calls S5.
--
-- Nothing about what the database *enforces* changes here. Every native enum
-- becomes `TEXT` plus a membership CHECK over exactly the labels the type
-- carried, so the set of storable values is identical before and after. No
-- column is added, dropped, widened or narrowed; no row's value changes.
--
--
-- WHAT DOES CHANGE, AND IS NOT ABOUT ENFORCEMENT: **this migration is not
-- backward compatible with the binary of the previous release**, for exactly
-- 0032's reason and on a much larger surface. `vpay-db` binds these columns
-- with explicit casts today —
-- `$4::intent_status`, `'submitting'::charge_state`, `$2::failure_code`,
-- `$4::refund_status` — and after this migration every one of those is
-- `ERROR: type "intent_status" does not exist` (SQLSTATE 42704).
--
-- Measured, on 2026-09-08, by building the previous release's `vpay-db`
-- (commit 889d045) and running it against real databases this migration had
-- been applied to. It has TWO failure modes, not one, and which one an
-- operator sees depends on whether the old process is already running:
--
--   * **A process that RESTARTS — a rollback to the previous image, a pod
--     rescheduled — does not serve at all. It fails at boot**, in
--     `run_migrations()`, before `ConfigReconcile::reconcile` and before the
--     listener binds:
--         `migration 37 was previously applied but is missing in the
--          resolved migrations`   (`sqlx::MigrateError::VersionMissing(37)`)
--     `Migrator::validate_applied_migrations` refuses any applied version the
--     binary does not carry, and `run_migrations` never sets `ignore_missing`
--     (`sqlx-core-0.9.0/src/migrate/migrator.rs:363-380`). This is exactly
--     what 0032 does on a restart too; the difference between the two
--     migrations is NOT here.
--   * **A process that KEEPS RUNNING — the rolling-deploy window — serves
--     until it touches a money WRITE, and then fails that request.** Measured
--     on the previous binary against a 0037-shaped database:
--         `PaymentIntents::insert`        -> 42704 type "intent_status" does not exist
--         `PaymentIntents::transition`    -> 42704 type "intent_status" does not exist
--         `Settlement::set_live_state`    -> 42704 type "charge_state"  does not exist
--     This is where 0037 differs from 0032, which had no in-flight failure
--     mode of its own: an old process serving a confirm, a settlement or a
--     cancel in that window fails that request with a storage error.
--
-- **The READS are NOT affected, and that is the quiet half.** An earlier
-- draft of this header said they were "equally affected"; that was wrong and
-- the measurement above is what corrected it. `COLUMNS` selects
-- `status::TEXT AS status`, and `TEXT` is a built-in type this migration does
-- not drop — the cast is a no-op, not a reference to a vanished type.
-- Measured on the previous binary: `PaymentIntents::get_for_merchant`
-- answered `Some("requires_payment_method")` normally on a 0037 database.
-- So the rolling-deploy window does not look like an outage. `GET` keeps
-- serving, dashboards keep rendering, and only the statements that move money
-- fail. Do not wait for reads to go dark as the signal.
--
-- Migrations here are forward-only (no `down.sql`). Deploy this with the
-- release that carries the matching code, **drain the previous version before
-- it lands** — that is what closes the write window above — and do not roll
-- that release back past it, because a rolled-back image will not boot.
--
-- The expand/contract alternative (add a `TEXT` column beside each enum,
-- dual-write for a release, drop the enum a release later) would remove that
-- constraint. It is not taken here for the reason 0032 recorded and one more:
-- it would put a second copy of `charges.state` in the table for the length of
-- a release, and two columns that both claim to be the state of a charge is a
-- worse hazard than a deploy ordering rule. It remains a maintainer's call —
-- see docs/plans/exp34-money-tables-notes/opus.md.
--
--
-- WHY EACH KIND OF STATEMENT
--
--   * CrateStack never emits a native Postgres enum. Its generated row
--     decoders read an enum column with `try_get::<String>()` and `.parse()`,
--     so a native enum column fails to decode on *every* read
--     (cratestack-migrate 0.12.0 `emit/postgres/columns.rs`, upstream issue
--     #228). Any CrateStack query touching these tables would have errored.
--   * A generated enum CHECK is named `<table>_<column>_enum_check`
--     (`naming.rs::check_name`) and the diff engine matches CHECKs **by name
--     first** (`diff/checks.rs`), so the name is the half that can converge.
--     Postgres deparses `IN (...)` as `= ANY (ARRAY[...])`, which is the exact
--     shape `introspect/postgres/check_pattern.rs::reconstruct_enum` reads
--     back — measured on a real database while writing this migration.
--   * The conversion itself moves the drift count by **zero**, and that is
--     expected rather than a disappointment: `introspect/postgres/enums.rs`
--     already synthesised an `AddCheck { kind: Enum }` out of `pg_enum` for
--     the native column, and `resolve_column` maps a native enum and a `TEXT`
--     column onto the same `ColumnType::Scalar("String")`. The report was
--     structurally blind to the whole thing in both directions. What the
--     conversion buys is that a CrateStack query on these tables can decode at
--     all. docs/reference/vpay-db.md § "The enum conversion no report can see"
--     is the long form.
--
-- WHAT THIS MIGRATION DELIBERATELY DOES NOT DO
--
--   * **It drops no column default.** Migration 0033 dropped five on
--     `providers` because `cratestack-macros` filters every `@default(...)`
--     field out of `Create{Model}Input`, so a defaulted column is one the
--     CrateStack writer could not set. That argument applies only to a column
--     a CrateStack write must name, and no write on these four tables moves
--     (docs/reference/vpay-db.md § "What stays raw sqlx, and why" says why:
--     every write here either returns a `jsonb` column or carries a
--     correlated sub-select). `payment_intents.insert` does **not** name
--     `amount_received`, `amount_refunded`, `amount_refund_pending` or
--     `updated_at`; `checkout_sessions.create` does not name `updated_at`.
--     Dropping those defaults would turn each of those inserts into a `23502`
--     — a refusal, in the money path, bought for nothing. The models declare
--     the defaults instead, so the two sides agree and the columns cost no
--     drift (0033's own measurement: a default costs drift only when the two
--     sides disagree).
--   * **It renames no hand-written CHECK.** `checkout_sessions`' three closed
--     vocabularies (`ui_mode_is_known`, `status_is_known`,
--     `payment_status_is_known`) are already `TEXT` + CHECK and are left under
--     their own names, exactly as `events.type_is_a_documented_event` is. A
--     rename moves the count by zero (measured for 0032's two `currencies`
--     CHECKs), and `model CheckoutSession` declares those three columns as
--     plain `String` rather than `.cstack` enums — so there is no generated
--     name for them to converge on.
--   * **It touches no multi-column CHECK.** `no_over_refund`,
--     `partial_refunds_imply_refunds`, `lpe_paired`, `failure_paired`,
--     `urls_match_ui_mode` and the rest are invisible to `migrate baseline` in
--     both directions (`introspect_checks` selects
--     `array_length(c.conkey, 1) = 1`), contribute nothing to the drift this
--     migration is about, and are pinned by `postgres_smoke.rs` instead.

-- --- payment_intents --------------------------------------------------------

-- `USING status::TEXT` is the enum's own label text, so every existing row
-- keeps exactly the value it had. No index on this column, partial or
-- otherwise, so nothing has to be dropped first.
ALTER TABLE payment_intents ALTER COLUMN status TYPE TEXT USING status::TEXT;

-- The five labels `intent_status` carried (migration 0003), in the order the
-- type declared them, under the name CrateStack generates for
-- `status IntentStatus`.
ALTER TABLE payment_intents ADD CONSTRAINT payment_intents_status_enum_check
    CHECK (status IN (
        'requires_payment_method',
        'requires_action',
        'processing',
        'succeeded',
        'canceled'
    ));

ALTER TABLE payment_intents
    ALTER COLUMN last_payment_error_code TYPE TEXT USING last_payment_error_code::TEXT;

-- Nullable, and the predicate is a bare `IN (...)` anyway — no `IS NULL OR`.
-- That is not a shortcut: `NULL IN (...)` is NULL and a CHECK fails only on
-- FALSE, so a NULL already passes, and the generator relies on exactly that
-- (`emit/postgres/checks.rs`: "NULL passes … Nullable enum columns therefore
-- need no special casing"). Spelling the NULL case out would deparse to
-- `last_payment_error_code IS NULL OR (last_payment_error_code = ANY (…))`,
-- which `introspect/postgres/check_pattern.rs::reconstruct_enum` cannot read
-- back — it matches `<column> = ANY (ARRAY[…])` and nothing else — so the
-- constraint would introspect as `CheckKind::Raw` and diff against the
-- generated `CheckKind::Enum` as a kind mismatch. Which half of the pair is
-- allowed to be absent stays `lpe_paired`'s job (migration 0014).
ALTER TABLE payment_intents
    ADD CONSTRAINT payment_intents_last_payment_error_code_enum_check
    CHECK (last_payment_error_code IN (
        'insufficient_funds',
        'payer_timeout',
        'payer_declined',
        'invalid_payer',
        'payer_limit_reached',
        'payer_account_blocked',
        'invalid_payee',
        'payee_account_blocked',
        'provider_account_blocked',
        'provider_unavailable',
        'provider_error'
    ));

-- --- charges ----------------------------------------------------------------

-- `charges_live_idx` (migration 0014) is a PARTIAL index whose predicate names
-- four `charge_state` literals, and Postgres refuses to alter a column a
-- partial index predicate depends on: measured while writing this migration,
-- `ERROR: operator does not exist: text = charge_state`. It is dropped and
-- rebuilt below with the same columns and the same predicate.
--
-- `charges_state_idx` (a plain index on the same column) needs no such
-- handling — Postgres rebuilds a plain index across an `ALTER COLUMN ... TYPE`
-- on its own, which was measured in the same session.
DROP INDEX charges_live_idx;

ALTER TABLE charges ALTER COLUMN state TYPE TEXT USING state::TEXT;

ALTER TABLE charges ADD CONSTRAINT charges_state_enum_check
    CHECK (state IN (
        'submitting',
        'submitted',
        'pending',
        'unresolved',
        'succeeded',
        'failed'
    ));

-- Rebuilt byte-for-byte from migration 0014, which is also where the reason
-- for it lives: it is the index `PaymentIntents::cancel`'s and
-- `CheckoutSessions::expire`'s `NOT EXISTS (… state IN (…))` guards plan
-- against, and `vpay_db::payment_intents::LIVE_CHARGE_STATES` is the Rust
-- copy of the same four labels.
CREATE INDEX charges_live_idx ON charges (state)
    WHERE state IN ('submitting', 'submitted', 'pending', 'unresolved');

ALTER TABLE charges ALTER COLUMN failure_code TYPE TEXT USING failure_code::TEXT;

-- Nullable; a bare `IN (...)`, for the reason spelled out on
-- `payment_intents_last_payment_error_code_enum_check` above.
ALTER TABLE charges ADD CONSTRAINT charges_failure_code_enum_check
    CHECK (failure_code IN (
        'insufficient_funds',
        'payer_timeout',
        'payer_declined',
        'invalid_payer',
        'payer_limit_reached',
        'payer_account_blocked',
        'invalid_payee',
        'payee_account_blocked',
        'provider_account_blocked',
        'provider_unavailable',
        'provider_error'
    ));

-- --- refunds ----------------------------------------------------------------

ALTER TABLE refunds ALTER COLUMN status TYPE TEXT USING status::TEXT;

ALTER TABLE refunds ADD CONSTRAINT refunds_status_enum_check
    CHECK (status IN ('pending', 'succeeded', 'failed', 'canceled'));

ALTER TABLE refunds ALTER COLUMN failure_code TYPE TEXT USING failure_code::TEXT;

-- Nullable; a bare `IN (...)`, for the reason spelled out on
-- `payment_intents_last_payment_error_code_enum_check` above.
ALTER TABLE refunds ADD CONSTRAINT refunds_failure_code_enum_check
    CHECK (failure_code IN (
        'insufficient_funds',
        'payer_timeout',
        'payer_declined',
        'invalid_payer',
        'payer_limit_reached',
        'payer_account_blocked',
        'invalid_payee',
        'payee_account_blocked',
        'provider_account_blocked',
        'provider_unavailable',
        'provider_error'
    ));

-- --- the types themselves ---------------------------------------------------

-- Three of vpay's six remaining native enums, and the fourth
-- (`failure_code`) was shared by three columns across three tables — which is
-- why it is dropped last, after all three have been converted. `DROP TYPE`
-- without `CASCADE` on purpose: if any column, domain, function argument or
-- composite type this migration did not find still references one of these,
-- Postgres refuses the statement and the migration fails loudly, rather than
-- silently dropping whatever depended on it. That refusal is the check that
-- "nothing references it any more" is true rather than believed, and it is
-- re-run on every fresh database.
--
-- `account_kind` and `direction` (migration 0005, on the ledger tables) are
-- deliberately NOT converted: no CrateStack query touches `ledger_entries` or
-- `ledger_transactions`, this migration's scope is the four tables above, and
-- converting a money column's type buys nothing until something needs to
-- decode it. docs/status.md carries them as the remaining two.
DROP TYPE intent_status;
DROP TYPE charge_state;
DROP TYPE refund_status;
DROP TYPE failure_code;

-- --- what an operator should be able to see afterwards ----------------------

COMMENT ON CONSTRAINT payment_intents_status_enum_check ON payment_intents IS
    'The five intent_status labels migration 0003 created as a native enum, restored as a membership CHECK when 0037 converted the column to TEXT. Named the way CrateStack names a generated enum CHECK (<table>_<column>_enum_check) so the diff engine matches it rather than proposing a drop-and-add pair.';
COMMENT ON CONSTRAINT charges_state_enum_check ON charges IS
    'The six charge_state labels migration 0004 created as a native enum. See payment_intents_status_enum_check for why the name is what it is.';
COMMENT ON CONSTRAINT refunds_status_enum_check ON refunds IS
    'The four refund_status labels migration 0017 created as a native enum. See payment_intents_status_enum_check for why the name is what it is.';
COMMENT ON COLUMN charges.state IS
    'TEXT since migration 0037, native charge_state enum before it. The storable set is unchanged: charges_state_enum_check carries exactly the six labels the type did. vpay_db::payment_intents::LIVE_CHARGE_STATES names the four non-terminal ones and charges_live_idx indexes them.';
