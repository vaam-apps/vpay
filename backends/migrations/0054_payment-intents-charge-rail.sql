-- The rail and the payment-method type of an intent's one charge, copied
-- onto the intent so that /v1 can show them (the maintainer's "Both" on
-- 2026-10-24, shape A of docs/plans/2026-10-24-rail-on-v1.md, chosen as
-- "Go with A" the same day; ADR-0029 D10 as amended that day).
--
-- WHY ON THE INTENT AND NOT READ FROM THE CHARGE. Every surface that renders
-- an intent -- the /v1 routes, the payer's browser routes and the worker's
-- event snapshot -- builds it from a payment_intents row, through one
-- column list (vpay_db::payment_intents::COLUMNS). Two columns written once
-- reach every one of them through that row; a read of the charge would have
-- to be added beside every one. And an event snapshot taken from the row is
-- true at the moment it was taken.
--
-- THE INVARIANT: charge_rail and charge_method_type are the
-- intent's one charge's provider_code and method_type, exactly. They are
-- written by vpay_db::charges::insert_for_intent, in the transaction that
-- inserts the charge, and never again: one charge per intent, forever
-- (one_charge_per_intent, ADR-0029 D11), and nothing updates a charge's
-- provider_code or method_type.
--
-- THE BACKFILL COPIES, IT DOES NOT INFER. A charge written before 0053 has a
-- NULL method_type and its intent gets a NULL charge_method_type, even on a
-- rail that serves one type. Typing it by rule (mtn_momo -> mtn_wallet)
-- would make the intent say something its charge does not, and 0053 chose
-- NULL as "the truth about a charge whose network vpay never recorded".
-- updated_at is not touched: the backfill changes no fact about the intent,
-- it copies one that was already true onto it.
--
-- THE SPELLINGS move in lockstep with vpay_core::routing::MethodType and
-- with 0053's method_type_is_known; a type added there is added here too.
ALTER TABLE payment_intents
    ADD COLUMN charge_rail TEXT REFERENCES providers (code),
    ADD COLUMN charge_method_type TEXT;

ALTER TABLE payment_intents ADD CONSTRAINT charge_method_type_is_known
    CHECK (charge_method_type IS NULL OR charge_method_type IN ('mtn_wallet', 'orange_wallet'));

ALTER TABLE payment_intents ADD CONSTRAINT charge_method_type_needs_a_rail
    CHECK (charge_method_type IS NULL OR charge_rail IS NOT NULL);

UPDATE payment_intents AS p
   SET charge_rail = c.provider_code,
       charge_method_type = c.method_type
  FROM charges AS c
 WHERE c.payment_intent_id = p.id;

COMMENT ON COLUMN payment_intents.charge_rail IS
    'The rail (providers.code) of this intent''s one charge, or NULL while it has none. Copied from charges.provider_code by vpay_db::charges::insert_for_intent in the transaction that opens the charge, and never changed. Rendered on /v1 as payment_method_details.rail (migration 0054).';

COMMENT ON COLUMN payment_intents.charge_method_type IS
    'The payment-method type of this intent''s one charge (mtn_wallet, orange_wallet), copied from charges.method_type with charge_rail; NULL where the charge records none. Rendered on /v1 as payment_method_details.type (migration 0054).';

-- 0053 commented routing_decisions "Never on the /v1 object", and that stays
-- true of the decision record. What changed on 2026-10-24 is that the
-- outcome is on /v1, from the two columns above; this says so where an
-- operator reading \d+ routing_decisions will look.
COMMENT ON TABLE routing_decisions IS
    'ADR-0029 D10: the router''s decision for one routed charge, written in the transaction that opens it. The method type, the chosen and pinned rails, every rail considered with its score breakdown or its exclusion reason, the score formula (provisional until RFC-0008''s 2026-10-23 revision is decided) and the configuration version. The decision record is never on the /v1 object; since migration 0054 its outcome (the type and the rail) is, as payment_intents.charge_method_type and charge_rail.';
