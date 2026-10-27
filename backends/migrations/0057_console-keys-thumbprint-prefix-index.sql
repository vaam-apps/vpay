-- console_keys: an index the signed transport's per-request read can use.
--
-- `/dash/v1` resolves a request's key by the COSE `kid`, which is the first 8
-- bytes of a thumbprint (16 lower-case hex characters), with
--
--     SELECT ... FROM console_keys
--      WHERE thumbprint LIKE $1 || '%' AND revoked_at IS NULL AND expires_at > $2
--      LIMIT 16
--
-- (`vpay_db::ConsoleKeys::live_console_keys_for_kid`), before the signature is
-- verified, so any well-formed `kid` an unauthenticated caller sends costs one
-- such read. `console_keys_thumbprint_key` (0056) cannot serve it: a btree index
-- only answers a left-anchored LIKE when its column is compared under the C
-- collation or the index was built with `text_pattern_ops`, and under any other
-- collation (a database created `en_US.UTF-8`, or with an ICU locale) the
-- planner falls back to a sequential scan of the whole table. Measured on 50 000
-- rows under an `en-US` ICU database: `Seq Scan` before this index, `Index Scan
-- ... Index Cond: (thumbprint ~>=~ '0123456789abcdef' AND thumbprint ~<~
-- '0123456789abcdeg')` after, from the first execution of the prepared statement
-- through the eighth (past the custom-plan threshold).
--
-- Partial on `revoked_at IS NULL`: the read always carries that predicate, and a
-- revoked key is never wanted by it, so the index stays the size of the keys that
-- can still authenticate (an expired key is filtered by `expires_at`, which is
-- time, and cannot be a predicate of an index). `console_keys_thumbprint_key`
-- stays: it is the uniqueness wall that keeps a revoked thumbprint taken.
--
-- Migration 0056 is not edited to add this: its bytes are recorded in
-- MANIFEST.sha256 and `just migrations-manifest` only appends.

CREATE INDEX console_keys_thumbprint_prefix_idx
    ON console_keys (thumbprint text_pattern_ops)
    WHERE revoked_at IS NULL;

COMMENT ON INDEX console_keys_thumbprint_prefix_idx IS
    'Serves the signed transport''s per-request lookup by COSE kid (thumbprint LIKE ''<16 hex>%'') under any collation; console_keys_thumbprint_key cannot. Partial on revoked_at IS NULL.';
