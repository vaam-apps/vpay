-- staff_sso_attempts: one row per SSO sign-in in flight (ADR-0030).
--
-- WHAT THIS TABLE IS. When a staff member presses "Sign in with ..." the
-- dashboard asks vpay to start an OpenID Connect authorization-code flow
-- against the deployment's identity provider. vpay mints a state, a nonce and
-- a PKCE verifier, sends the first two (and the verifier's S256 challenge)
-- to the provider through the browser, and has to REMEMBER all three until the
-- provider sends the browser back. This is where it remembers them.
--
-- WHY IT IS NOT A `staff_sessions` ROW. `staff_sessions.staff_id` is NOT NULL
-- and a real foreign key, and at this point in an SSO sign-in there IS no
-- staff member: the first sign-in of a person who has never been here
-- provisions one (ADR-0030, just-in-time provisioning), and nobody can say who
-- until the provider has answered. Making that column nullable to hold a
-- half-built sign-in would put a `NULL` into the one column every session
-- read relies on, and give an unauthenticated route a way to write rows into
-- the table a bearer token is looked up in. A separate table keeps
-- `staff_sessions` exactly what it was: a row exists only for a person.
--
-- A SUCCESSFUL CALLBACK MINTS A FRESH SESSION TOKEN rather than promoting the
-- one this row was keyed by. The token that identified an unauthenticated
-- browser's pending sign-in therefore never becomes a credential for an
-- authenticated one, which is session fixation closed by construction.
--
-- SHAPE: migration 0035's, to the letter. No DEFAULT on any column a writer
-- names, no native enum, no bytea, no jsonb, no seq.

CREATE TABLE staff_sso_attempts (
    -- The SHA-256 of the pending-flow token, lower-case hex — never the token.
    -- The token travels in the dashboard's httpOnly cookie and in
    -- `X-Vpay-Staff-Session`, exactly as a session token does, and for the
    -- same reason a dump of this table yields nothing presentable.
    id TEXT PRIMARY KEY,

    -- The SHA-256 of the `state` parameter sent to the provider. The raw
    -- value is only ever in the browser's redirect; the callback hashes what
    -- it is handed and compares in constant time. Hashed so a dump cannot
    -- forge a callback for a flow still in flight.
    state_hash TEXT NOT NULL,

    -- The SHA-256 of the `nonce` sent to the provider. The id_token must echo
    -- the raw nonce; the callback hashes the claim and compares. Only the
    -- comparison is ever needed, so only a hash is kept.
    nonce_hash TEXT NOT NULL,

    -- The PKCE code_verifier, SEALED with AES-256-GCM under
    -- `staff_auth.totp_encryption_key` (the same sealing the TOTP secret
    -- gets). It cannot be hashed: the token request has to present the raw
    -- value. A database dump alone therefore yields no verifier; and a
    -- verifier without the one-time authorization code is useless in any case.
    code_verifier TEXT NOT NULL,

    -- WHETHER THIS FLOW IS ITSELF THE `prompt=login` RETRY. The callback
    -- re-runs the provider's login forms ONCE when a token's only defect is
    -- that it does not prove a second factor (ADR-0030: a provider session
    -- re-entered from its cookie, or a first sign-in that enrolled its OTP
    -- through a required action, carries a password-only `amr`). A retry that
    -- fails the same rule is refused: at most one retry, no loop. This is the
    -- bit that makes "at most one" a property of the data.
    is_retry BOOLEAN NOT NULL,

    created_at TIMESTAMPTZ NOT NULL,

    -- Ten minutes after creation (set by `vpay_db::staff_sso_attempts`). A
    -- person completing a login form at the provider needs minutes, not
    -- hours, and every minute this row lives is a minute a leaked `state`
    -- could be replayed into it.
    expires_at TIMESTAMPTZ NOT NULL,

    CONSTRAINT staff_sso_attempts_id_length CHECK (char_length(id) = 64),
    CONSTRAINT staff_sso_attempts_state_hash_length CHECK (char_length(state_hash) = 64),
    CONSTRAINT staff_sso_attempts_nonce_hash_length CHECK (char_length(nonce_hash) = 64),
    CONSTRAINT staff_sso_attempts_code_verifier_length CHECK (char_length(code_verifier) BETWEEN 1 AND 1024),
    CONSTRAINT staff_sso_attempts_lifetime_is_positive CHECK (expires_at > created_at)
);

-- The index an expiry sweep needs. `vpay_db::staff_sso_attempts` deletes
-- expired rows opportunistically on every start, because `start` is an
-- unauthenticated write path and an unswept table is one a caller fills.
CREATE INDEX staff_sso_attempts_expires_idx ON staff_sso_attempts (expires_at);

COMMENT ON TABLE staff_sso_attempts IS
    'An SSO sign-in in flight (ADR-0030): the hashed state and nonce and the sealed PKCE verifier of one authorization request, until the provider sends the browser back. Single use (consumed by DELETE), ten-minute lifetime. Not a staff_sessions row: no staff member exists yet when a first sign-in begins.';
COMMENT ON COLUMN staff_sso_attempts.code_verifier IS
    'The PKCE code_verifier, AES-256-GCM sealed under staff_auth.totp_encryption_key. Sealed rather than hashed because the token request must present the raw value.';
COMMENT ON COLUMN staff_sso_attempts.is_retry IS
    'True when this flow is the single prompt=login retry the callback starts when a token''s only defect is a missing second factor. A retry that fails the same rule is refused.';
