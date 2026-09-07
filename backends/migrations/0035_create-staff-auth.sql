-- staff, staff_sessions, oauth_authorization_codes: the three tables that
-- turn `/dash/v1` from a resource server with no issuer into a surface a
-- human can sign in to (ADR-0017).
--
-- WHY THESE THREE AND NOT ONE
--
-- They answer three different questions and have three different lifetimes.
-- `staff_members` is who a person is and survives every session; `staff_sessions` is
-- "this browser is that person, until 12 hours from now"; and
-- `oauth_authorization_codes` is "this 60-second string may be exchanged
-- once, by this client, for a token naming that person". Collapsing any two
-- of them would tie a credential's lifetime to an identity's.
--
-- ALL THREE ARE BORN WITH A `schemas/vpay.cstack` MODEL, and that is what
-- shapes the DDL below rather than the other way round. `customers` (0034)
-- established the rule and this migration follows it to the letter:
--
--   * NO COLUMN THIS DEPLOYMENT'S CODE EVER WRITES CARRIES A `DEFAULT`.
--     `cratestack-macros` drops every `@default(...)` field from
--     `Create{Model}Input` and from `upsert_update_columns`
--     (`model/inputs.rs:20-26`, `model/descriptor/columns.rs:92-101`), which
--     is what migration 0033 had to undo for `providers` after the fact. So
--     `created_at` and `updated_at` here are written by the writer and are
--     NOT `DEFAULT now()`, unlike every table before 0034.
--   * NO `seq` IDENTITY COLUMN. None of these three is paginated — the
--     dashboard lists payments, never staff — so there is no cursor to
--     order, and adding one would cost a permanent `column seq default value
--     differs` drift line for a column nothing reads.
--   * NO NATIVE POSTGRES ENUM. CrateStack decodes every enum column with
--     `try_get::<String>()` and a native enum fails to decode on every read
--     (`emit/postgres/columns.rs`, upstream #228) — the defect migration
--     0032 had to convert `providers.flow` out of. `staff_members.status` and
--     `staff_sessions.state` are `TEXT` with a hand-named CHECK, exactly as
--     `events.type` and `jobs.kind` are, and for the same reason: the
--     vocabulary moves in lockstep with the code that writes it.
--   * NO `BYTEA`. `bytea` is not in `cratestack-migrate`'s `map_scalar`, so a
--     `BYTEA` column is excluded from the drift comparison outright and
--     cannot be named in a generated input. The encrypted TOTP secret is
--     therefore stored as base64url `TEXT` — a rendering choice forced by the
--     data layer and stated here so nobody "fixes" it later.
--
-- WHAT THIS MIGRATION DELIBERATELY DOES NOT ADD
--
--   * No `audit_log`. ADR-0008 wants one row per dashboard *write*, and this
--     slice mounts no dashboard write at all (`vpay_api::require_dashboard_token`
--     refuses every non-read method before the router matches). A table with
--     no writer is a claim nothing checks.
--   * No `staff_merchants` join table. A staff member belongs to exactly one
--     merchant in this slice (ADR-0017 decision 1); a join table would be the
--     shape of a decision nobody has taken.
--   * No password history, no lockout counter. Rate limiting is in-process
--     and fixed-window (ADR-0017 decision 2); a durable lockout is a denial
--     of service an attacker triggers by guessing at somebody else's email.

-- WHO A STAFF MEMBER IS.
--
-- `staff_members` AND NOT `staff`, and the name is decided by the data layer
-- rather than by taste. CrateStack derives a table name from a model name with
-- `cratestack_core::route_naming::pluralize`, which is
-- `format!("{value}s")` for anything not ending in `s` or a consonant + `y` —
-- so `model Staff` reads and writes a table called `staffs`. Renaming the
-- MODEL is the only way to change that (0.11.1 has no `@@map`), and
-- `StaffMember` pluralises to a word English also uses. Measured: the first
-- draft of this migration created `staff`, every query answered
-- `relation "staffs" does not exist`, and no gate said anything until a
-- container-backed test ran.
--
-- The Rust trait is still `vpay_db::Staff`, because it is a trait about staff
-- and not about a table.
CREATE TABLE staff_members (
    -- `stf_…`, minted by `vpay_core::ids::staff_id` exactly as `cus_…` and
    -- `pi_…` are, before the insert.
    id TEXT PRIMARY KEY,

    -- The one merchant whose rows this person may see. No FK: there is no
    -- merchants table (ADR-0003) — the same absence `customers.merchant_id`
    -- records. It is checked against `merchant_clients[].merchant_id` by the
    -- CLI that creates the row, so a typo is a refused `staff add` rather
    -- than an account that can never see anything.
    --
    -- SINGULAR, and that is ADR-0017 decision 1 rather than a simplification:
    -- `/dash/v1`'s tenant comes from `dashboard_client.merchant_id`, and the
    -- token's merchant claim has to agree with it or the request is refused.
    -- A staff member with two merchants would need an authorisation decision
    -- per request against a list nothing validates.
    merchant_id TEXT NOT NULL,

    -- Lower-cased at the API and CLI boundary, never here: a `lower(email)`
    -- functional unique index would let two writers disagree about the
    -- canonical form while the database silently agreed with neither. The
    -- column is the canonical form, the unique index below is plain, and
    -- `email_is_lower_case` refuses a writer that forgot.
    --
    -- UNIQUE ACROSS THE WHOLE DEPLOYMENT, not per merchant. Sign-in names an
    -- email and no tenant — a staff member types an address, not an address
    -- and a merchant id — so two rows sharing an address would make "which
    -- account is this?" a question the login form cannot answer.
    email TEXT NOT NULL,

    -- What the dashboard greets them by. Not an identifier and never matched
    -- on.
    display_name TEXT NOT NULL,

    -- An argon2id PHC string (`$argon2id$v=19$m=…,t=…,p=…$salt$hash`),
    -- produced by `vpay_api::staff_auth::password`. The salt and the cost
    -- parameters travel inside it, which is the whole point of the PHC
    -- format: a row written under today's parameters is still verifiable
    -- after they are raised.
    --
    -- THE PEPPER IS NOT IN THIS COLUMN AND MUST NEVER BE. It is a
    -- deployment secret (`staff_auth.password_pepper`, refused at boot in
    -- livemode if absent) mixed in as argon2's secret input, so a stolen
    -- database dump is not by itself an offline cracking target. That also
    -- means: LOSING THE PEPPER INVALIDATES EVERY ROW IN THIS COLUMN. It
    -- belongs in the same Secret as the signing key and has the same
    -- backup story.
    password_hash TEXT NOT NULL,

    -- True from `staff add` until the person picks their own password. Every
    -- authenticated route refuses a session whose staff row still has this
    -- set, so the one-time password the CLI prints cannot become a
    -- long-lived credential by being ignored.
    password_change_required BOOLEAN NOT NULL,

    -- The RFC 6238 shared secret, AES-256-GCM sealed under
    -- `staff_auth.totp_encryption_key` and rendered base64url — nonce
    -- prepended, exactly as `vpay_api::staff_auth::secret_box` writes it.
    -- NULL until enrolment, which is mandatory at first sign-in: a session
    -- never reaches `authenticated` while this is NULL.
    --
    -- ENCRYPTED RATHER THAN HASHED because TOTP verification needs the
    -- secret itself; there is no one-way form of it that still works. The
    -- key is a second deployment secret and is deliberately not the pepper:
    -- one is an argon2 secret input, the other an AEAD key, and reusing one
    -- value for both would mean rotating either forces rotating both.
    totp_secret TEXT,

    -- When enrolment completed. NULL exactly when `totp_secret` is —
    -- `totp_is_paired` below makes that an invariant rather than a habit.
    totp_enrolled_at TIMESTAMPTZ,

    -- The RFC 6238 time step of the LAST ACCEPTED code, and the whole
    -- replay guard: a code is accepted only if its step is strictly greater
    -- than this, so the ±1-step window that makes TOTP usable across a
    -- clock skew cannot be used to present the same six digits twice.
    -- NULL until the first accepted code.
    --
    -- A step number rather than the code itself, deliberately: storing the
    -- last code would refuse a *replay* and admit the neighbouring step an
    -- attacker who shoulder-surfed one code can compute anyway from the
    -- same window.
    --
    -- `NOT NULL`, seeded to 0 by the writer, and that is a data-layer
    -- decision with a security consequence rather than a tidiness one. The
    -- guard is a compare-and-swap — `update_many().where_(id)
    -- .where_(last_totp_step().lt(step))` — and in SQL `NULL < 12345` is
    -- NULL, not true, so a nullable column would make the FIRST code of
    -- every staff member match zero rows and be refused forever. 0 is the
    -- Unix epoch's step and is below every real one.
    last_totp_step BIGINT NOT NULL,

    -- 'active' or 'disabled'. TEXT + CHECK, never a native enum — see the
    -- header. A disabled staff member is refused at sign-in AND at every
    -- session read, so disabling one takes effect on their next request
    -- rather than at their next login.
    status TEXT NOT NULL,

    -- Written by `vpay_db::staff`, never by a trigger or a DEFAULT: 0034's
    -- rule, and the reason is the same one — a value filled in by something
    -- no code names is a value nobody can explain.
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,

    -- When this person last completed a full sign-in (password AND TOTP).
    -- NULL until they have. Not "last seen": that is
    -- `staff_sessions.last_seen_at`, and conflating the two would make an
    -- idle-session touch look like a fresh authentication in an audit.
    last_sign_in_at TIMESTAMPTZ,

    CONSTRAINT staff_members_id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT staff_members_merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),
    CONSTRAINT staff_members_email_length CHECK (char_length(email) BETWEEN 3 AND 512),
    CONSTRAINT staff_members_display_name_length CHECK (char_length(display_name) BETWEEN 1 AND 256),

    -- The canonical form is lower case, and the database is what refuses a
    -- writer that did not canonicalise. Without it, `Staff::find_by_email`
    -- — a plain `email = $1` — would answer "no such account" for a person
    -- whose row was written with a capital letter, which is a sign-in
    -- failure with no diagnosis at all.
    CONSTRAINT staff_members_email_is_lower_case CHECK (email = lower(email)),

    -- MULTI-COLUMN, therefore INVISIBLE to `cratestack migrate baseline` in
    -- both directions (`introspect/postgres/constraints.rs` filters
    -- `array_length(c.conkey, 1) = 1`), exactly as `customers`'
    -- `at_least_one_identifier` is. `postgres_smoke.rs` asserts it directly
    -- against a real Postgres for that reason: the drift report cannot be
    -- the guard here.
    --
    -- What it buys: "enrolled" is one fact with two columns, so no code path
    -- can leave a secret behind with no enrolment date or claim an enrolment
    -- with no secret. `Staff::enrol_totp` writes both in one statement.
    CONSTRAINT staff_members_totp_is_paired CHECK (
        (totp_secret IS NULL) = (totp_enrolled_at IS NULL)
    ),

    -- The vocabulary, closed. `vpay_db::staff::StaffStatus` mirrors it
    -- exactly; a value spelled here and not there is a row no code can
    -- decode, and one spelled there and not here is refused at the insert.
    CONSTRAINT staff_members_status_is_known CHECK (status IN ('active', 'disabled')),

    -- A step is never negative: it is `unix_seconds / 30`, and the seed is 0.
    CONSTRAINT staff_members_totp_step_is_not_negative CHECK (last_totp_step >= 0)
);

