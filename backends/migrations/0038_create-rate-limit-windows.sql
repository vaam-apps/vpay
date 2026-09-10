-- `rate_limit_windows`: the fixed-window attempt counters the staff sign-in
-- limiter spends from, moved out of one process's memory and into the
-- database so that every replica shares one budget (issue #79 item 2,
-- ADR-0017 decision 2 as amended 2026-09-10).
--
-- WHAT THIS REPLACES, AND WHAT IT COSTS
--
-- `vpay_api::staff::rate_limit::SignInLimiter` was a `Mutex<HashMap<..>>`.
-- ADR-0017's Consequences stated the cost of that in one sentence — "**The
-- rate limit is per replica.** Three replicas admit three times the attempts
-- a single one does" — and named the alternative in the next: "a shared
-- counter in Postgres … puts a write on the unauthenticated path, which is a
-- denial-of-service amplifier of a different kind".
--
-- Both halves of that trade are real and this table takes the second one
-- deliberately, with the amplifier bounded rather than accepted:
--
--   * THE KEY IS A DIGEST, NEVER THE VALUE. `id` is the SHA-256 of
--     `<action>:<dimension>:<value>`, lower-case hex, exactly as
--     `staff_sessions.id` and `oauth_authorization_codes.code_hash` are. An
--     unauthenticated caller chooses the *value* — an email address, on a
--     route that has no account for it — so a column holding that value
--     verbatim would be a column an attacker sizes. It is also PII this
--     table has no reason to hold: an address that never had an account
--     would otherwise be written here by the act of guessing it.
--   * THE ROW COUNT IS BOUNDED BY A SWEEP IN THE SAME STATEMENT.
--     `vpay_db::rate_limits` deletes up to 32 elapsed rows on every count,
--     which is sixteen times the two rows one attempt adds, so an attacker
--     spending fresh keys drains the table faster than they fill it. The
--     sweep is `FOR UPDATE SKIP LOCKED` and never names the row the same
--     statement is about — see that module for both reasons.
--   * ONE ROUND TRIP, ONE ROW LOCK. The count is a single
--     `INSERT … ON CONFLICT (id) DO UPDATE … RETURNING attempts`, so two
--     replicas incrementing one key serialise on the row rather than racing.
--     That is the property the whole change exists for and it is the reason
--     this is not a read-then-write.
--
-- BORN WITH A `schemas/vpay.cstack` MODEL, and 0034's rule applies to it in
-- full: NO `DEFAULT` on any column a writer names, no `seq` identity column,
-- no native enum, no `bytea`, no `jsonb`. `model RateLimitWindow` declares
-- every column of it.
--
-- WHAT IT DELIBERATELY DOES NOT DO
--
--   * NO `@@allow` ARM ON THE MODEL, and that is the load-bearing half of
--     the sentence above. Nothing reads or writes this table through the
--     generated data layer: the count has to be `attempts = attempts + 1`
--     inside an `ON CONFLICT DO UPDATE`, and a CrateStack 0.12.0
--     `Update…Input` carries values rather than expressions, so there is no
--     builder that renders it. The model exists so that the drift report
--     compares this table column by column, exactly as `model PaymentIntent`
--     does for `payment_intents`, and
--     `the_rate_limit_window_model_answers_no_rows_to_every_action` pins the
--     arms' ABSENCE for the same reason `schema.rs` pins the money models':
--     an arm appearing here would be a standing permission over a table
--     nothing asks for.
--   * NO LOCKOUT COLUMN. A fixed window, still — `rate_limit`'s own header
--     argues it, and durability does not change the argument: a durable
--     lockout is a denial of service an attacker triggers by guessing at
--     somebody else's address, and it is precisely what a durable *counter*
--     must not become.
--   * NO FOREIGN KEY TO `staff_members`. The commonest key on this table is
--     an address with no account, which is the whole point of counting it.

