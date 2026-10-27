-- console_keys: a person's registered console signing key, and the end of
-- password, TOTP, SSO and bearer-token sign-in (VP-6a).
--
-- WHAT CHANGES. `/dash/v1` is a signed message now, exactly as `/v1` became in
-- 0055's change (ADR-0035): a person's console holds a non-extractable P-256
-- key, signs every request with it, and vpay finds the person by the key. There
-- is no session, no access token, no authorization code and no password, so
-- the four tables and the two credential kinds that existed to carry them go,
-- in the same step that adds the table that replaces them. Owner rulings,
-- 2026-10-26: Q1, phone pairing only (so Keycloak SSO and password + TOTP are
-- deleted, not kept as a second door); Q2, break-glass is a command-line key
-- enrolment (`vpay-server staff pair-key`); R2, a person paired through vaam is a
-- plain member bound to `dashboard.merchant_id`, never an admin; R3, a console
-- key expires 30 days after it is paired.
--
-- WHAT IS DROPPED, AND WHY EACH IS SAFE TO DROP
--
--   * `oauth_authorization_codes`, `staff_sessions`, `staff_sso_attempts`:
--     short-lived by construction (a code lives minutes, a session twelve
--     hours, a pending SSO flow ten minutes). Nothing a deployment must keep
--     lives in them; the cutover signs everyone out, which is the intent. Dropped
--     children-first: `oauth_authorization_codes.session_id` names a
--     `staff_sessions` row.
--   * the `password` and `totp` rows of `credentials`: material that
--     authenticates nobody once the code that checks it is gone, and an
--     argon2id digest and a sealed TOTP seed are exactly what should not
--     outlive their use. The `oidc` rows STAY: `(issuer, subject)` is how a
--     staff member provisioned from a vaam identity is found again, and it is a
--     public identifier, not a secret. The table, its `must_change`/`counter`
--     columns and its eight-name `kind` CHECK are left as shipped (0044 is never
--     edited); the CHECK added below closes the two names this migration
--     retires, so a row of either can never be written again.
--
-- THE SHAPE OF THE NEW TABLE is 0035's, to the letter: no `DEFAULT` on a column
-- a writer names (cratestack drops a defaulted field from the Create input), no
-- native enum, no `bytea`, no `jsonb` (an unmapped type is excluded from the
-- drift comparison outright). The public key is therefore base64url TEXT, as
-- 0044 said a WebAuthn key would be.
--
-- THE 30 DAYS ARE A CONSTRAINT, NOT ONLY A CONSTANT. `expires_at` is
-- `paired_at + 720 hours` and the database refuses any other row. Hours, not
-- `30 days`: `timestamptz + interval '30 days'` is calendar arithmetic in the
-- session time zone and differs from thirty 24-hour days across a DST change,
-- while the Rust writer adds exactly 2 592 000 seconds. A key is never extended:
-- a console pairs again with a new key, so a stolen browser profile has a
-- bounded life even if nobody notices.
--
-- A REVOKED OR EXPIRED KEY IS NEVER REGISTERED AGAIN, and the row is never
-- deleted, so its thumbprint stays taken: `console_keys_thumbprint_key`
-- refuses the same key twice, and `vpay_db::console_keys` answers a repeat of a
-- LIVE key for the SAME member with the row it already has.

DROP TABLE oauth_authorization_codes;
DROP TABLE staff_sessions;
DROP TABLE staff_sso_attempts;

DELETE FROM credentials WHERE kind IN ('password', 'totp');

ALTER TABLE credentials
    ADD CONSTRAINT credentials_password_and_totp_are_retired
    CHECK (kind NOT IN ('password', 'totp'));

