-- provider_requests: one row per *attempt* to talk to a rail.
--
-- This is the table docs/flows/crash-safety.md's "never let a payer act on
-- a transaction you cannot name" rule needs on the far side of the network
-- call. The write-before-network ordering already gives us a charge row
-- carrying the `provider_reference_id` before anything is submitted; this
-- table records what happened to each individual call made with that
-- reference — including the calls that got no answer at all, which are
-- precisely the ones a recovery sweep has to find.
--
-- NOT unique on charge_id, deliberately: a charge is submitted once but
-- *queried* many times (the poll ladder), and a submit that timed out may
-- legitimately be retried against the same reference. One row per attempt
-- is the whole point; `attempt` numbers them.
CREATE TABLE provider_requests (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    charge_id TEXT NOT NULL REFERENCES charges (id),
    provider_code TEXT NOT NULL REFERENCES providers (code),
    operation TEXT NOT NULL,
    -- The rail-facing idempotency key of the charge this attempt belongs to
    -- (vpay_provider::ChargeRef::reference_id). Denormalised from
    -- charges.provider_reference_id on purpose: an operator reconciling
    -- against a rail's dashboard has only that reference to search by, and
    -- a retry that ever generates a *new* reference must be visible here as
    -- two different values rather than hidden behind a join.
    provider_reference_id UUID NOT NULL,
    attempt INT NOT NULL DEFAULT 1,
    -- The rail's HTTP status, once there is one. NULL means "no answer was
    -- received", which is a materially different thing from "the rail said
    -- no" and is why this column is nullable at all.
    status_code INT,
    -- Set when the attempt failed without an HTTP status: a timeout, a TLS
    -- failure, a `ProviderError::NotImplemented`. Free text on purpose —
    -- this is an operator/debugging field, never a merchant-facing code
    -- (that vocabulary is `failure_code`, and it lives on the charge).
    error_kind TEXT,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    responded_at TIMESTAMPTZ,

    CONSTRAINT operation_is_known CHECK (operation IN ('submit', 'query_status', 'refund')),
    CONSTRAINT attempt_is_positive CHECK (attempt >= 1),
    CONSTRAINT error_kind_length CHECK (error_kind IS NULL OR char_length(error_kind) <= 128),
    -- "Answered" and "has a status" are the same fact, so they may not
    -- disagree. This is what makes `status_code IS NULL` a trustworthy
    -- index predicate for "attempts still outstanding" — a row with a
    -- `responded_at` but no status would be invisible to that sweep while
    -- looking answered to a human reading the table.
    CONSTRAINT response_is_paired CHECK ((status_code IS NULL) = (responded_at IS NULL))
);

CREATE INDEX provider_requests_charge_id_sent_at_idx ON provider_requests (charge_id, sent_at);

COMMENT ON TABLE provider_requests IS
    'One row per attempt to call a rail (never one per charge — the poll ladder queries repeatedly). status_code/responded_at are NULL until an answer arrives, and the response_is_paired CHECK keeps those two facts in step; a NULL-status row is an unanswered attempt, which is exactly what a recovery sweep looks for.';
COMMENT ON COLUMN provider_requests.error_kind IS
    'Operator-facing failure label for an attempt with no HTTP status (timeout, TLS failure, not_implemented). Not the merchant-facing failure vocabulary — that is failure_code on charges (docs/flows/failures.md).';
