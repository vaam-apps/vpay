-- scan_deliveries: the backstop behind the webhook delivery queue.
--
-- WHY THE `jobs` VOCABULARY IS REOPENED AGAIN
--
-- Same argument as 0022's, and the same mechanism: the database is what
-- refuses a job no handler exists for, so `kind_is_known` changes in lockstep
-- with the handlers rather than being written permissively ahead of them.
-- 0022 shipped `fan_out_events` and `deliver_webhook` and said in its own
-- comment that `webhook_deliveries_live_idx` served "the backstop scan" —
-- which did not exist. `vpay_db::webhook_deliveries::pending_due` was written
-- and nothing called it (docs/plans/2026-09-03-step5-webhooks.md's Outcome
-- section listed that plainly under "What is not done"). This migration is
-- the kind that makes it real.
--
-- WHAT IT COVERS THAT THE QUEUE DOES NOT
--
-- Delivery is driven by `jobs`: the fan-out enqueues a `deliver_webhook` job
-- in the same transaction that creates the `webhook_deliveries` row, and each
-- failed attempt reschedules it. That transaction makes the job *exist*; it
-- does not make it survive an operator's DELETE or a `jobs` truncation during
-- an incident. Without a scan, such a delivery is owed an attempt that nothing
-- will ever make, and the merchant is never told about the payment behind it.
-- `scan_deliveries` is a singleton (dedupe key `scan:deliveries`) on exactly
-- the terms `scan_live_charges` is to poll jobs: in a healthy deployment it
-- finds nothing, and a steady stream from it means the enqueue is broken —
-- that is the bug, not this job.
--
-- WHAT IT DOES NOT COVER: A DEAD-LETTERED JOB
--
-- A dead letter is NOT a deleted row. `vpay_db::jobs::dead_letter` parks it at
-- `run_at = 'infinity'` and keeps its `dedupe_key` occupied, precisely so that
-- no scan re-creates work that is known to be unrunnable. So the scan's
-- `INSERT ... ON CONFLICT (dedupe_key) DO NOTHING` is a no-op for a delivery
-- whose job was parked, and that delivery stays `pending` forever unless a
-- human intervenes. Deliberate: a `deliver_webhook` job is parked for a
-- Poisoned reason (an event that will not render, a body whose digest no
-- longer matches what was signed), and none of them is fixed by retrying. The
-- scan names such rows in one WARN per pass instead; un-parking one is the
-- manual UPDATE in docs/runbooks/webhook-delivery-failures.md.
ALTER TABLE jobs DROP CONSTRAINT kind_is_known;
ALTER TABLE jobs ADD CONSTRAINT kind_is_known CHECK (kind IN
  ('poll_charge','resubmit_charge','sweep_expired','scan_live_charges','fan_out_events','deliver_webhook','scan_deliveries'));

COMMENT ON COLUMN jobs.kind IS
    'What to run, from a vocabulary closed by kind_is_known and mirrored exactly by vpay_worker::jobs::JobKind: poll_charge (ask the rail about one charge), resubmit_charge (send a charge again under its existing reference), sweep_expired and scan_live_charges (housekeeping singletons), fan_out_events (the outbox drain singleton), deliver_webhook (one POST to one merchant endpoint), scan_deliveries (the singleton backstop that re-enqueues a deliver_webhook job for a delivery whose own job was deleted or lost — not one that was dead-lettered, whose dedupe_key is still occupied by the parked row). A kind spelled here and not in that enum is a row no worker can dispatch; one spelled there and not here is refused at the insert.';

-- Corrects 0022's index comment, which described a scan that did not exist:
-- `webhook_deliveries_live_idx` is now genuinely `pending_due`'s index, and
-- `pending_due` genuinely has a caller. The predicate is unchanged — the
-- never-attempted arm the scan added (`next_attempt_at IS NULL AND created_at
-- < now() - lease`) is served by the same partial index, because a btree
-- indexes NULLs.
COMMENT ON INDEX webhook_deliveries_live_idx IS
    'The scan_deliveries backstop''s query and an operator''s "what is outstanding right now?": deliveries still owed an attempt, oldest due first. Partial on state = ''pending'' so the index stays the size of the outstanding work rather than of the delivery log.';
