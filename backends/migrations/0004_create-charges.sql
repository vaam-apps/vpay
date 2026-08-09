-- Charge: mirrors backends/crates/vpay-core/src/state.rs (ChargeState),
-- backends/crates/vpay-core/src/failure.rs (FailureCode), and
-- backends/crates/vpay-provider/src/lib.rs (ChargeRef.reference_id: Uuid),
-- plus schemas/vpay.cstack's `Charge` model.

-- backends/crates/vpay-core/src/state.rs — ChargeState. Unlike IntentStatus,
-- Charge does have a real terminal `failed`.
CREATE TYPE charge_state AS ENUM (
    'submitting',
    'submitted',
    'pending',
    'unresolved',
    'succeeded',
    'failed'
);

-- backends/crates/vpay-core/src/failure.rs — FailureCode. "This vocabulary is
-- closed and owned by the core" (the crate's own doc comment) — a native
-- Postgres enum enforces that closure at the database, not just in Rust.
CREATE TYPE failure_code AS ENUM (
    'insufficient_funds',
    'payer_timeout',
    'payer_declined',
    'invalid_payer',
    'payer_limit_reached',
    'payer_account_blocked',
    'invalid_payee',
    'payee_account_blocked',
    'provider_account_blocked',
    'provider_unavailable',
    'provider_error'
);

CREATE TABLE charges (
    id TEXT PRIMARY KEY,
    payment_intent_id TEXT NOT NULL REFERENCES payment_intents (id),
    provider_code TEXT NOT NULL REFERENCES providers (code),
    -- Matches vpay_provider::ChargeRef::reference_id: Uuid exactly. This is
    -- the reference generated *before* any network call (docs/flows/crash-
    -- safety.md) — on a push rail it is the X-Reference-Id sent to the rail.
    provider_reference_id UUID NOT NULL,
    -- vpay_provider::RefExtra = BTreeMap<String, String>; rail key material
    -- (e.g. Orange's pay_token) captured from a previous `submit`.
    provider_ref_extra JSONB,
    -- Present iff the rail's flow is ProviderFlow::Redirect.
    redirect_url TEXT,
    state charge_state NOT NULL,
    amount BIGINT NOT NULL,
    currency_code TEXT NOT NULL REFERENCES currencies (code),
    -- Payer instrument. NULL on redirect rails, where the payer authenticates
    -- with the rail and we may never learn who they are.
    payer_ref TEXT,
    payer_ref_masked TEXT,
    -- vpay_provider::ChargeStatus::Failed carries both `code` AND `raw` — both
    -- columns exist so neither is silently dropped.
    failure_code failure_code,
    failure_raw TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT payment_intent_id_length CHECK (char_length(payment_intent_id) BETWEEN 1 AND 64),
    CONSTRAINT failure_raw_length CHECK (failure_raw IS NULL OR char_length(failure_raw) <= 2000),
    CONSTRAINT amount_non_negative CHECK (amount >= 0)
);

-- "One charge per intent, forever" (AGENTS.md, docs/flows/payment-lifecycle.md).
-- A plain unique index, deliberately NOT partial/scoped to "live" states — the
-- flow doc explains why: the moment a charge moves to `failed`, a predicate
-- scoped to live states would stop covering it and a second charge would
-- become insertable, and "failed" can mean a state reached *before* the
-- rail's answer was final. Retry means a new PaymentIntent, not a second
-- Charge row.
CREATE UNIQUE INDEX one_charge_per_intent ON charges (payment_intent_id);

CREATE INDEX charges_state_idx ON charges (state);

COMMENT ON TABLE charges IS
    'Mirrors vpay_core::state::ChargeState and vpay_provider::ChargeRef. See docs/flows/crash-safety.md for the write-before-network ordering this table exists to support.';
