-- envelope_nonces: the replay cache of the signed `/v1` transport (VP-3).
--
-- Every request a merchant signs carries a `cti` (a 16-byte random id) and an
-- `iat`. cratestack's `CoseEnvelope` refuses a request whose `iat` is further
-- from the verifier's clock than the skew (300 s by default) and then records
-- `(kid, cti)` until `iat + 2*skew + 1` seconds; a second request with the same
-- pair inside that window is a replay and is answered with the coarse, unsigned
-- 401. cratestack ships two stores for that record, in memory and Redis. The
-- in-memory one is per process, so each of vpay's replicas (and its separate
-- business and management Deployments, ADR-0022) would accept one replay of a
-- captured request. vpay runs no Redis. This table is the third store, and the
-- shared one: Postgres, like every other piece of state that has to hold across
-- replicas here (`oauth_client_assertion_jtis`, `rate_limit_windows`).
--
-- THE PRIMARY KEY IS THE ATOMIC GUARD, as it is for `oauth_client_assertion_jtis`
-- (0011): `id` is cratestack's own key, `cose:<hex kid>:<hex cti>` (5 + 16 + 1 +
-- up to 32 characters; the CHECK below allows 64). A request is first-seen iff
-- one statement returned a row:
--
--     INSERT INTO envelope_nonces (id, expires_at) VALUES ($1, $2)
--     ON CONFLICT (id) DO UPDATE SET expires_at = EXCLUDED.expires_at
--       WHERE envelope_nonces.expires_at < now()
--     RETURNING id
--
-- Never check-then-insert. The `DO UPDATE ... WHERE` arm takes over a row whose
-- window has closed and not yet been swept, which is what cratestack's
-- `NonceStore` contract asks ("must drop entries past `expires_at`") and what a
-- counter-shaped `cti` (cratestack allows 1-4 byte ones) needs; a row whose
-- window is still open is left alone and the statement returns nothing.
--
-- WHAT WRITES IT. A request that has already VERIFIED against a registered
-- merchant key (the nonce is recorded after the signature, not before), so an
-- anonymous caller cannot put a row here, and the keyspace is `kid`s that
-- belong to a merchant. That is the opposite of `rate_limit_windows`, whose keys
-- an unauthenticated caller chooses, and is why this table needs no
-- sweep-in-the-write: the worker's hourly housekeeping sweep
-- (`vpay_worker::handlers::sweep_expired`) deletes rows whose `expires_at` is
-- more than `ENVELOPE_NONCE_RETENTION` past, on the database's clock
-- (ADR-0026).
--
-- BORN WITH A `schemas/vpay.cstack` MODEL, with 0034's rule: no `DEFAULT` on a
-- column a writer names, no `bytea`, no `jsonb`. The model carries no `@@allow`
-- arm: nothing reads or writes the table through the generated layer, and an
-- arm appearing would be a standing permission over a table whose rows decide
-- whether a captured request is accepted again.
CREATE TABLE envelope_nonces (
    id TEXT PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT envelope_nonces_id_length CHECK (char_length(id) BETWEEN 1 AND 64)
);

-- The sweep's index: `DELETE ... WHERE expires_at < now() - retention`. The
-- write path addresses rows by `id` alone.
CREATE INDEX envelope_nonces_expires_at_idx ON envelope_nonces (expires_at);

COMMENT ON TABLE envelope_nonces IS
    'Replay cache of the signed /v1 transport (VP-3): one row per (kid, cti) a verified merchant request has used, kept until iat + 2*skew + 1 seconds. The id primary key is the single-use guard: INSERT ... ON CONFLICT (id) DO UPDATE ... WHERE expires_at < now() RETURNING id, never check-then-insert. Swept hourly by the worker.';
COMMENT ON COLUMN envelope_nonces.id IS
    'cratestack''s nonce key: cose:<hex kid>:<hex cti>. Written only after the request''s signature verified against a registered merchant key.';
COMMENT ON COLUMN envelope_nonces.expires_at IS
    'When the (kid, cti) pair stops being replayable: the request''s iat plus twice the clock skew plus one second, as cratestack computes it.';
