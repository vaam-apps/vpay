-- jobs.kind_is_known reopened for one kind: ensure_callbacks (RFC-0013,
-- delivery steps 2-4, 2026-10-22).
--
-- WHAT THE JOB IS. A singleton on dedupe key `ensure:callbacks`, seeded at
-- every worker start beside the five `seed_singletons` already seeds, that
-- makes sure each rail whose adapter declares `CallbackUrlMode::Registered`
-- has one callback endpoint vpay created, registered with the rail account,
-- at `{public_base_url}/provider/{code}/callback`. It reschedules itself
-- hourly, and sooner after a transient failure. Notch Pay is the first such
-- rail: until this job, its webhook endpoint was registered by hand.
--
-- WHY A JOB, AND NOT THE BOOT PATH. A boot that waits on a provider's API is
-- a deployment the provider can take down, and `vpay-server` is the surface
-- that autoscales: every scale-out would re-register against every provider
-- at once (RFC-0013 section 2). The worker job never fails a process and
-- never dead-letters: its handler always answers with a reschedule, because
-- a parked singleton keeps its dedupe key and seed_singletons' ON CONFLICT DO
-- NOTHING could never revive it (0023's comment, lines 30-33).
--
-- WHY ONLY THIS KIND. The mechanism 0022, 0023 and 0034 describe: the
-- database refuses a kind no code writes, so the CHECK moves in lockstep with
-- vpay_worker::jobs::JobKind, in the commit that adds the handler.
ALTER TABLE jobs DROP CONSTRAINT kind_is_known;
ALTER TABLE jobs ADD CONSTRAINT kind_is_known CHECK (kind IN
  ('poll_charge','resubmit_charge','sweep_expired','scan_live_charges','fan_out_events','deliver_webhook','scan_deliveries','sweep_idle_customers','ensure_callbacks'));

COMMENT ON COLUMN jobs.kind IS
    'What to run, from a vocabulary closed by kind_is_known and mirrored exactly by vpay_worker::jobs::JobKind: poll_charge, resubmit_charge, sweep_expired, scan_live_charges, fan_out_events, deliver_webhook, scan_deliveries, sweep_idle_customers (0034 — the twelve-month retention sweep, a singleton on dedupe key sweep:customers), and ensure_callbacks (0051 — RFC-0013: keep one vpay-created callback endpoint registered with each rail account that registers its URL, a singleton on dedupe key ensure:callbacks). A kind spelled here and not in that enum is a row no worker can dispatch; one spelled there and not here is refused at the insert.';
