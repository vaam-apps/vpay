-- idempotency_keys.response_retry: the third thing a replay has to re-emit.
--
-- `vpay_api::error`'s `ApiError::into_response` puts `stripe-should-retry` on
-- every error it renders, derived from `Classify::retry` (ADR-0011: a boundary
-- renders a classification, it never makes one). A *replay* did not go through
-- that renderer — `v1::payment_intents::replay` rebuilds the response from
-- `(response_status, response_body)` — so until this migration a merchant who
-- retried under a key whose stored answer was a `409` got that `409` with no
-- advisory, and stripe-node then applied its own "retry every 409 twice" rule
-- to a refusal that waiting cannot fix. That was written down as a known gap
-- in three documents and pinned by two tests; this column closes it.
--
-- # Why the header's own text, and not a BOOLEAN
--
-- Because storing the text is the option that cannot re-derive anything.
-- `store` is handed the value the fresh response *actually carried*, read
-- straight off its `HeaderMap`, and `replay` writes those same bytes back —
-- the same relationship `response_status` and `response_body` already have
-- with the response they replay. A BOOLEAN would be smaller and would make
-- the two-valued domain a property of the type rather than of a CHECK, but it
-- would put a second `bool -> "true"/"false"` rendering in the replay path,
-- and "the advisory is rendered in exactly one place" is the invariant
-- ADR-0011 is protecting here. The CHECK below buys the domain back.
--
-- # Why NULL is a value and not a default
--
-- NULL means "the response this row stores carried no advisory", which is a
-- real state and not the same as `'false'`: a `200` from a successful create
-- never passes through the error renderer at all. A replay of one must emit
-- no header, exactly as the fresh response did — `NOT NULL DEFAULT 'false'`
-- would invent an advisory for every stored success.
--
-- Existing rows therefore get NULL, which is correct rather than merely
-- convenient: they were stored before anything recorded the advisory, so
-- "unknown" is the truth about them, and their replays behave exactly as they
-- did before this migration.
ALTER TABLE idempotency_keys
    ADD COLUMN response_retry TEXT
        CONSTRAINT response_retry_is_an_advisory CHECK (response_retry IN ('true', 'false'));

COMMENT ON COLUMN idempotency_keys.response_retry IS
    'The verbatim stripe-should-retry header value the stored response carried, or NULL if it carried none (a 2xx never passes through the error renderer). Written by vpay_db::idempotency::store from the rendered response''s own HeaderMap and re-emitted unchanged by vpay_api::v1::payment_intents::replay, so the advisory is derived from Classify::retry in exactly one place (ADR-0011) and a replay can never disagree with the response it replays.';
