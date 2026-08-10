-- disabled_clients: an operator kill-switch for instant OAuth client
-- revocation, without a deploy.
--
-- vpay's client identity (dashboard SPA, and every merchant's
-- `client_credentials` + `private_key_jwt` registration) lives in YAML
-- (ADR-0003) — that is unchanged by this migration. This table stores no
-- credential and no identity of its own: only a client_id and a flag saying
-- "this client is currently disabled." Checking it is meant to be the last
-- gate before a token is issued or accepted: even though YAML still says a
-- client exists and what its key is, a row here means "refuse it anyway,"
-- and flipping that row is instant — no pull request, no redeploy, no config
-- reload. That is the entire point: revoking a compromised merchant key or a
-- compromised dashboard client at 2am cannot wait on a deploy pipeline.
--
-- No FK to authkestra.oauth_clients(client_id): merchant clients are
-- identified only in YAML (per ADR-0003 and this table's own purpose above),
-- never written as rows into `authkestra.oauth_clients` — that table backs
-- `SqlxOpStore`'s own `ClientStore` impl, which is not necessarily what
-- resolves a merchant's `client_credentials` registration. Requiring a
-- Postgres row to exist before a client could be disabled would make this
-- table unable to disable the exact clients it exists to disable. Same
-- no-FK reasoning already used for `merchant_id` in the (now-dropped)
-- `merchant_api_keys` table: there is nothing to point a foreign key at.
--
-- OPERATIONAL CONSEQUENCE, stated plainly: config (YAML) is no longer the
-- *sole* authority for whether a client may authenticate — this table can
-- override it. A runbook for "is this client actually live" must check
-- both YAML and this table, not YAML alone.
CREATE TABLE disabled_clients (
    client_id TEXT PRIMARY KEY,
    disabled_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reason TEXT
);

COMMENT ON TABLE disabled_clients IS
    'Operator kill-switch: disables an OAuth client (dashboard or merchant client_credentials) instantly, without a deploy. Stores no credential and no identity — only a disable flag. YAML (ADR-0003) remains authoritative for client identity; this table can still override it to revoke. Consequence: config is no longer the sole authority for whether a client may authenticate, so a runbook must read both.';
COMMENT ON COLUMN disabled_clients.reason IS
    'Free-text operator note (e.g. "key compromised, ticket INC-123"). Not machine-read by anything; purely for audit/context.';
