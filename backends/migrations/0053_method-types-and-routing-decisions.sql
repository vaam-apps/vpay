-- Payment-method types on charges, and the router's decision for each routed
-- charge (ADR-0029 D6, D10; RFC-0008 as revised on 2026-10-23).
--
-- WHY. The maintainer, 2026-10-23: "we need a way to differentiate mtn x
-- notchpay and mtn x direct; same with orange and the future integrations".
-- Until this migration a charge named its rail (provider_code) and nothing
-- else, so an MTN payment through Notch Pay and one through MTN's own API
-- were told apart only by knowing which rails serve which network.
--
-- charges.method_type is what the payer chose: one wallet network
-- (ADR-0029 D12, spellings confirmed by the maintainer the same day). It is
-- set on every charge the router chose a rail for, and on a charge confirmed
-- by a rail's own code when that rail serves exactly one type. It stays NULL
-- on a charge confirmed by the code of a rail serving several (notchpay),
-- where the rail code says nothing about the network, and on every charge
-- written before this migration. No backfill: a NULL is the truth about a
-- charge whose network vpay never recorded.
--
-- routing_decisions is D10's row: written in the transaction that opens the
-- charge, never on the /v1 object, so that "why did this payment go through
-- that rail" is answered from a row rather than a log. One row per routed
-- charge, and none for a charge confirmed by rail code, which the router
-- never saw.
--
-- THE SPELLINGS move in lockstep with vpay_core::routing::MethodType, as
-- jobs.kind_is_known moves with JobKind: the database refuses a type no code
-- writes.
ALTER TABLE charges ADD COLUMN method_type TEXT;
ALTER TABLE charges ADD CONSTRAINT method_type_is_known
    CHECK (method_type IS NULL OR method_type IN ('mtn_wallet', 'orange_wallet'));

COMMENT ON COLUMN charges.method_type IS
    'The payment-method type the payer chose (vpay_core::routing::MethodType: mtn_wallet, orange_wallet). Set on a routed charge and on a rail-code charge whose rail serves one type; NULL on a rail-code charge on a rail serving several, and on charges before migration 0053. Carried to the adapter as ChargeRef::method_type (notchpay sends cm.mtn or cm.orange from it).';

CREATE TABLE routing_decisions (
    -- One decision per charge. Cascades with its charge: the row explains
    -- the charge and means nothing without it.
    charge_id TEXT PRIMARY KEY REFERENCES charges (id) ON DELETE CASCADE,
    method_type TEXT NOT NULL,
    chosen_rail TEXT NOT NULL REFERENCES providers (code),
    -- payment_method_options[<type>][rail] on the confirm, when one was sent.
    pinned_rail TEXT,
    -- Every rail considered, in the router's order: a candidate as
    -- {"rail", "score": {"total", "parts": [{"component", "points"}]}}, an
    -- excluded one as {"rail", "excluded": "<reason>"}.
    considered JSONB NOT NULL,
    -- vpay_core::routing::SCORE_FORMULA when the row was written.
    formula TEXT NOT NULL,
    -- 16 hex digits of the configuration the decision was made under.
    config_version TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT routing_method_type_is_known
        CHECK (method_type IN ('mtn_wallet', 'orange_wallet')),
    CONSTRAINT pinned_rail_length
        CHECK (pinned_rail IS NULL OR char_length(pinned_rail) BETWEEN 1 AND 64),
    CONSTRAINT considered_is_an_array CHECK (jsonb_typeof(considered) = 'array'),
    CONSTRAINT formula_length CHECK (char_length(formula) BETWEEN 1 AND 64),
    CONSTRAINT config_version_shape CHECK (config_version ~ '^[0-9a-f]{16}$')
);

COMMENT ON TABLE routing_decisions IS
    'ADR-0029 D10: the router''s decision for one routed charge, written in the transaction that opens it. The method type, the chosen and pinned rails, every rail considered with its score breakdown or its exclusion reason, the score formula (provisional until RFC-0008''s 2026-10-23 revision is decided) and the configuration version. Never on the /v1 object.';
