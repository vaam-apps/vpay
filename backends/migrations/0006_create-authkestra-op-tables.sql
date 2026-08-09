-- authkestra-op's own OpStore schema, for vpay's `/dash/v1` OpenID Provider
-- (ADR-0009, docs/flows/dashboard-auth.md).
--
-- *** DO NOT EDIT THIS DDL INDEPENDENTLY OF THE PINNED `authkestra-op` VERSION ***
--
-- This is a byte-faithful transcription of the `CREATE SCHEMA`/`CREATE TABLE`
-- statements hardcoded as a string literal inside
-- `SqlxOpStore<sqlx::Postgres>::migrate()`, in the Postgres arm of the
-- `impl_opstore_sql!` macro invocation, in:
--
--   ~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/authkestra-op-0.3.4/src/sqlx_store.rs
--   (lines 392-448 as read on 2026-08-09)
--
-- transcribed from `authkestra-op = "=0.3.4"` (root Cargo.toml). Table names,
-- column names, column types and the `authkestra.` schema prefix are not
-- configurable — every other method in that file (`find_client`, `store_code`,
-- `consume_code`, `store_token`, `get_token`, `consume_token`,
-- `store_device_code`, …) hand-builds SQL referencing these exact identifiers
-- as string literals, so a column that differs by so much as its type here
-- compiles fine and fails (or silently misbehaves) only at runtime. Verified
-- byte-identical between `authkestra-op` 0.3.3 and 0.3.4 via `diff -rq`
-- against both extracted crates.io sources, so the exact patch pin does not
-- change this file's correctness.
--
-- If `authkestra-op`'s version pin ever moves, re-read `sqlx_store.rs`'s
-- `migrate()` block at the new version and re-diff against this file before
-- assuming it still matches — do not assume semver-minor bumps preserve
-- schema compatibility for a pre-1.0 crate.
--
-- Indexes NOT present in the crate's own `migrate()` may be *added* below
-- (they cannot break anything the store's hand-built SQL relies on, since it
-- only ever names columns, never index names) — see the additions noted
-- inline.

CREATE SCHEMA IF NOT EXISTS authkestra;

-- authkestra.oauth_clients — registered OAuth2/OIDC clients (vpay's dashboard
-- SPA, in practice exactly one row: PKCE, no client secret).
CREATE TABLE authkestra.oauth_clients (
    client_id VARCHAR(255) PRIMARY KEY,
    client_secret_hash VARCHAR(255),
    require_pkce BOOLEAN NOT NULL DEFAULT TRUE,
    redirect_uris JSONB NOT NULL,
    grant_types JSONB NOT NULL,
    scopes JSONB NOT NULL,
    allowed_audiences JSONB NOT NULL
);

