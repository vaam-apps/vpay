-- authkestra-op 0.5.4 -> 0.7.1: the additive schema that version's own
-- `SqlxOpStore<Postgres>::migrate()` now creates on top of what migration
-- 0006 transcribed from 0.3.4.
--
-- *** DO NOT EDIT THIS DDL INDEPENDENTLY OF THE PINNED `authkestra-op` VERSION ***
--
-- Same rule as 0006's header: this is a transcription, not a design. Source,
-- read on 2026-09-02:
--
--   ~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/authkestra-op-0.7.1/src/sqlx_store.rs
--   `impl_opstore_sql!` Postgres arm, `migrate()` (lines 587-656): the
--   `CREATE TABLE IF NOT EXISTS authkestra.oauth_dpop_jti` block, and the
--   three `ensure_postgres_column(...)` calls that follow the big DDL literal.
--
-- Diffed against 0.3.4's `migrate()` with `diff` over both extracted crate
-- sources: the four tables 0006 already creates are byte-identical at 0.7.1
-- (no column renamed, retyped or dropped), and *everything* below is new.
-- What each addition backs, per the crate's own comments:
--
--   * `oauth_dpop_jti` (authkestra#291) — RFC 9449 §11.1 DPoP proof replay
--     tracking. No FK to `oauth_clients`, deliberately: a DPoP `jti` is
--     client-generated and checked before the grant is dispatched, so it is
--     not owned by a client row and must not cascade-delete with one.
--   * `oauth_refresh_tokens.jkt` (authkestra#287) — the DPoP key thumbprint a
--     refresh token is bound to. `SqlxOpStore::get_token`/`consume_token`
--     now `SELECT ... jkt` and `try_get("jkt")` unconditionally, so at 0.7.1
--     the store's refresh-token reads FAIL at runtime against 0006's table
--     without this column — this is not optional for the pinned version.
--   * `oauth_clients.token_endpoint_auth_method`, `oauth_clients.jwks`
--     (authkestra#287) — RFC 7523 `private_key_jwt` registration data.
--     `find_client` now SELECTs both. **This closes the exact gap ADR-0010
--     cites as its reason for keeping merchant clients in YAML**: at 0.3.4
--     `find_client` hardcoded both to `None`; at 0.7.1 they are persisted and
--     read back. ADR-0010's *decision* (YAML-registered merchant clients, no
--     database-stored merchant identity) is unchanged by this migration — an
--     ADR is superseded, never edited — but its "not buildable on Authkestra
--     as published" premise is no longer true and the status page says so.
--
-- Why a vpay migration rather than calling `SqlxOpStore::migrate()` at boot:
-- the crate's own doc comment on `migrate()` explains it keeps no
-- bookkeeping table specifically so that it can coexist with a host that runs
-- `sqlx::migrate!` (vpay does — `vpay_db::run_migrations`). vpay owns its
-- schema history in this directory (0006 set that precedent); running the
-- crate's `CREATE TABLE IF NOT EXISTS` + `ADD COLUMN IF NOT EXISTS` sweep on
-- every boot alongside would be two writers of one schema.
--
-- Plain `ALTER TABLE ... ADD COLUMN`, not `IF NOT EXISTS`: sqlx migrations run
-- exactly once, in order, against a database whose prior state is 0012 — if
-- the column already existed something else wrote this schema and the
-- migration *should* fail loudly rather than paper over it.
--
-- Proven, not just transcribed: `backends/tests/integration/tests/
-- authkestra_op_smoke.rs` drives the real 0.7.1 `SqlxOpStore<Postgres>`
-- against this schema — `find_client` decodes `token_endpoint_auth_method`
-- and `jwks`, `store_token`/`get_token` round-trip `jkt`, and
-- `check_and_record_dpop_jti` inserts into `oauth_dpop_jti` and refuses the
-- replay — so a column the store's hand-built SQL names but this file lacks
-- fails a test, not a production request.

CREATE TABLE authkestra.oauth_dpop_jti (
    jti VARCHAR(255) PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL
);

ALTER TABLE authkestra.oauth_refresh_tokens ADD COLUMN jkt VARCHAR(255);
ALTER TABLE authkestra.oauth_clients ADD COLUMN token_endpoint_auth_method JSONB;
ALTER TABLE authkestra.oauth_clients ADD COLUMN jwks JSONB;

-- Added beyond the crate's own migrate(), same reasoning as 0006's index
-- block: the store only ever looks `oauth_dpop_jti` up by primary key, but an
-- expired-row sweep (there is none yet — the worker job loop is not built,
-- see docs/status.md) will scan on `expires_at`.
CREATE INDEX oauth_dpop_jti_expires_at_idx ON authkestra.oauth_dpop_jti (expires_at);

COMMENT ON TABLE authkestra.oauth_dpop_jti IS
    'Transcribed verbatim from authkestra-op 0.7.1 SqlxOpStore::migrate() (authkestra#291, RFC 9449 DPoP replay tracking). vpay offers no DPoP-bound grant today; created because the pinned store implements check_and_record_dpop_jti against it unconditionally. Do not edit independently of the pinned crate version.';
COMMENT ON COLUMN authkestra.oauth_refresh_tokens.jkt IS
    'authkestra-op 0.7.1 (authkestra#287): DPoP key thumbprint the refresh token is bound to. Read unconditionally by SqlxOpStore::get_token/consume_token at the pinned version.';
COMMENT ON COLUMN authkestra.oauth_clients.token_endpoint_auth_method IS
    'authkestra-op 0.7.1 (authkestra#287): JSON-encoded authkestra_op::client::TokenEndpointAuthMethod (e.g. "private_key_jwt"). NULL = a registration predating the field, which the OP never accepts via private_key_jwt.';
COMMENT ON COLUMN authkestra.oauth_clients.jwks IS
    'authkestra-op 0.7.1 (authkestra#287): inline public JWK Set ({"keys":[...]}) for private_key_jwt. Re-validated by the OP on every use; must never hold a private component.';
