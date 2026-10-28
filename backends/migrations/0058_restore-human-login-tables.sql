-- staff_sessions, staff_sso_attempts, oauth_authorization_codes: the three
-- tables migration 0056 dropped, created again, EMPTY, with the DDL they last
-- shipped with (VPA-2, owner ruling R5, 2026-10-27).
--
-- WHY THEY COME BACK. vpay serves applications beyond vaam. Which ways in a
-- deployment permits is configuration (`auth.methods`, VPA-1), and the default
-- keeps every legacy way, human login among them: a staff member who signs in
-- with a password and a TOTP code, or through an OIDC provider, holds a
-- session, reaches `/dash/v1` through an authorization code, and starts an SSO
-- flow from a row of these three tables. A deployment that sets `auth.methods`
-- to `[keypair]` never writes one.
--
-- WHY A NEW MIGRATION AND NOT AN EDIT OF 0056. The owner chose forward-only
-- migrations. 0056 ran in deployments and its bytes are pinned by
-- MANIFEST.sha256; what it dropped is created here, and nothing it added
-- (`console_keys`) is touched.
--
-- THE DDL IS THE ORIGINALS', TO THE LETTER, in the one order the foreign keys
-- allow: `staff_sessions` (0035 with 0040's `access_token_expires_at` and its
-- paired CHECK folded into the CREATE, because there is no live row to
-- backfill), `oauth_authorization_codes` (0035; `session_id` names a
-- `staff_sessions` row, which is why 0056 dropped it first), and
-- `staff_sso_attempts` (0050). `staff_members` was never dropped and is not
-- touched. Each statement below is the original's, comments included, so the
-- reasoning that shaped it is read where it is written.
--
-- WHAT IS NOT COMING BACK, AND A DEPLOYMENT THAT RAN 0056 MUST KNOW IT.
--
--   * The rows. 0056 dropped the three tables, and their rows with them: every
--     session, every authorization code and every SSO flow in flight was
--     deleted, and these tables start empty. All three are short-lived by
--     construction (a code lives minutes, a session twelve hours, an SSO flow
--     ten minutes), so only the sign-out is felt.
--   * The `password` and `totp` credentials. 0056 ran
--     `DELETE FROM credentials WHERE kind IN ('password', 'totp')` and this
--     migration cannot undo a delete. A staff member who signed in with a
--     password and a TOTP code must be enrolled again (a new one-time password
--     and a new authenticator), and an upgrader should plan for that.
--   * The `oidc` credentials were KEPT by 0056: an SSO staff member signs in
--     again as before and needs nothing.
--
-- THE CONSTRAINT. 0056 closed the two credential names it retired with
-- `credentials_password_and_totp_are_retired CHECK (kind NOT IN ('password',
-- 'totp'))`. It is dropped here, so a `password` or `totp` row can be written
-- again; the eight-name `credentials_kind_is_known` CHECK (0044) was never
-- touched and is the vocabulary again.

ALTER TABLE credentials
    DROP CONSTRAINT credentials_password_and_totp_are_retired;

