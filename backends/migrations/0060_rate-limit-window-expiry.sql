-- rate_limit_windows.window_expires_at: each row's own fixed-window expiry,
-- so the sweep in `vpay_db::rate_limits::COUNT_ATTEMPT` removes only rows whose
-- OWN window has elapsed (stephane-segning P1, PR #396).
--
-- Before this, the sweep judged every row by the *calling* request's window:
-- a 60-second lookup (issue #56) deleted staff sign-in counters (300 seconds)
-- that were still inside their own window, resetting their brute-force budget
-- the moment a merchant performed a lookup. Storing the expiry per row lets the
-- sweep ask "has THIS row's window elapsed" instead of "has a 60-second window
-- elapsed since THIS row started".
--
-- `window_expires_at` is `window_started_at + window`. It is written by
-- `count_attempt`, never by a generated builder, so 0034's rule applies and
-- the temporary DEFAULT is dropped in the very next statement.

ALTER TABLE rate_limit_windows ADD COLUMN window_expires_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- Backfill pre-existing rows (all written under the 300-second staff windows)
-- so the new NOT NULL column has a value on every row before the DEFAULT goes.
UPDATE rate_limit_windows
   SET window_expires_at = window_started_at + interval '5 minutes'
 WHERE window_expires_at IS NULL;

-- No DEFAULT on a column a writer names (0034's rule).
ALTER TABLE rate_limit_windows ALTER COLUMN window_expires_at DROP DEFAULT;

COMMENT ON COLUMN rate_limit_windows.window_expires_at IS
    'window_started_at + window: the instant this row''s own fixed window ends, decided by the writer. The COUNT_ATTEMPT sweep removes only rows whose own window_expires_at has passed, so one scope''s shorter window never resets another scope''s longer one (PR #396, stephane-segning P1).';
