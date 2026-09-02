-- Refund: the persistence shape for `POST /v1/refunds`, mirroring
-- schemas/vpay.cstack's `Refund` sketch and the amounts already tracked on
-- `payment_intents` (`amount_refunded`, `amount_refund_pending`).
--
-- STATUS, stated before anything else: **nothing in this repository writes
-- or reads this table.** There is no refunds repository in `vpay-db`, no
-- `/v1/refunds` route, and no adapter `refund` implementation — the port's
-- refund path is still `ProviderError::NotImplemented` (docs/status.md).
-- This migration exists so the schema lands with the rest of the
-- payment-intent shape rather than in a later, riskier ALTER-heavy step; it
-- is the intended shape, not evidence that refunds work. Same posture, and
-- the same wording, as 0005's ledger tables.
CREATE TYPE refund_status AS ENUM ('pending', 'succeeded', 'failed', 'canceled');

CREATE TABLE refunds (
    -- Caller-supplied `re_…` id (vpay_core::ids), like every other public
    -- object id in this schema. Postgres generates none of them: the id has
    -- to exist in Rust before the row is written so a crash between the two
    -- still leaves something to reconcile by.
    id TEXT PRIMARY KEY,
    payment_intent_id TEXT NOT NULL REFERENCES payment_intents (id),
    -- Nullable: a refund is requested against an *intent*, and the intent
    -- may not have a charge row to attribute it to at the moment the
    -- request arrives. It is filled in once the charge is known.
    charge_id TEXT REFERENCES charges (id),
    amount BIGINT NOT NULL,
    -- Carried verbatim from the intent, never converted (docs/flows/money.md:
    -- one currency per object, no conversion anywhere in vpay).
    currency_code TEXT NOT NULL REFERENCES currencies (code),
    status refund_status NOT NULL,
    -- Merchant-supplied free text ("duplicate", "requested_by_customer").
    -- Deliberately NOT an enum: unlike `failure_code` this vocabulary is
    -- the merchant's, not vpay's, so closing it here would reject reasons
    -- vpay has no business having an opinion about.
    reason TEXT,
    -- Same pair, and the same reasoning, as `charges.failure_code` /
    -- `charges.failure_raw`: the closed code a merchant branches on, and
    -- the rail's own text kept so nothing is silently dropped.
    failure_code failure_code,
    failure_raw TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- The rail-facing reference for this refund, generated before the call
    -- (docs/flows/crash-safety.md). NULL until a rail call is attempted.
    provider_reference_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    -- Strictly positive, unlike `charges.amount`/`payment_intents.amount`
    -- (which allow 0): a zero-amount refund moves no money and is a
    -- caller mistake, not a legitimate no-op to store.
    CONSTRAINT amount_positive CHECK (amount > 0),
    CONSTRAINT reason_length CHECK (reason IS NULL OR char_length(reason) <= 512),
    CONSTRAINT failure_raw_length CHECK (failure_raw IS NULL OR char_length(failure_raw) <= 2000),
    CONSTRAINT metadata_is_object CHECK (jsonb_typeof(metadata) = 'object'),
    -- Same pairing rule the charge's failure columns imply and 0014 made
    -- explicit for the intent: a code with no raw text, or raw text with no
    -- code, is a half-written failure.
    CONSTRAINT failure_paired CHECK ((failure_code IS NULL) = (failure_raw IS NULL))
);

CREATE INDEX refunds_payment_intent_id_idx ON refunds (payment_intent_id);
CREATE INDEX refunds_charge_id_idx ON refunds (charge_id);

-- GAP, not handled here: docs/flows/ledger.md's over-refund invariant is
-- enforced on `payment_intents` (the `no_over_refund` CHECK added by 0003),
-- which is where the running totals live. This table has no constraint
-- tying SUM(refunds.amount) to those totals — a row-level CHECK cannot
-- express an aggregate, exactly as 0005's ledger comment explains. Whoever
-- implements the refunds repository must update `amount_refund_pending` in
-- the same transaction that inserts the row, and it is that UPDATE, not
-- this table, that the over-refund CHECK stops.
COMMENT ON TABLE refunds IS
    'Intended persistence shape for refunds. NOT WRITTEN OR READ BY ANY CODE IN THIS REPOSITORY — no refunds repository, no /v1/refunds route, and the adapter refund path is still NotImplemented (docs/status.md). Schema only, like the ledger tables in 0005.';
