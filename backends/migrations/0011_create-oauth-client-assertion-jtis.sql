-- oauth_client_assertion_jtis: durable, cross-replica replay protection for
-- `private_key_jwt` client assertions (RFC 7523 §3 point 7), backing
-- `authkestra_op::client_assertion::ClientAssertionStore::record_jti`.
--
-- Merchant API auth (`/v1`) is `client_credentials` + `private_key_jwt`: each
-- merchant holds its own private key, configured in vpay's YAML, and proves
-- possession by signing a short-lived JWT assertion. `authkestra-op` ships
-- exactly two implementations of `ClientAssertionStore`:
--   - `NoClientAssertionStore` — the trait's own default, fails closed
--     (returns `OpError::ReplayProtectionUnavailable` unconditionally). See
--     `authkestra-op-0.3.4/src/client_assertion.rs`'s doc comment on
--     `record_client_assertion_jti` for why: a store that cannot offer the
--     single-use guarantee must refuse every assertion, not silently accept
--     one it cannot protect against replay.
--   - `MemoryClientAssertionStore` — single-process only. Its own doc
--     comment states the deployment requirement directly: "a multi-node
--     deployment gets one accepted replay per node; such a deployment must
--     supply a store backed by something shared (Redis `SET NX`, a SQL
--     unique index) instead."
-- vpay runs multiple replicas on Kubernetes, so neither shipped option is
-- viable — this table is that "SQL unique index" store, exactly as the
-- crate's own documentation anticipates.
--
-- The primary key IS the atomic guard. `record_jti`'s contract requires
-- "Ok(true) if this is its first use ... Ok(false) if it was already
-- recorded" as a single atomic operation — a separate SELECT-then-INSERT is
-- the exact TOCTOU race the crate's doc comment calls out (two concurrent
-- presentations of the same captured assertion would both observe "not yet
-- seen"). The Rust side must use
-- `INSERT INTO oauth_client_assertion_jtis (jti, expires_at) VALUES ($1, $2)
--  ON CONFLICT (jti) DO NOTHING` and read `rows_affected()`: 1 row affected
-- means first use (accept), 0 rows affected means already spent (reject).
-- Never check-then-insert as two statements.
--
-- KNOWN, NOT HANDLED: there is no cleanup job for expired rows. vpay's
-- worker job loop is not implemented yet (docs/status.md: "Poll ladder" row,
-- "Job loop not started") and nothing else in this repository runs
-- scheduled work. Every spent jti is kept forever, so this table grows
-- unboundedly for as long as any client presents `private_key_jwt`
-- assertions. `expires_at` and its index exist so a future sweep ("DELETE
-- WHERE expires_at < now()") is cheap to add once the job loop exists — this
-- migration does not add that sweep itself.
CREATE TABLE oauth_client_assertion_jtis (
    jti TEXT PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- For the cleanup sweep this table anticipates but does not yet implement
-- (see the KNOWN, NOT HANDLED note above) — not required for the store's own
-- read/write path, which only ever addresses rows by `jti` (the primary
-- key).
CREATE INDEX oauth_client_assertion_jtis_expires_at_idx
    ON oauth_client_assertion_jtis (expires_at);

COMMENT ON TABLE oauth_client_assertion_jtis IS
    'Durable, multi-replica replay protection for private_key_jwt client assertions (RFC 7523 §3 point 7) — backs authkestra_op::client_assertion::ClientAssertionStore. The jti primary key is the atomic single-use guard: INSERT ... ON CONFLICT (jti) DO NOTHING, read rows_affected(), never check-then-insert. No cleanup job exists yet (the worker job loop is not implemented — see docs/status.md), so this table grows unbounded; known, not handled.';