-- authkestra.oauth_codes — single-use authorization codes issued at
-- `/authorize`, consumed at `/token`. `SqlxOpStore::consume_code` (Postgres
-- arm) enforces single-use with an atomic
-- `UPDATE ... SET used = TRUE WHERE code = $1 AND used = FALSE RETURNING *`,
-- not a CHECK constraint here — see the acceptance test in
-- `backends/tests/integration/tests/authkestra_op_smoke.rs` that proves a
-- second consume of the same code returns no row.
CREATE TABLE authkestra.oauth_codes (
    code VARCHAR(255) PRIMARY KEY,
    client_id VARCHAR(255) NOT NULL REFERENCES authkestra.oauth_clients(client_id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    scope TEXT NOT NULL,
    code_challenge VARCHAR(255),
    code_challenge_method VARCHAR(10),
    nonce VARCHAR(255),
    identity JSONB NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used BOOLEAN NOT NULL DEFAULT FALSE
);

-- authkestra.oauth_refresh_tokens — issued at `/token`, rotated on use
-- (docs/flows/dashboard-auth.md's token-lifetime table).
CREATE TABLE authkestra.oauth_refresh_tokens (
    token VARCHAR(255) PRIMARY KEY,
    client_id VARCHAR(255) NOT NULL REFERENCES authkestra.oauth_clients(client_id) ON DELETE CASCADE,
    identity JSONB NOT NULL,
    scope TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);

-- authkestra.oauth_device_codes — backs RFC 8628 device-authorization grant.
--
-- JUDGEMENT CALL: created even though vpay's dashboard login flow
-- (docs/flows/dashboard-auth.md) is authorization-code + PKCE only and never
-- exercises the device-code grant today. Reasons to create it anyway, not
-- skip it:
--   1. `SqlxOpStore<Postgres>` implements `DeviceCodeStore` unconditionally
--      (`impl_opstore_sql!` wires all four store traits onto the same struct
--      with no cfg gate per-trait) — the moment vpay constructs a
--      `SqlxOpStore`, that impl exists and is reachable through the trait,
--      regardless of whether any handler ever calls it. If a future
--      `/dash/v1/device_authorization` route (or a CLI-login flow) is ever
--      wired up, or if `authkestra-axum`'s router mounts device-grant
--      handlers by default, the first call would hit
--      `relation "authkestra.oauth_device_codes" does not exist` at runtime
--      instead of failing at migration time where it is cheap to notice.
--   2. This migration's whole stated purpose (task instructions, ADR-0009) is
--      "copy them EXACTLY, do not design them" — selectively omitting one of
--      the four tables the crate's own `migrate()` creates is itself a
--      design decision this migration is not supposed to make. The crate
--      author chose to create all four unconditionally, in one statement,
--      with no feature flag gating `oauth_device_codes` specifically.
--   3. Cost of creating an unused table is negligible: no rows are ever
--      written to it if nothing calls `store_device_code`, and this table
--      has no CHECK or trigger that could misfire.
-- Net: create it, matching the crate's own unconditional behaviour, rather
-- than diverging from "copy exactly" on our own judgement.
CREATE TABLE authkestra.oauth_device_codes (
    device_code VARCHAR(255) PRIMARY KEY,
    user_code VARCHAR(255) UNIQUE NOT NULL,
    client_id VARCHAR(255) NOT NULL REFERENCES authkestra.oauth_clients(client_id) ON DELETE CASCADE,
    scope TEXT NOT NULL,
    status JSONB NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    last_polled_at TIMESTAMPTZ
);

-- --- Indexes added beyond the crate's own migrate() ---------------------
--
-- The crate's `migrate()` creates no secondary indexes at all — only the
-- PRIMARY KEY/UNIQUE constraints above, which already index `client_id` (via
-- being the PK of `oauth_clients`) but NOT the foreign-key columns on the
-- three dependent tables. Every lookup the store performs is by primary key
-- (`code`, `token`, `device_code`, `user_code` — all already indexed above),
-- so these are not required for the store's own read paths to work. They are
-- added for operational query patterns the store doesn't need but an
-- operator or a future admin view will (e.g. "list all live codes/tokens for
-- client X", cleanup sweeps for expired rows). None of this changes any
-- column the store reads or writes, and none of it is a UNIQUE/CHECK
-- constraint that could reject a row the store's own hand-built SQL would
-- otherwise accept.
CREATE INDEX oauth_codes_client_id_idx ON authkestra.oauth_codes (client_id);
CREATE INDEX oauth_codes_expires_at_idx ON authkestra.oauth_codes (expires_at);
CREATE INDEX oauth_refresh_tokens_client_id_idx ON authkestra.oauth_refresh_tokens (client_id);
CREATE INDEX oauth_refresh_tokens_expires_at_idx ON authkestra.oauth_refresh_tokens (expires_at);
CREATE INDEX oauth_device_codes_client_id_idx ON authkestra.oauth_device_codes (client_id);
CREATE INDEX oauth_device_codes_expires_at_idx ON authkestra.oauth_device_codes (expires_at);

COMMENT ON TABLE authkestra.oauth_clients IS
    'Transcribed verbatim from authkestra-op 0.3.4 SqlxOpStore::migrate(). Do not edit independently of the pinned crate version — see this migration''s header comment.';
COMMENT ON TABLE authkestra.oauth_codes IS
    'Transcribed verbatim from authkestra-op 0.3.4 SqlxOpStore::migrate(). Single-use enforced by SqlxOpStore::consume_code''s atomic UPDATE, not a constraint here.';
COMMENT ON TABLE authkestra.oauth_refresh_tokens IS
    'Transcribed verbatim from authkestra-op 0.3.4 SqlxOpStore::migrate().';
COMMENT ON TABLE authkestra.oauth_device_codes IS
    'Transcribed verbatim from authkestra-op 0.3.4 SqlxOpStore::migrate(). Unused by vpay''s PKCE-only login flow today; created because SqlxOpStore implements DeviceCodeStore unconditionally — see the judgement-call comment above this table.';
