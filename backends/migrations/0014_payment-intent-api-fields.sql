-- The columns `/v1/payment_intents` needs that 0003 did not have: a stable
-- pagination key, merchant-supplied `metadata`/`description`, an
-- `updated_at`, and a *structured* last-payment error.
--
-- HARD CUTOVER, stated plainly: `last_payment_error` (a free TEXT column
-- added by 0003) is DROPPED, not backfilled, and replaced by the pair
-- `last_payment_error_code failure_code` + `last_payment_error_message TEXT`.
-- Nothing in this repository has ever written that column — no SQLx query
-- referenced `payment_intents` at all before this step (docs/status.md,
-- "Database schema") — so there is no data to migrate and a backfill would
-- be inventing values. If a deployment somewhere *did* hold rows with a
-- non-NULL `last_payment_error`, this migration discards them; that is the
-- deliberate trade, and it is only defensible because the column was
-- write-dead.
--
-- Why a pair rather than one string: docs/api/README.md's wire object nests
-- `last_payment_error` as an object with a `code` a merchant may branch on,
-- and docs/flows/failures.md fixes that vocabulary as closed
-- (`vpay_core::FailureCode`, already a native Postgres enum since 0004).
-- Keeping the code in a TEXT blob would have put a closed vocabulary back
-- into free text at exactly the layer that is supposed to enforce it, and a
-- merchant branching on a substring is the failure mode that taxonomy
-- exists to prevent. `lpe_paired` then makes the two columns move together:
-- a code with no message (or a message with no code) is a half-written
-- failure and the database refuses it.
ALTER TABLE payment_intents
    -- The pagination order for `GET /v1/payment_intents`, and the only one.
    -- `created_at` cannot be it: two intents created inside the same
    -- microsecond (a burst from one merchant) would tie, and a cursor over
    -- a non-unique ordering either skips or repeats rows at the page
    -- boundary. A monotone identity column is total and gap-tolerant —
    -- gaps from rolled-back inserts are harmless to a `seq < cursor` scan.
    -- GENERATED ALWAYS (not BY DEFAULT): nothing may supply its own `seq`,
    -- because a hand-picked value could land *below* an already-served
    -- cursor and become invisible to every client that has paged past it.
    ADD COLUMN seq BIGINT GENERATED ALWAYS AS IDENTITY,
    -- Stripe-shaped merchant metadata. The per-key/per-value limits
    -- (50 keys, 40-char keys, 500-char values) are enforced at the API
    -- boundary, not here: they are a *product* limit that a merchant must
    -- get a 400 with a `param` for, and a CHECK violation cannot carry
    -- which key was too long.
    ADD COLUMN metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN description TEXT,
    -- Written explicitly by every state transition in
    -- `vpay_db::payment_intents` (there is deliberately no trigger — see
    -- the note at the foot of this file).
    ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN last_payment_error_code failure_code,
    ADD COLUMN last_payment_error_message TEXT,
    DROP COLUMN last_payment_error;

ALTER TABLE payment_intents
    -- Both halves of the structured error, or neither.
    ADD CONSTRAINT lpe_paired
        CHECK ((last_payment_error_code IS NULL) = (last_payment_error_message IS NULL)),
    -- Same 512-character ceiling the dropped `last_payment_error_length`
    -- put on the column this replaces: a rail's raw text is captured for
    -- operators, not stored in full.
    ADD CONSTRAINT lpe_message_length
        CHECK (last_payment_error_message IS NULL OR char_length(last_payment_error_message) <= 512),
    ADD CONSTRAINT description_length
        CHECK (description IS NULL OR char_length(description) <= 1000),
    -- `metadata` is a JSON *object* on the wire. Without this, `metadata`
    -- could hold `[1,2]` or `"x"` and the API would serialise something no
    -- merchant SDK can deserialise into a map.
    ADD CONSTRAINT metadata_is_object CHECK (jsonb_typeof(metadata) = 'object'),
    -- Same argument for `payment_method_types`, which 0003 typed as bare
    -- JSONB with no shape at all (see its own comment there for why it is
    -- not a `TEXT[]`).
    ADD CONSTRAINT pmt_is_array CHECK (jsonb_typeof(payment_method_types) = 'array');

-- An identity column is not implicitly unique. The cursor scan
-- (`seq < $cursor ORDER BY seq DESC`) is only a total order if `seq` is,
-- so this is enforced rather than assumed.
CREATE UNIQUE INDEX payment_intents_seq_key ON payment_intents (seq);

-- The one index `GET /v1/payment_intents` reads: every list query is
-- merchant-scoped and ordered newest-first, so the leading `merchant_id`
-- equality plus `seq DESC` serves both the default page and every
-- `starting_after` cursor without a sort.
CREATE INDEX payment_intents_merchant_seq_idx ON payment_intents (merchant_id, seq DESC);

COMMENT ON COLUMN payment_intents.seq IS
    'Pagination order for GET /v1/payment_intents (D8: newest first; ending_before scans ASC and reverses in Rust). Never exposed on the wire — the cursor a merchant sends is an id, resolved to a seq by a merchant-scoped subquery.';
COMMENT ON COLUMN payment_intents.last_payment_error_code IS
    'Closed vocabulary (vpay_core::FailureCode). Paired with last_payment_error_message by the lpe_paired CHECK; replaced the free-text last_payment_error column dropped by this migration.';

-- Charges get the same `updated_at` for the same reason: the poll ladder
-- (docs/flows/payment-lifecycle.md) needs to know when a charge row last
-- moved, and `created_at` stops being that answer the moment a state
-- transition happens.
ALTER TABLE charges ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- The worker's future sweep of charges that are still in flight
-- ("live" = every state that is not terminal). NOT unique, and deliberately
-- unrelated to `one_charge_per_intent`, which 0004's own comment explains
-- must stay unscoped: a partial *unique* index over live states would stop
-- covering a charge the moment it failed and let a second charge row in.
-- This one is a plain lookup index over the same predicate — narrowing the
-- scan, never the constraint. The four labels are the non-terminal members
-- of the `charge_state` enum created in 0004 (`succeeded` and `failed` are
-- the terminal two).
CREATE INDEX charges_live_idx ON charges (state)
    WHERE state IN ('submitting', 'submitted', 'pending', 'unresolved');

-- KNOWN, NOT HANDLED: `updated_at` on both tables is maintained by the
-- application (`vpay_db::payment_intents::transition` sets it in the same
-- statement as the status change), not by a trigger. A trigger was
-- considered and rejected for this step: it would be new, unexercised SQL
-- duplicating a write the repository layer already makes and tests, and a
-- writer that forgets `updated_at` is a bug a trigger would *hide* rather
-- than surface. Any future writer that bypasses the repository must set it
-- itself.