-- Sign-in names an email and nothing else — see the column comment.
CREATE UNIQUE INDEX staff_members_email_key ON staff_members (email);

-- "Who can see this merchant's dashboard?", the only other way this table is
-- ever read. Not unique: a merchant may have several staff.
CREATE INDEX staff_members_merchant_idx ON staff_members (merchant_id);

COMMENT ON TABLE staff_members IS
    'A human who may sign in to /dash/v1 (ADR-0017). Created only by `vpay-server staff add`; no HTTP endpoint creates one. Belongs to exactly one merchant. Credentials: argon2id password (peppered from configuration) plus mandatory RFC 6238 TOTP.';
COMMENT ON COLUMN staff_members.password_hash IS
    'argon2id PHC string. The pepper is NOT in it: it is a deployment secret (staff_auth.password_pepper) mixed in as argon2''s secret input, so losing the pepper invalidates every row in this column.';
COMMENT ON COLUMN staff_members.totp_secret IS
    'The RFC 6238 shared secret, AES-256-GCM sealed under staff_auth.totp_encryption_key, nonce-prefixed and base64url-rendered. Encrypted rather than hashed because verification needs the secret itself. NULL until enrolment, which is mandatory at first sign-in.';
COMMENT ON COLUMN staff_members.last_totp_step IS
    'The RFC 6238 time step of the last ACCEPTED code. A code is accepted only if its step is strictly greater, which is what stops the +/-1-step skew window being used to replay the same six digits.';

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

    CONSTRAINT staff_members_sessions_id_length CHECK (char_length(id) = 64),
    CONSTRAINT staff_members_sessions_state_is_known CHECK (state IN ('pending_totp', 'authenticated'))
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
