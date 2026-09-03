-- webhook_deliveries: one row per (event, endpoint) pair — the delivery
-- attempt log behind docs/flows/webhooks.md, and the two job kinds that
-- drive it.
--
-- WHY THE `jobs` VOCABULARY IS REOPENED HERE
--
-- 0021 closed `kind_is_known` over four kinds and said so in its own
-- comment: "`deliver_webhook` is deliberately absent — Step 5 adds it in its
-- own migration, so this step cannot enqueue a webhook delivery by accident
-- and then silently never run it." This is that migration, and the same
-- argument is why the constraint is dropped and re-added rather than being
-- written permissively the first time: the database is the thing that
-- refuses a job no handler exists for, so it has to change in lockstep with
-- the handlers.
--
-- `fan_out_events` is the outbox drain (one singleton job, dedupe key
-- `fanout:events`); `deliver_webhook` is one job per delivery row.
ALTER TABLE jobs DROP CONSTRAINT kind_is_known;
ALTER TABLE jobs ADD CONSTRAINT kind_is_known CHECK (kind IN
  ('poll_charge','resubmit_charge','sweep_expired','scan_live_charges','fan_out_events','deliver_webhook'));

-- WHY THERE IS NO `webhook_endpoints` TABLE
--
-- Endpoints are configuration, not merchant-mutable state: they live in
-- `merchant_clients[].webhooks[]` in YAML (ADR-0003, and
-- docs/flows/configuration.md already lists webhook endpoints among the
-- values safe to mutate there), and the dashboard cannot administer them
-- (ADR-0008). So this table references an endpoint by the operator-authored
-- string id from that document and joins to nothing.
CREATE TABLE webhook_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id TEXT NOT NULL REFERENCES events (id),
    -- The operator-authored `merchant_clients[].webhooks[].id` from YAML.
    -- Not a URL hash: an operator correcting a typo'd URL must not orphan
    -- the delivery history, and a hash is unreadable in a runbook.
    endpoint_id TEXT NOT NULL,
    -- Denormalised for forensics; the endpoint may be re-pointed later.
    url TEXT NOT NULL,
    attempt INT NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'pending',
    status_code INT,
    response_excerpt TEXT,
    -- Proof the bytes did not change between attempts: the envelope is
    -- re-rendered per attempt rather than stored, so the first attempt
    -- records this digest and every later one compares against it. A
    -- mismatch means a renderer changed under a live delivery, which a
    -- merchant would see as a bad signature.
    payload_sha256 TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ, responded_at TIMESTAMPTZ,
    next_attempt_at TIMESTAMPTZ,
    CONSTRAINT state_is_known CHECK (state IN ('pending','succeeded','failed','exhausted')),
    CONSTRAINT endpoint_id_length CHECK (char_length(endpoint_id) BETWEEN 1 AND 64),
    CONSTRAINT url_length CHECK (char_length(url) BETWEEN 1 AND 2048),
    CONSTRAINT excerpt_length CHECK (response_excerpt IS NULL OR char_length(response_excerpt) <= 2000),
    CONSTRAINT attempt_is_not_negative CHECK (attempt >= 0)
);

-- What makes the fan-out transaction's `INSERT … ON CONFLICT (event_id,
-- endpoint_id) DO NOTHING` legal, and what makes "one delivery per event per
-- endpoint, forever" a property of the schema rather than of whichever
-- fan-out pass happened to run first. The drain is at-least-once by
-- construction (a crash between the inserts and the `fanout_state` flip
-- re-runs the whole event), so without this index a retried pass would
-- deliver every event twice.
CREATE UNIQUE INDEX webhook_deliveries_event_endpoint ON webhook_deliveries (event_id, endpoint_id);

-- Deliveries that are still owed an attempt, oldest due first. Partial on
-- `state = 'pending'` for the same reason `jobs_claimable_idx` is partial on
-- `locked_at IS NULL` — the index stays the size of the *outstanding* work
-- rather than of the delivery log, which in a healthy system is near-empty
-- however many webhooks have ever been sent.
--
-- Written for a backstop scan that this migration does NOT ship: at 0022 the
-- only caller was an operator running the query by hand, and
-- `vpay_db::webhook_deliveries::pending_due` had no caller at all. Migration
-- 0023 adds the `scan_deliveries` job that reads it. Left as it stands rather
-- than rewritten (a migration is history, docs/adr/0003), with 0023 carrying
-- the corrected COMMENT ON INDEX.
CREATE INDEX webhook_deliveries_live_idx ON webhook_deliveries (next_attempt_at)
    WHERE state = 'pending';

COMMENT ON TABLE webhook_deliveries IS
    'One row per (event, endpoint) pair: the durable record of what vpay owes a merchant and what happened on each attempt. Created by the fan_out_events drain in the same transaction that marks the event fanned out, and updated by deliver_webhook. Endpoints themselves are YAML (ADR-0003), so endpoint_id references no table.';
COMMENT ON COLUMN webhook_deliveries.endpoint_id IS
    'The operator-authored merchant_clients[].webhooks[].id, unique within a merchant and refused as a duplicate at boot. Deliberately not a hash of the URL: a corrected URL must keep its delivery history, and a runbook has to be able to name the endpoint.';
COMMENT ON COLUMN webhook_deliveries.url IS
    'The URL as it was when this delivery was created. Denormalised on purpose — re-pointing an endpoint in YAML must not rewrite the history of where bytes were actually sent.';
COMMENT ON COLUMN webhook_deliveries.attempt IS
    'How many attempts have *failed* so far. Zero for a delivery that has never been tried and for one that succeeded first time; it is the index into the retry ladder (vpay_worker::delivery_delay), and running off the end of that ladder is what state = ''exhausted'' records.';
COMMENT ON COLUMN webhook_deliveries.state IS
    'pending (owed an attempt), succeeded (a 2xx was read), exhausted (the retry ladder ran out). ''failed'' is in the vocabulary but nothing writes it today — a failure that is not yet exhausted stays pending, because that is what says a further attempt is owed.';
COMMENT ON COLUMN webhook_deliveries.payload_sha256 IS
    'SHA-256 of the exact bytes signed and sent on the first attempt. The body itself is not stored (it would duplicate every event once per endpoint); this makes "we sent what we signed" checkable, and a later attempt whose re-rendered body hashes differently is poisoned rather than delivered.';
COMMENT ON COLUMN webhook_deliveries.sent_at IS
    'When the most recent attempt was issued. With status_code and responded_at both NULL this is the transport-failure shape — the request went out and nothing came back — which is why there is deliberately no CHECK pairing these three columns.';
COMMENT ON COLUMN webhook_deliveries.next_attempt_at IS
    'When the next attempt becomes due, from the retry ladder. NULL for a delivery that has never been attempted (its deliver_webhook job was enqueued in the same transaction and already owns it) and for one that is no longer owed an attempt.';
