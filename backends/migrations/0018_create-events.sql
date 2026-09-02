-- events: the durable log behind `GET /v1/events` and webhook fan-out
-- (docs/flows/webhooks.md).
--
-- STATUS, before anything else: **nothing in this repository writes or
-- reads this table.** No event is emitted anywhere, `/v1/events` is
-- deliberately not routed this step (an honest 404 rather than an empty
-- list), and the worker's fan-out job loop does not exist (docs/status.md).
-- This is the intended shape, landed alongside the payment-intent schema so
-- the emitting code has somewhere to write when it is built — not evidence
-- that events work. Same posture as 0005's ledger tables and 0017's refunds.
CREATE TABLE events (
    -- Caller-supplied `evt_…` id (vpay_core::ids).
    id TEXT PRIMARY KEY,
    -- The fan-out cursor, and the same argument as payment_intents.seq
    -- (0014): `created_at` ties under a burst, and a delivery cursor that
    -- can skip an event is a webhook a merchant never receives.
    seq BIGINT GENERATED ALWAYS AS IDENTITY,
    -- No FK: there is no merchants table (ADR-0003; see 0003's comment).
    merchant_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    type TEXT NOT NULL,
    -- The id of the object this event is about (`pi_…`, `ch_…`, `re_…`).
    -- Untyped and un-foreign-keyed on purpose: it points into three
    -- different tables depending on `type`, and a polymorphic reference
    -- cannot be a foreign key. The alternative — three nullable typed
    -- columns — would let a row name two objects at once.
    object_id TEXT NOT NULL,
    -- The full wire object as it was at emit time. Snapshot, not a join:
    -- a webhook must deliver what was true when the event happened, even
    -- if the object has moved on by the time delivery succeeds.
    data JSONB NOT NULL,
    fanout_state TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),
    CONSTRAINT object_id_length CHECK (char_length(object_id) BETWEEN 1 AND 64),
    CONSTRAINT data_is_object CHECK (jsonb_typeof(data) = 'object'),
    CONSTRAINT fanout_state_is_known CHECK (fanout_state IN ('pending', 'done')),
    -- The seven types docs/flows/webhooks.md fixes, transcribed exactly.
    -- That document's rule is "only real Stripe event types" — an invented
    -- `payment_intent.updated` would be a type no merchant's existing
    -- Stripe-shaped handler knows, so the closure is enforced here rather
    -- than trusted to whoever writes the emitting code later.
    CONSTRAINT type_is_a_documented_event CHECK (type IN (
        'payment_intent.created',
        'payment_intent.processing',
        'payment_intent.succeeded',
        'payment_intent.payment_failed',
        'payment_intent.canceled',
        'charge.refunded',
        'charge.refund.updated'
    ))
);

-- An identity column is not implicitly unique, and every cursor below
-- assumes a total order — same reasoning as payment_intents_seq_key.
CREATE UNIQUE INDEX events_seq_key ON events (seq);

-- The fan-out worker's whole query: the oldest undelivered events, in
-- order. Partial on `fanout_state = 'pending'` so the index stays the size
-- of the *backlog* rather than of the log — in a healthy system that is
-- near-empty, and a scan of it costs nothing regardless of how many events
-- have ever been emitted.
CREATE INDEX events_pending_idx ON events (seq) WHERE fanout_state = 'pending';

-- `GET /v1/events`: merchant-scoped, newest first, same shape as
-- payment_intents_merchant_seq_idx.
CREATE INDEX events_merchant_seq_idx ON events (merchant_id, seq DESC);

COMMENT ON TABLE events IS
    'Intended persistence shape for the event log and webhook fan-out. NOT WRITTEN OR READ BY ANY CODE IN THIS REPOSITORY — nothing emits events, /v1/events is not routed, and the fan-out job loop does not exist (docs/status.md). Schema only.';
COMMENT ON COLUMN events.type IS
    'Constrained to the seven event types in docs/flows/webhooks.md. Only real Stripe event types, so a merchant''s existing Stripe-shaped handler recognises every one of them.';
