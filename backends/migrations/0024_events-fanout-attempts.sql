-- events.fanout_attempts, and a third `fanout_state`: the bound on an event
-- the drain can never fan out.
--
-- WHAT WAS WRONG
--
-- `vpay_worker::webhooks::handle_fan_out` isolates a per-event failure: the
-- event's transaction rolls back, the pass logs `alert = true` and moves on,
-- and the event stays `pending`. That is right for a transient failure and
-- wrong for a permanent one, because `events_pending_idx` is ordered by `seq`
-- and a `pending` event is at the head of *every* subsequent page:
--
--   * the alert fires again on every pass — every 5 seconds when the backlog
--     is draining — so one poisoned event is an unbounded alert storm, and an
--     alert that fires forever is one an operator mutes;
--   * the row occupies one of `FAN_OUT_PAGE`'s 100 slots permanently, so 100
--     of them stop the drain entirely for every merchant behind them.
--
-- WHAT THIS ADDS
--
-- A counter and a terminal state. The drain increments `fanout_attempts` on
-- each failure and, on the fifth (`vpay_worker::webhooks::FANOUT_MAX_ATTEMPTS`),
-- sets `fanout_state = 'failed'`. A `failed` event is not `pending`, so it
-- leaves `events_pending_idx`, leaves `events::pending_page`, and stops being
-- retried and stops alerting. Exactly one `error!(alert = true, …)` is emitted
-- for the transition — so a page of 99 poisoned events costs 99 alerts in
-- total rather than 99 per pass.
--
-- The cost, stated plainly, because it is the same cost `jobs.run_at =
-- 'infinity'` has (0021's `dead_letter`): a `failed` event is a webhook the
-- merchant will never receive and nothing will ever retry by itself. Nothing
-- resurrects one automatically — auto-resurrecting a poisoned row is the hot
-- loop this column exists to stop — so re-arming it is a deliberate `UPDATE`
-- after the cause is fixed, and
-- docs/runbooks/webhook-delivery-failures.md carries the statement.
ALTER TABLE events
    ADD COLUMN fanout_attempts INT NOT NULL DEFAULT 0
        CONSTRAINT fanout_attempts_is_not_negative CHECK (fanout_attempts >= 0);

-- 0018 closed this over two values. Dropped by name and re-added rather than
-- written permissively there, for the reason `jobs.kind_is_known` is reopened
-- in 0022 and 0023: the database is what refuses a state no code writes, so it
-- changes in lockstep with the code that writes it.
ALTER TABLE events DROP CONSTRAINT fanout_state_is_known;
ALTER TABLE events ADD CONSTRAINT fanout_state_is_known
    CHECK (fanout_state IN ('pending', 'done', 'failed'));

COMMENT ON COLUMN events.fanout_attempts IS
    'How many fan-out passes have FAILED on this event. Zero for an event that has never been attempted and for one fanned out first time. Incremented in its own short statement, because the failure it counts is one whose own transaction rolled back — counting inside that transaction would roll the count back with it, and the event would be retried forever at zero.';
COMMENT ON COLUMN events.fanout_state IS
    'pending (owed a fan-out), done (delivery rows created), failed (vpay_worker::webhooks::FANOUT_MAX_ATTEMPTS passes failed on it; abandoned, alerted once, and excluded from events::pending_page so it stops heading every page). Re-arming a failed event is a deliberate UPDATE after the cause is fixed — see docs/runbooks/webhook-delivery-failures.md; nothing resurrects one automatically, because a poisoned event that resurrects itself is a hot loop.';

-- Corrects 0022's comment, which said "the first attempt". It is not: the
-- unconfigured-endpoint branch of `vpay_worker::webhooks::handle_deliver`
-- records a failed attempt having rendered nothing and sent nothing, and
-- passes `None` for this column precisely so it stays NULL. So a delivery can
-- reach `attempt = 3` with a NULL digest, and the digest that is eventually
-- stored belongs to the first attempt that got as far as rendering and
-- signing a body — including one whose socket never opened, because those
-- bytes were signed either way. Re-issued here rather than edited in place
-- for the reason 0023 re-issued the index comment: a migration is history
-- (docs/adr/0003).
COMMENT ON COLUMN webhook_deliveries.payload_sha256 IS
    'SHA-256 of the exact bytes signed on the first attempt that rendered and signed a body — not necessarily attempt 1, since an attempt abandoned before rendering (no endpoint in configuration, no signing secret) stores nothing and leaves this NULL. Written once and COALESCEd thereafter. The body itself is not stored (it would duplicate every event once per endpoint); this makes "we sent what we signed" checkable, and a later attempt whose re-rendered body hashes differently is poisoned rather than delivered.';
