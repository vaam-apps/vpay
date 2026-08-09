-- PaymentIntent: mirrors backends/crates/vpay-core/src/state.rs (IntentStatus)
-- and schemas/vpay.cstack's `PaymentIntent` model.

-- backends/crates/vpay-core/src/state.rs — IntentStatus. Note there is no
-- `failed` variant: docs/flows/payment-lifecycle.md's state diagram labels
-- its "failed" box as an alias for `requires_payment_method` with
-- `last_payment_error` set — a rail-reported failure returns the intent to
-- requires_payment_method, it does not add a state.
CREATE TYPE intent_status AS ENUM (
    'requires_payment_method',
    'requires_action',
    'processing',
    'succeeded',
    'canceled'
);

CREATE TABLE payment_intents (
    id TEXT PRIMARY KEY,
    -- No FK: there is no `Merchant` model/table. Nothing in
    -- backends/crates/*/src declares a `struct Merchant` today — see
    -- schemas/vpay.cstack's GAP comment on this same field. Inventing a
    -- `merchants` table to hang a foreign key off would be exactly the
    -- plausible-but-fabricated failure mode this schema avoids elsewhere.
    merchant_id TEXT NOT NULL,
    livemode BOOLEAN NOT NULL,
    -- All four amount columns are integer minor units (docs/flows/money.md) —
    -- BIGINT to match Money's i64 representation exactly.
    amount BIGINT NOT NULL,
    amount_received BIGINT NOT NULL DEFAULT 0,
    amount_refunded BIGINT NOT NULL DEFAULT 0,
    amount_refund_pending BIGINT NOT NULL DEFAULT 0,
    currency_code TEXT NOT NULL REFERENCES currencies (code),
    status intent_status NOT NULL,
    -- Populated when a rail-reported failure returns the intent to
    -- requires_payment_method (docs/flows/payment-lifecycle.md).
    last_payment_error TEXT,
    -- vpay_provider::ChargeRef and friends don't fix a concrete list shape
    -- for this yet, and cratestack-parser rejects list-arity scalars
    -- (`String[]`) on a database-backed model outright — see the note in
    -- schemas/vpay.cstack. JSONB is the flexible, queryable equivalent.
    payment_method_types JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),
    CONSTRAINT last_payment_error_length CHECK (last_payment_error IS NULL OR char_length(last_payment_error) <= 512),
    CONSTRAINT amount_non_negative CHECK (amount >= 0),
    CONSTRAINT amount_received_non_negative CHECK (amount_received >= 0),
    CONSTRAINT amount_refunded_non_negative CHECK (amount_refunded >= 0),
    CONSTRAINT amount_refund_pending_non_negative CHECK (amount_refund_pending >= 0),

    -- Over-refund guard: `amount_refunded + amount_refund_pending <= amount`.
    --
    -- docs/flows/ledger.md previously claimed a database CHECK rejects "the
    -- second of two concurrent over-refunds at the database level" — that
    -- claim was corrected to say no such constraint exists, because
    -- schemas/vpay.cstack's grammar cannot express a cross-column CHECK.
    -- Raw SQL can, and this is a genuine improvement over the design sketch.
    --
    -- One honest limit even with this CHECK in place: it rejects any single
    -- statement that would leave the row over-refunded, but it does not by
    -- itself provide the "second of two *concurrent* writers loses" row-lock
    -- semantics docs/flows/ledger.md originally (incorrectly) described —
    -- that still needs `SELECT ... FOR UPDATE` (or an equivalent) around the
    -- read-modify-write in application code, which is not implemented here.
    -- What this CHECK does guarantee unconditionally, concurrency included,
    -- is that no committed row can ever end up over-refunded: two concurrent
    -- `UPDATE`s racing to increment `amount_refund_pending` still serialize
    -- at the database (the second writer blocks on the row lock MVCC already
    -- takes for any UPDATE, then re-evaluates the CHECK against the first
    -- writer's committed value), so the second one fails the CHECK instead
    -- of silently over-committing.
    CONSTRAINT no_over_refund CHECK (amount_refunded + amount_refund_pending <= amount)
);

CREATE INDEX payment_intents_merchant_id_idx ON payment_intents (merchant_id);

COMMENT ON TABLE payment_intents IS
    'Mirrors vpay_core::state::IntentStatus and PaymentIntent design sketch in schemas/vpay.cstack.';
