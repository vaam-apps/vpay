-- merchant_api_keys: the /v1 merchant credential store. Deliberately in the
-- default `public` schema — this is vpay's own table, unrelated to
-- `authkestra.*` (migrations 0006/0007).
--
-- ADR-0009's scope boundary is explicit: `/v1`, the merchant API, keeps
-- Stripe-shaped opaque `sk_live_`/`sk_test_` bearer keys and does NOT move to
-- Authkestra. Reasons, restated here because they drive this table's design:
--   - Authkestra has no opaque-API-key primitive at all (it is an OIDC/OAuth
--     provider; the closest thing is client_credentials + a registered
--     client, not a per-merchant bearer secret).
--   - `authkestra`'s own `verify_secret()` is argon2 — correct for a
--     low-entropy, human-chosen password, wrong for a high-entropy random
--     API key on a payment API's hot path (argon2 is deliberately slow;
--     every `/v1/*` request would pay that cost).
--   - OAuth client_credentials would add a token-exchange round trip that
--     breaks drop-in Stripe SDK compatibility (callers expect to hand the
--     `sk_live_...` string straight to the API, not exchange it first).
--
-- Design, to support the three properties the task calls out:
--   1. Fast lookup on every request without a slow KDF: `sk_live_`/`sk_test_`
--      keys are high-entropy random tokens (unlike a password), so there is
--      nothing for a KDF's deliberate slowness to protect against — the key
--      space is already too large to brute-force offline even from a bare
--      hash. A SHA-256 digest of the full key, indexed, gives O(1) exact-match
--      lookup at request-handling speed; `key_prefix` lets an operator/log
--      line identify *which* key was used without ever storing or displaying
--      the secret itself (mirrors Stripe's own UX: "sk_live_51H...", never
--      the full value, after creation).
--   2. `livemode` distinguishing `sk_live_` vs `sk_test_` — a plain boolean
--      column, matching `payment_intents.livemode` already in this schema
--      (0003_create-payment-intents.sql) so the two line up.
--   3. Instant revocation: `revoked_at`. ADR-0008 (referenced by ADR-0009)
--      complains bearer keys generally have "no expiry or revocation story";
--      a nullable timestamp column plus an application-level
--      `WHERE revoked_at IS NULL` check on every lookup is the instant,
--      unconditional revocation path this table exists to provide — no TTL
--      to wait out, no distributed cache to invalidate.
CREATE TABLE merchant_api_keys (
    id TEXT PRIMARY KEY,
    -- No FK: same reasoning as `payment_intents.merchant_id`
    -- (0003_create-payment-intents.sql) — there is no `Merchant` table/model
    -- anywhere in this workspace yet (schemas/vpay.cstack's GAP comment
    -- confirms it). Inventing one here to hang a foreign key off would be
    -- exactly the plausible-but-fabricated failure mode CLAUDE.md warns
    -- against.
    merchant_id TEXT NOT NULL,
    -- true = sk_live_*, false = sk_test_*. Matches payment_intents.livemode's
    -- shape and meaning.
    livemode BOOLEAN NOT NULL,
    -- The first several characters of the plaintext key (e.g. 'sk_live_51H'),
    -- stored so an operator can identify a key in logs/UI without ever
    -- reading the secret back. NOT sufficient on its own to authenticate —
    -- see key_digest.
    key_prefix TEXT NOT NULL,
    -- SHA-256 digest (32 bytes) of the full plaintext key, hex-encoded.
    -- This is the only thing checked at request time: hash the presented
    -- key, look up this column. The plaintext key itself is NEVER stored —
    -- once created, the full key is unrecoverable from this table by design,
    -- exactly as merchants expect from Stripe-shaped keys ("we can't show
    -- you your key again, only issue a new one").
    key_digest TEXT NOT NULL,
    -- Instant revocation (see header comment). NULL = live. Once set, this
    -- key must never authenticate again — no grace period, no TTL to wait
    -- out.
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),
    CONSTRAINT key_prefix_length CHECK (char_length(key_prefix) BETWEEN 1 AND 32),
    -- A SHA-256 digest hex-encoded is exactly 64 lowercase hex characters.
    -- Enforcing the shape here catches an accidental base64/binary/upper-case
    -- write at insert time rather than at the next failed lookup.
    CONSTRAINT key_digest_is_sha256_hex CHECK (key_digest ~ '^[0-9a-f]{64}$'),
    -- A revoked key's revocation must not predate its own creation — cheap
    -- sanity, same spirit as oauth_signing_keys' expiry_after_creation.
    CONSTRAINT revoked_after_created CHECK (revoked_at IS NULL OR revoked_at >= created_at)
);

-- The hot-path lookup: hash the presented key, find the row. UNIQUE because
-- a digest collision would mean two different merchants' keys authenticate
-- as each other — this must be structurally impossible, not just
-- operationally unlikely (SHA-256 collision probability aside, a bug that
-- generates duplicate plaintext keys must fail loudly at INSERT, not
-- silently grant one merchant another's key).
CREATE UNIQUE INDEX merchant_api_keys_key_digest_idx ON merchant_api_keys (key_digest);

-- Operator/dashboard lookups: "list this merchant's keys" (revoked ones
-- included, for audit — hence not a partial index scoped to live keys).
CREATE INDEX merchant_api_keys_merchant_id_idx ON merchant_api_keys (merchant_id);

COMMENT ON TABLE merchant_api_keys IS
    'Stripe-shaped sk_live_/sk_test_ opaque bearer keys for /v1, the merchant API. Deliberately outside Authkestra — see this migration''s header and ADR-0009''s scope boundary. Only a SHA-256 digest is ever stored; the plaintext key is unrecoverable after creation.';
COMMENT ON COLUMN merchant_api_keys.key_digest IS
    'SHA-256 digest of the full plaintext key, hex-encoded. The plaintext itself is never persisted anywhere — this is intentional and irreversible, matching merchant expectations for a Stripe-shaped credential.';