-- THAT A BROWSER IS THAT PERSON.
CREATE TABLE staff_sessions (
    -- The SHA-256 of the opaque session token, lower-case hex — NEVER the
    -- token itself. The token is 32 CSPRNG bytes rendered base64url and is
    -- returned to the caller exactly once; a dump of this table therefore
    -- yields no usable session, which is the same property
    -- `oauth_authorization_codes.code_hash` has and the same one
    -- `idempotency_keys` established for a request hash.
    id TEXT PRIMARY KEY,

    -- Whose session it is. A real FK, unlike `staff_members.merchant_id`, because
    -- `staff_members` is a table this schema owns. ON DELETE CASCADE: a staff row
    -- that goes away takes its sessions with it, which is the only sane
    -- reading of "this account no longer exists".
    staff_id TEXT NOT NULL REFERENCES staff_members (id) ON DELETE CASCADE,

    -- 'pending_totp' -> 'authenticated'. Two states and no third: a session
    -- that has presented a password but not a code can do exactly one thing
    -- (present a code), and `/dash/v1/oauth/authorize` refuses it.
    --
    -- The password-change step does NOT get a state of its own: it is read
    -- off `staff_members.password_change_required`, so a session cannot be
    -- promoted past it by anything that writes this column.
    state TEXT NOT NULL,

    created_at TIMESTAMPTZ NOT NULL,

    -- The ABSOLUTE bound, 12 hours from creation and never extended. This is
    -- the one an attacker holding a stolen session cannot outrun by using
    -- it, which is exactly what makes it different from `last_seen_at`.
    expires_at TIMESTAMPTZ NOT NULL,

    -- The IDLE bound's input: a session is refused once this is more than 30
    -- minutes old, and every accepted request moves it forward. Both bounds
    -- are enforced in `vpay_db::staff_sessions::load`, which reads the row
    -- and refuses rather than deleting — a delete would make "expired" and
    -- "never existed" indistinguishable to an operator reading the table.
    last_seen_at TIMESTAMPTZ NOT NULL,

    -- The `/dash/v1` access token minted for this session by the
    -- authorization-code exchange, or NULL before it. NOT a second
    -- credential: it is the same short-lived bearer token the exchange
    -- returned to the dashboard's own server, kept here so that DELETING
    -- THE SESSION ROW REVOKES IT — which is the deny-list ADR-0009's
    -- Consequences section left open ("which of these vpay will actually
    -- build is not decided by this ADR"). ADR-0017 decides it.
    --
    -- Stored as issued, not hashed: the dashboard's server has to be able to
    -- present it, so there is no one-way form that still works. That is a
    -- real residual — a database dump yields live tokens for at most the
    -- access-token TTL — and it is written down in ADR-0017's Consequences
    -- rather than glossed.
    access_token TEXT,

    -- When that token stops being accepted: migration 0040's column, created
    -- here with the table. NULL exactly when `access_token` is
    -- (`staff_sessions_token_expiry_is_paired`).
    access_token_expires_at TIMESTAMPTZ,

    CONSTRAINT staff_sessions_id_length CHECK (char_length(id) = 64),
    CONSTRAINT staff_sessions_state_is_known CHECK (state IN ('pending_totp', 'authenticated')),
    CONSTRAINT staff_sessions_token_expiry_is_paired CHECK (
        (access_token IS NULL) = (access_token_expires_at IS NULL)
    )
);

-- "Which sessions belong to this staff member?" — what sign-out-everywhere
-- and a disabled account would ask. Nothing asks it today; the index is here
-- because the FK's own delete does, and an unindexed FK turns a `DELETE FROM
-- staff` into a sequential scan of every session.
CREATE INDEX staff_sessions_staff_idx ON staff_sessions (staff_id);

-- The sweep's backlog query, ordered by the column the horizon is compared
-- against. Nothing sweeps this table yet — see docs/status.md; expired rows
-- are refused on read, not deleted on a schedule — and this index is what a
-- sweep would need rather than evidence that one exists.
CREATE INDEX staff_sessions_expiry_idx ON staff_sessions (expires_at);

COMMENT ON TABLE staff_sessions IS
    'A signed-in browser (ADR-0017). Keyed by the SHA-256 of the opaque session token, never the token. Two bounds: absolute 12h (expires_at, never extended) and idle 30min (last_seen_at). Sign-out deletes the row, which revokes the access token it carries.';
COMMENT ON COLUMN staff_sessions.access_token IS
    'The /dash/v1 access token this session''s authorization-code exchange minted, or NULL. Kept here so deleting the session revokes it — the server-side deny-list ADR-0009 left undecided and ADR-0017 decides. Stored as issued because the caller must present it.';

COMMENT ON COLUMN staff_sessions.access_token_expires_at IS
    'When the /dash/v1 token in access_token stops being accepted. NULL exactly when that column is. Read by GET /dash/v1/staff/session so the dashboard app can re-mint BEFORE the expiry rather than after a 401 (issue #88 item 1); the token itself carries the same instant in its own `exp`, and this is the copy a reader can see without verifying a JWT.';

-- THAT THIS STRING MAY BE EXCHANGED ONCE.
--
-- WHY A VPAY-OWNED TABLE AND NOT `authkestra.oauth_codes`
--
-- `authkestra.oauth_codes` (migration 0006) is a byte-faithful transcription
-- of `authkestra-op`'s own DDL for `SqlxOpStore`, and `SqlxOpStore` is not a
-- free choice any more: it sits behind the `sqlx-postgres` feature, which
-- pins `sqlx ^0.8`, and this workspace moved to `=0.9.0` so that CrateStack
-- and `vpay-db` could share a transaction. Reviving it would take the whole
-- workspace back a major. This table is vpay's own, modelled in
-- `schemas/vpay.cstack`, read and written through CrateStack like every
-- other table born after 0034 — and it carries two columns authkestra's
-- shape has nowhere to put: `session_id` and `merchant_id`.
CREATE TABLE oauth_authorization_codes (
    -- SHA-256 of the code, lower-case hex. `staff_sessions.id`'s argument
    -- verbatim: the code is handed out once and never stored.
    code_hash TEXT PRIMARY KEY,

    -- The client the code was issued to, checked again at the exchange
    -- (`default_handle_authorization_code` step 4). Not an FK: clients are
    -- YAML (ADR-0003), not rows.
    client_id TEXT NOT NULL,

    -- Who the code speaks for. Becomes the token's `sub`.
    staff_id TEXT NOT NULL REFERENCES staff_members (id) ON DELETE CASCADE,

    -- WHICH SESSION AUTHORISED IT, and the reason this table cannot be
    -- authkestra's. A code outlives nothing: if the session that produced it
    -- is signed out before the exchange, the code must die with it. ON
    -- DELETE CASCADE makes sign-out do that in one statement.
    session_id TEXT NOT NULL REFERENCES staff_sessions (id) ON DELETE CASCADE,

    -- The tenant that will be stamped into the token's merchant claim. A
    -- copy of `staff_members.merchant_id` taken when the code was issued, so that a
    -- staff row edited between issue and exchange cannot silently move a
    -- token to another tenant.
    merchant_id TEXT NOT NULL,

    -- Space-delimited, as granted. Checked against the registration at
    -- `/authorize` (`handle_authorize` step 3) before it lands here.
    scope TEXT NOT NULL,

    -- PKCE, MANDATORY. Both columns are `NOT NULL` — not `Option` mirrored
    -- into the database — because `default_handle_authorization_code`
    -- refuses a stored code with no challenge, and a nullable column here
    -- would be a shape whose only legal value the exchange rejects.
    -- `code_challenge_method` is constrained to `S256` for the same reason:
    -- the exchange refuses anything else with `server_error`, so a row
    -- carrying `plain` is a row that can only ever produce a 500.
    code_challenge TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL,

    -- Matched byte for byte at the exchange (step 5). Stored rather than
    -- re-derived from configuration because that is what makes the match
    -- mean something: it is the URI the *authorization request* named.
    redirect_uri TEXT NOT NULL,

    -- OIDC's replay nonce, echoed into the id token when one is issued.
    nonce TEXT,

    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,

    -- SINGLE USE, and this one nullable column is the whole enforcement.
    -- The consume is a compare-and-swap through CrateStack —
    -- `update_many().where_(code_hash).where_(consumed_at().is_null())
    -- .set(consumed_at)` — so `BatchSummary::ok == 1` is the atomic answer to
    -- "did I win the race", which is the property
    -- `AuthorizationCodeStore::consume_code`'s own doc calls "the single most
    -- important correctness property in this crate". A check-then-mark in
    -- Rust would be the TOCTOU that doc names.
    --
    -- ONE COLUMN AND NOT A `consumed BOOLEAN` BESIDE IT. A boolean would need
    -- a cross-column CHECK to keep the two in step, that CHECK would be
    -- multi-column and therefore invisible to `cratestack migrate baseline`,
    -- and the pair would carry no fact this column does not: "spent" and
    -- "spent at" are the same fact.
    consumed_at TIMESTAMPTZ,

    CONSTRAINT oauth_authorization_codes_code_hash_length CHECK (char_length(code_hash) = 64),
    CONSTRAINT oauth_authorization_codes_method_is_s256 CHECK (code_challenge_method = 'S256')
);

-- The exchange looks a code up by its hash (the primary key). This index is
-- for the *other* question — "what did this session issue?" — which the
-- cascade above asks on every sign-out, and which is a sequential scan
-- without it.
CREATE INDEX oauth_authorization_codes_session_idx
    ON oauth_authorization_codes (session_id);

-- What a sweep would need. Nothing sweeps this table either: an expired code
-- is refused by `expires_at` at the exchange, and the sign-out cascade is
-- what actually removes rows today. See docs/status.md.
CREATE INDEX oauth_authorization_codes_expiry_idx
    ON oauth_authorization_codes (expires_at);

COMMENT ON TABLE oauth_authorization_codes IS
    'vpay''s own authorization-code store for the dashboard client only (ADR-0017), replacing the refusing store for that one grant. Keyed by the SHA-256 of the code. Single use, enforced by a compare-and-swap on `consumed_at`. Cascades from the session that issued it, so signing out kills a code in flight. NOT authkestra.oauth_codes: that table serves SqlxOpStore, which pins sqlx 0.8.';
COMMENT ON COLUMN oauth_authorization_codes.merchant_id IS
    'The tenant stamped into the minted token''s merchant claim. Copied from staff.merchant_id at issue, so editing a staff row between issue and exchange cannot move a token to another tenant.';

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
