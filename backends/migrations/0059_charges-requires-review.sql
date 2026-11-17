-- charges.state gains `requires_review` (issue #96, item 1).
--
-- A cancel and a confirm can race so that a charge commits onto an intent the
-- cancel has already moved to `canceled` (the confirm's charge insert waits on
-- the cancel's row lock since 0054, then commits). The rail is called, the
-- payer pays, and the settlement meets the `canceled` intent. Until this
-- migration the settlement then failed `WriteMatchedNoRow` and rolled back
-- whole: the charge stayed live, the rail's transaction id was discarded, and
-- the poll job retried the same refusal until it was dead-lettered.
--
-- The decision on #96 is that vpay does not refund on its own. The settlement
-- parks the charge in `requires_review` instead, in its own committed
-- transaction, keeping the rail's answer on the row; the intent stays
-- `canceled`; nothing is posted to the ledger; and an operator decides what
-- happens to the money. `vpay_db::settlement` is the only writer.
--
-- `requires_review` is NOT live: it is deliberately absent from
-- `charges_live_idx` (0014, rebuilt by 0037) and from
-- `vpay_db::payment_intents::LIVE_CHARGE_STATES`, so the poll ladder, the
-- stale-charge backstop and the settlement's own compare-and-swap all leave a
-- parked charge alone. That is what makes a second delivery of the same rail
-- answer a no-op instead of a second parking and a second alert. The index is
-- therefore untouched here.
--
-- Drop-and-add rather than an edit of 0037: that file's bytes are recorded in
-- MANIFEST.sha256 and an applied migration is never changed. The constraint
-- keeps its name, `charges_state_enum_check`, which is the name CrateStack
-- gives the CHECK it derives from `enum ChargeState` in schemas/vpay.cstack,
-- so the drift report matches it rather than proposing a drop-and-add pair.
-- The schema's enum gains the same label in the same change.
--
-- Widening a membership CHECK validates every existing row against the new
-- list, and every existing row holds one of the six old labels, so the ADD
-- cannot fail on data a 0058 database holds.

ALTER TABLE charges DROP CONSTRAINT charges_state_enum_check;

ALTER TABLE charges ADD CONSTRAINT charges_state_enum_check
    CHECK (state IN (
        'submitting',
        'submitted',
        'pending',
        'unresolved',
        'succeeded',
        'failed',
        'requires_review'
    ));

COMMENT ON CONSTRAINT charges_state_enum_check ON charges IS
    'The six charge_state labels migration 0004 created as a native enum, plus requires_review (0059, issue #96): a charge the rail reported paid after its intent was canceled, parked for an operator. Named the way CrateStack names a generated enum CHECK so the drift engine matches it.';

COMMENT ON COLUMN charges.state IS
    'TEXT since migration 0037, native charge_state enum before it. charges_state_enum_check carries the six labels the type did plus requires_review (0059). vpay_db::payment_intents::LIVE_CHARGE_STATES names the four non-terminal ones and charges_live_idx indexes them; requires_review is terminal for the rail and waits for an operator.';
