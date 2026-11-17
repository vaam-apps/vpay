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
-- `count_attempt`, never by a generated builder, so 0034's rule applies: the
-- column carries no DEFAULT at all.

-- Added NULLABLE, backfilled, then made NOT NULL — in that order, on purpose.
--
-- The obvious `ADD COLUMN ... NOT NULL DEFAULT now()` followed by
-- `UPDATE ... WHERE window_expires_at IS NULL` is a no-op backfill: the
-- DEFAULT has already filled every existing row with the migration's own
-- instant, so the WHERE matches nothing and every in-flight counter would
-- "expire" the moment the migration ran — resetting each staff sign-in and
-- change-password budget an attacker was part-way through. Proven against a
-- real Postgres by `vpay-db`'s
-- `migration_0061_backfills_each_in_flight_window_from_its_own_start`.
ALTER TABLE rate_limit_windows ADD COLUMN window_expires_at TIMESTAMPTZ;

-- Every row that can exist before this migration was written under a staff
-- budget (`sign_in:email`, `sign_in:address`, `change_password:session`);
-- `lookup:merchant_key` arrives in 0060, one migration earlier, and no server
-- can have counted under it in between. Those budgets' windows are
-- configuration (`vpay_config::RateLimitPolicy`), which a migration cannot
-- read; 300 seconds is the shipped default for all three, so that is the
-- window each in-flight row is given. A deployment that configured a longer
-- window has its in-flight counters shortened to 300 seconds once, at
-- migration — never reset to zero.
UPDATE rate_limit_windows
   SET window_expires_at = window_started_at + interval '300 seconds';

-- No DEFAULT on a column a writer names (0034's rule): `count_attempt` always
-- writes it.
ALTER TABLE rate_limit_windows ALTER COLUMN window_expires_at SET NOT NULL;

COMMENT ON COLUMN rate_limit_windows.window_expires_at IS
    'window_started_at + window: the instant this row''s own fixed window ends, decided by the writer. The COUNT_ATTEMPT sweep removes only rows whose own window_expires_at has passed, so one scope''s shorter window never resets another scope''s longer one (PR #396, stephane-segning P1).';
