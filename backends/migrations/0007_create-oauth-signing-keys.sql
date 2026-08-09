-- oauth_signing_keys: vpay's own RS256 signing-key store and rotation
-- bookkeeping for the `/dash/v1` OpenID Provider (ADR-0009,
-- docs/flows/dashboard-auth.md's "JWKS publication and key rotation"
-- section).
--
-- Confirmed by reading the source directly: `authkestra` has no signing-key
-- type, no key store and no rotation logic at any published version.
-- `grep -rn 'struct SigningKey\|trait KeyStore\|fn rotate'` across
-- `authkestra-op-0.3.4` and `authkestra-engine-0.3.4` (both under
-- `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`) returns nothing.
-- `TokenManager::new_asymmetric(pem, issuer, kid)` holds exactly one key at a
-- time. So key generation, storage, rotation and JWKS assembly across
-- multiple keys are entirely vpay's own responsibility — this table is not
-- mirroring anything the crate ships, unlike migration 0006.
--
-- This is deliberately in the `public` schema, not `authkestra.`: it is
-- vpay's own table, not part of the transcribed OpStore schema.
--
-- Precedent read (read-only, not modified): ~/dev/vsms/crates/sms-auth/src/op.rs
-- and its `oauth_signing_keys` table in
-- ~/dev/vsms/schema/migrations/postgres/0001_init/up.sql. vsms's rotation
-- generates a new RSA-2048 key, inserts it active, and deactivates the
-- previous active row(s) with a 30-minute overlap window
-- (`ROTATION_OVERLAP`) so in-flight tokens signed by the old key still
-- verify against a JWKS that keeps publishing it. Treated as a starting
-- point, not gospel: vsms only ever issued machine tokens via
-- client_credentials, never PKCE under a human login flow, so its schema is
-- adopted here but its constraint set is reconsidered from scratch below.
CREATE TABLE oauth_signing_keys (
    id TEXT PRIMARY KEY,
    -- SECRET MATERIAL. This column holds an RSA private key in PEM form.
    -- Encryption-at-rest is NOT implemented by this migration or by any code
    -- in this repository today — the column is plain TEXT, protected only by
    -- whatever access control and disk/backup encryption the Postgres
    -- deployment itself provides. Anyone who can `SELECT` this column reads
    -- the live signing key outright. Stating this plainly rather than
    -- implying it is handled: it is not handled.
    private_key_pem TEXT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    -- NULL while active (an active key has no scheduled retirement). Set to
    -- `now() + ROTATION_OVERLAP`-equivalent when rotated out, per the vsms
    -- precedent: a deactivated key keeps publishing in JWKS until its
    -- verification window closes, so tokens it already signed keep verifying
    -- for their remaining lifetime.
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    -- Non-null PEM is already enforced by NOT NULL above; this adds a shape
    -- floor so an empty string or a stray whitespace-only value (which would
    -- satisfy NOT NULL but is obviously not a PEM-encoded key) cannot slip
    -- in undetected. Not a full PEM-grammar validator — Postgres CHECK is
    -- the wrong tool for that — just a sanity floor.
    CONSTRAINT private_key_pem_looks_like_pem
        CHECK (private_key_pem LIKE '-----BEGIN%KEY-----%'),
    -- Expiry sanity: an active key must not carry an expiry (it has none
    -- scheduled yet — rotation sets both `active = false` and `expires_at`
    -- together, never one without the other), and if set, it must be after
    -- creation. This is stricter than vsms, which leaves expires_at
    -- nullable on both active and inactive rows with no CHECK at all;
    -- vpay's human-login flow makes an accidentally-expired *active* key a
    -- worse failure mode (dashboard staff locked out with no fallback IdP —
    -- ADR-0009's whole point is that vpay IS the IdP), so this migration
    -- enforces the invariant at the database rather than trusting
    -- application code alone to keep the two fields in lockstep.
    CONSTRAINT active_key_has_no_expiry
        CHECK ((active AND expires_at IS NULL) OR (NOT active)),
    CONSTRAINT expiry_after_creation
        CHECK (expires_at IS NULL OR expires_at > created_at)
);

-- At most one active key at a time. A plain UNIQUE(active) would allow at
-- most one TRUE *and* at most one FALSE row, which is wrong — many retired
-- keys must coexist during their overlap windows. A partial unique index
-- scoped to `WHERE active` is the natural tool: it only constrains the rows
-- that matter and imposes nothing on inactive ones.
--
-- JUDGEMENT CALL: this makes rotation a two-statement operation (insert the
-- new active key, then in the same transaction flip the old row(s) to
-- inactive) rather than one atomic UPDATE...INSERT — inserting the new
-- active row before deactivating the old one would violate this index
-- mid-transaction if done in the wrong order, or trip it entirely if not
-- wrapped in a transaction. That is the correct tradeoff here: the
-- alternative (no uniqueness constraint, rely on application code alone to
-- never leave two keys active) is exactly the kind of invariant this
-- codebase's own convention (docs/flows/ledger.md, the CHECK constraints in
-- migrations 0002/0003) says belongs in the database, not only in Rust —
-- a bug in future rotation code should fail loudly at INSERT/UPDATE time,
-- not silently leave two keys signing tokens with no way to tell which JWKS
-- entry a given token actually used.
CREATE UNIQUE INDEX one_active_signing_key ON oauth_signing_keys (active) WHERE active;

CREATE INDEX oauth_signing_keys_expires_at_idx ON oauth_signing_keys (expires_at)
    WHERE expires_at IS NOT NULL;

COMMENT ON TABLE oauth_signing_keys IS
    'vpay-owned RS256 signing-key rotation store for the /dash/v1 OP. Not part of authkestra-op''s schema — authkestra ships no key/rotation type at all (see this migration''s header). private_key_pem is unencrypted at rest; see the column comment.';
COMMENT ON COLUMN oauth_signing_keys.private_key_pem IS
    'Secret material (RSA private key, PEM-encoded). Encryption-at-rest is NOT implemented. Restrict SELECT access at the database role level until it is.';