CREATE TABLE rate_limit_windows (
    -- SHA-256 of `<action>:<dimension>:<value>`, lower-case hex. Never the
    -- value — see the header. `vpay_api::staff::rate_limit` is the only
    -- thing that composes the pre-image, and it is the only place the
    -- spelling is decided.
    id TEXT PRIMARY KEY,

    -- What kind of budget this row is, WITHOUT the value: `sign_in:email`,
    -- `sign_in:address`, `change_password:session`. A closed vocabulary,
    -- constrained below.
    --
    -- Here so that an operator reading the table can tell a per-email budget
    -- from a per-address one and count how many of each are live. Without it
    -- every row is 64 hex characters and the table says nothing at all —
    -- which is a real cost of hashing the key, paid rather than ignored.
    scope TEXT NOT NULL,

    -- When the current window opened. Replaced, never extended: a window
    -- that has elapsed is a NEW window starting now, which is what makes
    -- this fixed rather than sliding. `vpay_db::rate_limits::count_attempt`
    -- is the only writer and it decides the replacement in the statement.
    window_started_at TIMESTAMPTZ NOT NULL,

    -- Attempts inside the current window, including the one being counted:
    -- the writer's `RETURNING attempts` is always at least 1, so "over
    -- budget" is `attempts > limit` and never an off-by-one about whether
    -- the caller has been counted yet.
    --
    -- `BIGINT` and not `INT`, for `staff_members.last_totp_step`'s reason:
    -- `int4` is one of the Postgres types `cratestack-migrate`'s
    -- `map_scalar` does not map, and a column it cannot map is a column the
    -- drift report excludes from the comparison outright. `Int` in a
    -- `.cstack` model is `int8`.
    attempts BIGINT NOT NULL,

    -- The last time this row was counted against. Not the same fact as
    -- `window_started_at`: the window's start is what the horizon is
    -- compared against, and this is what an operator reads to see whether a
    -- budget is being spent right now.
    updated_at TIMESTAMPTZ NOT NULL,

    CONSTRAINT rate_limit_windows_id_length CHECK (char_length(id) = 64),
    CONSTRAINT rate_limit_windows_scope_length CHECK (char_length(scope) BETWEEN 1 AND 64),

    -- The vocabulary, closed, exactly as `staff_members_status_is_known`
    -- closes `staff_members.status`. `vpay_api::staff::rate_limit::Budget`
    -- mirrors it; a value spelled there and not here is refused at the
    -- insert rather than written and never understood.
    CONSTRAINT rate_limit_windows_scope_is_known CHECK (
        scope IN ('sign_in:email', 'sign_in:address', 'change_password:session')
    ),

    -- A counted attempt is at least one attempt. The writer's `RETURNING`
    -- contract in a CHECK: if this ever fires, the increment expression is
    -- wrong and the limiter is answering about a window it did not count.
    CONSTRAINT rate_limit_windows_attempts_is_positive CHECK (attempts >= 1)
);

-- THE SWEEP'S INDEX, and unlike `staff_sessions_expiry_idx` there is a sweep.
-- `count_attempt`'s statement selects the oldest elapsed rows by this column
-- on every call, so without this index every sign-in attempt in the
-- deployment costs a sequential scan of the table an attacker is filling.
CREATE INDEX rate_limit_windows_window_idx ON rate_limit_windows (window_started_at);

COMMENT ON TABLE rate_limit_windows IS
    'Fixed-window attempt counters shared by every vpay replica (issue #79 item 2, ADR-0017 decision 2 as amended 2026-09-10). Keyed by the SHA-256 of <action>:<dimension>:<value>, never the value: an unauthenticated caller chooses it. Counted by one INSERT ... ON CONFLICT DO UPDATE ... RETURNING attempts, which sweeps up to 32 elapsed rows in the same statement.';
COMMENT ON COLUMN rate_limit_windows.id IS
    'SHA-256 of <action>:<dimension>:<value>, lower-case hex. The pre-image is composed only by vpay_api::staff::rate_limit; the value never reaches this table, because an attacker chooses it and it is PII for an address that has no account.';
COMMENT ON COLUMN rate_limit_windows.attempts IS
    'Attempts in the current window INCLUDING the one being counted, so the writer''s RETURNING is always >= 1 and "over budget" is attempts > limit.';