CREATE TABLE console_keys (
    -- `ck_…`, minted by `vpay_core::ids::console_key_id`.
    id TEXT PRIMARY KEY,

    -- WHICH PERSON. A real FK with ON DELETE CASCADE, the shape `credentials`
    -- has: a key that outlives its member is an authenticator with nobody to
    -- authenticate. NOT NULL: unlike a credential, a console key has exactly
    -- one kind of subject.
    staff_member_id TEXT NOT NULL REFERENCES staff_members (id) ON DELETE CASCADE,

    -- COSE `ESP256` (-9), spelled as the model's value. One value today; a CHECK
    -- and not a native enum for 0035's reason.
    alg TEXT NOT NULL,

    -- The SEC1 uncompressed P-256 point, 65 bytes starting 0x04, as unpadded
    -- base64url: what WebCrypto's `exportKey("raw")` returns and
    -- `CoseVerifyKey::p256_sec1` takes. 87 characters, the first of which is
    -- always `B` (the six high bits of 0x04 are 000001). A compressed point is
    -- a valid key in a second spelling and is REFUSED, not normalised, by
    -- `vpay_api::console_keys::parse_public_key`; the CHECK is the second wall.
    public_key TEXT NOT NULL,

    -- The RFC 9679 COSE key thumbprint of that key, SHA-256, lower-case hex:
    -- computed by `cratestack-cose` and never taken from a caller. Unique. Its
    -- first 16 characters are the 8-byte `kid` a signed request carries, which
    -- is how the resolver finds the candidates for a signature.
    thumbprint TEXT NOT NULL,

    -- A name for the key in logs and runbooks. Never an identifier.
    label TEXT,

    paired_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,

    -- When it was revoked, or NULL. Set once: a second revoke keeps the first
    -- value. Revocation is a column, not a delete, so a revoked thumbprint
    -- cannot be registered again.
    revoked_at TIMESTAMPTZ,

    CONSTRAINT console_keys_id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT console_keys_alg_enum_check CHECK (alg IN ('esp256')),
    CONSTRAINT console_keys_public_key_shape CHECK (public_key ~ '^B[A-Za-z0-9_-]{86}$'),
    CONSTRAINT console_keys_thumbprint_format CHECK (thumbprint ~ '^[0-9a-f]{64}$'),
    CONSTRAINT console_keys_label_length CHECK (char_length(label) BETWEEN 1 AND 64),
    CONSTRAINT console_keys_lives_exactly_thirty_days CHECK (
        expires_at = paired_at + INTERVAL '720 hours'
    )
);

CREATE UNIQUE INDEX console_keys_thumbprint_key ON console_keys (thumbprint);

-- "Every key of this member": the member's own list, and what the FK's cascade
-- asks on every staff delete.
CREATE INDEX console_keys_staff_member_idx ON console_keys (staff_member_id);

COMMENT ON TABLE console_keys IS
    'A person''s registered console signing key (VP-6a, ADR-0036): a P-256 public key whose private half lives non-extractable in a browser, paired by vaam through POST /svc/v1/console_keys or by an operator with vpay-server staff pair-key. /dash/v1 requests are COSE_Sign1 messages under it. Lives exactly 720 hours from paired_at, never extended; revoked_at is set once; never deleted.';
COMMENT ON COLUMN console_keys.public_key IS
    'The SEC1 uncompressed P-256 point (65 bytes, first byte 0x04) as unpadded base64url, 87 characters. A compressed point is refused, not normalised.';
COMMENT ON COLUMN console_keys.thumbprint IS
    'The RFC 9679 COSE key thumbprint (SHA-256, lower-case hex), computed by cratestack-cose and never taken from a caller. The first 16 characters are the kid a signed request carries.';
COMMENT ON COLUMN console_keys.expires_at IS
    'paired_at plus exactly 720 hours (console_keys_lives_exactly_thirty_days). A request signed under an expired key is the layer''s coarse 401.';
COMMENT ON COLUMN console_keys.revoked_at IS
    'When the key was revoked, or NULL. Set once; effective on the next request, because the resolver reads this row on every request.';
