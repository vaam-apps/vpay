-- `staff_sessions.access_token_expires_at`: when the `/dash/v1` token on this
-- row stops being accepted, so the dashboard can replace it BEFORE it does
-- (issue #88 item 1).
--
-- WHAT WAS WRONG WITHOUT IT
--
-- The token's TTL is 900 seconds and a session's bounds are ADR-0017 decision
-- 2's: thirty minutes idle, twelve hours absolute. `requireStaff` ran the
-- authorization-code leg only when the row carried NO token, so once one was
-- written it was used until sign-out — and a quarter of an hour into every
-- session `/dash/v1` began answering
--
--     The bearer token is invalid, expired, or was not issued for this endpoint.
--
-- on every render. Measured against the real stack (the exp28 review, finding
-- F4). `dash-read.ts` then learned to re-mint REACTIVELY, on the `401`, which
-- fixed the symptom at the cost of a failed request in front of every read
-- once a token has died. This column is what lets the decision be taken
-- before the failure instead of after it: the session read carries the
-- expiry, and the app re-mints when less than a fifth of the TTL is left.
--
-- NULLABLE, AND PAIRED WITH THE TOKEN
--
-- `access_token` is nullable — a session between the second factor and the
-- code exchange has none — and this column is NULL exactly when it is. That
-- is one fact with two columns, so the CHECK below is what stops a code path
-- writing a token with no expiry (which would read as "already expired", a
-- re-mint on every render) or an expiry with no token.
--
-- The constraint is MULTI-COLUMN and therefore INVISIBLE to
-- `cratestack migrate baseline` in both directions
-- (`introspect/postgres/constraints.rs` filters `array_length(c.conkey, 1) = 1`),
-- exactly as `staff_members_totp_is_paired` is. `postgres_smoke.rs` asserts it
-- against a real Postgres for that reason: the drift report cannot be the
-- guard here.
--
-- NO DEFAULT, like every column added since 0034: `cratestack-macros` drops
-- every `@default(...)` field from `Create{Model}Input` and from
-- `upsert_update_columns`, which is what 0033 had to undo for `providers`.
--
-- WHY THE EXISTING TOKENS ARE FORGOTTEN RATHER THAN GIVEN AN EXPIRY
--
-- A row written before this migration carries a token whose expiry nothing
-- recorded, and there is no honest value to invent for it: the TTL may have
-- changed, and guessing 900 seconds from `last_seen_at` would write down a
-- time that is not when that token expires. So the token is cleared instead.
--
-- That costs nothing a caller can see. A session with no token is the state
-- every session is in between the second factor and the first render, and
-- `vpay_api::staff::oauth::authorize` mints a replacement on the next one —
-- re-reading the staff row, the active status, the merchant binding and
-- `password_change_required` while it does, which is MORE checking than
-- carrying the old token would have been. Nobody is signed out: the session
-- row, which is what the cookie names, is untouched.
--
-- The UPDATE runs BEFORE the constraint is added, and it has to: the CHECK is
-- validated against every existing row, and a live session holding a token
-- with no expiry is exactly the row it would refuse.

ALTER TABLE staff_sessions
    ADD COLUMN access_token_expires_at TIMESTAMPTZ;

UPDATE staff_sessions
   SET access_token = NULL
 WHERE access_token IS NOT NULL;

ALTER TABLE staff_sessions
    ADD CONSTRAINT staff_sessions_token_expiry_is_paired CHECK (
        (access_token IS NULL) = (access_token_expires_at IS NULL)
    );

COMMENT ON COLUMN staff_sessions.access_token_expires_at IS
    'When the /dash/v1 token in access_token stops being accepted. NULL exactly when that column is. Read by GET /dash/v1/staff/session so the dashboard app can re-mint BEFORE the expiry rather than after a 401 (issue #88 item 1); the token itself carries the same instant in its own `exp`, and this is the copy a reader can see without verifying a JWT.';
