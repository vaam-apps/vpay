-- provider_requests.operation_is_known reopened for one operation: prepare
-- (ADR-0032, 2026-10-23).
--
-- WHAT THE OPERATION IS. The first of a two-step rail's two calls:
-- `vpay_provider::ProviderAdapter::prepare` creates the payment on the rail
-- without prompting the payer, and returns the reference the rail issued for
-- it. The core commits that reference on the charge (`provider_ref_extra`)
-- before it records a `submit` row and calls `submit`, which is the prompt.
-- Notch Pay is the first such rail: its initialize call issues the `trx.…`
-- reference it is addressed by from then on, and finds no payment by ours.
--
-- WHY A ROW AT ALL. A prepare calls the rail and sends it the payer's number,
-- so it is an attempt to talk to a rail in 0016's sense, and an operator
-- reconciling against the rail's dashboard needs it as much as a submit. It
-- is written before the call and answered after it, exactly as a submit is.
--
-- WHY ITS OWN OPERATION, AND NOT A `submit` ROW. Recovery reads the newest
-- `submit` row (`latest_submit_attempt`, `vpay_worker::recovery_step`), and
-- the whole of ADR-0032 rests on what the *absence* of one means: no `submit`
-- row for a charge is proof no call ever asked the rail to prompt its payer,
-- because every build records one before its first submit call. A prepare
-- filed as a `submit` would make that proof false the moment it was written.
--
-- WHY ONLY THIS VALUE. The CHECK moves in lockstep with the code that writes
-- the column, in the commit that adds the writer, as 0051 did for jobs.kind.
ALTER TABLE provider_requests DROP CONSTRAINT operation_is_known;
ALTER TABLE provider_requests ADD CONSTRAINT operation_is_known CHECK (operation IN
  ('prepare', 'submit', 'query_status', 'refund'));

COMMENT ON COLUMN provider_requests.operation IS
    'Which port call this attempt was, from a vocabulary closed by operation_is_known: prepare (0052, ADR-0032: a two-step rail creating the payment and issuing its own reference, no payer prompted), submit (the call that asks the rail to take the payment; on a two-step rail, the prompt, recorded only once prepare''s reference is committed), query_status and refund. Recovery reads submit rows only, and the absence of one is the proof that no payer was prompted.';
