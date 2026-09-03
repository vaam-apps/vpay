-- jobs: the worker's durable queue, and the `charges.provider_txn_id`
-- column the rail's own transaction identifier goes into.
--
-- WHY A TABLE AND NOT A QUEUE PRODUCT
--
-- The one thing this queue must do is share a transaction with the write
-- that produces the work. `confirm` commits the charge row *before* it calls
-- the rail (docs/flows/crash-safety.md), and the job that will later poll
-- that charge is enqueued in **that same transaction** — so all three of
-- crash-safety.md's kill points leave a job behind, and no scan is
-- load-bearing for recovery. An external broker cannot be enrolled in a
-- Postgres transaction, so "enqueued but the charge rolled back" and "charge
-- committed but nothing enqueued" both become reachable states. Neither is
-- reachable here.
CREATE TABLE jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- What to run. Closed vocabulary, see `kind_is_known`.
    kind TEXT NOT NULL,
    -- The idempotency key of the *work*, not of a request: `poll:<charge_id>`
    -- is the same job however many times something asks for it, so the
    -- backstop scan and the confirm path can both enqueue it with
    -- `ON CONFLICT DO NOTHING` and exactly one row results.
    dedupe_key TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- When the job becomes claimable. `now()` for work that is ready, a
    -- future instant for a rescheduled poll — the poll ladder is expressed
    -- entirely by moving this column.
    run_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Incremented by the claim itself, so a job that repeatedly kills its
    -- worker still counts up rather than being retried forever at zero.
    attempts INT NOT NULL DEFAULT 0,
    -- The lease. Both columns move together (`lock_is_paired`); `locked_by`
    -- is the worker id, and it is the guard on every write that ends a lease
    -- (see `finish`/`reschedule` in vpay_db::jobs) — the same ABA close
    -- `idempotency_keys.claim_id` is, for the same reason.
    locked_at TIMESTAMPTZ, locked_by TEXT,
    last_error TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Mirrors `events.type_is_a_documented_event` (0018): the vocabulary is
    -- closed, and it is closed *at the database* so a handler that does not
    -- exist cannot be enqueued for. `deliver_webhook` is deliberately absent
    -- — Step 5 adds it in its own migration, so this step cannot enqueue a
    -- webhook delivery by accident and then silently never run it.
    CONSTRAINT kind_is_known CHECK (kind IN ('poll_charge','resubmit_charge','sweep_expired','scan_live_charges')),
    -- Same argument as `payment_intents.metadata_is_object` (0014): a
    -- payload that is `[1,2]` or `"x"` is a payload no handler can
    -- deserialise, and the failure would surface in the worker rather than
    -- at the enqueue that caused it.
    CONSTRAINT payload_is_object CHECK (jsonb_typeof(payload) = 'object'),
    -- Mirrors `provider_requests.response_is_paired` (0016). "Leased" and
    -- "leased by someone" are one fact, so they may not disagree: a row with
    -- a `locked_at` and no `locked_by` would be invisible to `claim` (it is
    -- locked) and unreleasable by `finish` (nobody holds it), i.e. stuck
    -- forever with nothing in the row saying why.
    CONSTRAINT lock_is_paired CHECK ((locked_at IS NULL) = (locked_by IS NULL)),
    -- The same 2000-character ceiling `charges.failure_raw` has. A rail's
    -- text is captured for operators, not stored in full, and the writer
    -- truncates to this bound before binding — this CHECK is the backstop
    -- for a writer that forgets, not the primary guard.
    CONSTRAINT last_error_length CHECK (last_error IS NULL OR char_length(last_error) <= 2000)
);

-- What makes `ON CONFLICT (dedupe_key) DO NOTHING` legal, and what makes
-- "one poll job per charge" a property of the schema rather than of
-- whichever enqueue happened to run first.
CREATE UNIQUE INDEX jobs_dedupe_key ON jobs (dedupe_key);

-- The claim's own subquery: `WHERE run_at <= now() AND locked_at IS NULL
-- ORDER BY run_at`. Partial on `locked_at IS NULL` so the index stays the
-- size of the *runnable backlog* rather than of the table — in a healthy
-- system that is near-empty even while many jobs are in flight.
--
-- This is why the claim's predicate is `locked_at IS NULL` and not
-- "unlocked or the lease expired": the latter is not an index predicate
-- (it depends on `now()`), so it would degrade this into a scan of every
-- leased row on every claim. Lease expiry is a separate reaper instead
-- (`vpay_db::jobs::reap_expired_leases`, run by the `sweep_expired` job).
CREATE INDEX jobs_claimable_idx ON jobs (run_at) WHERE locked_at IS NULL;

-- The reaper's half of the same split: the leases, ordered by age.
CREATE INDEX jobs_leased_idx ON jobs (locked_at) WHERE locked_at IS NOT NULL;

COMMENT ON TABLE jobs IS
    'The worker''s durable queue. Enqueued in the same transaction as the write that creates the work (docs/flows/crash-safety.md), claimed with UPDATE ... FOR UPDATE SKIP LOCKED, and completed by DELETE ... WHERE id AND locked_by. Lease expiry is reaped separately so the claim predicate stays an exact index match.';
COMMENT ON COLUMN jobs.dedupe_key IS
    'Idempotency key of the work itself (e.g. poll:<charge_id>), unique across the table — every enqueue is ON CONFLICT DO NOTHING.';

-- The rail's own identifier for the money movement, learned from
-- `ChargeStatus::Succeeded { provider_txn_id }`.
--
-- WHY A COLUMN AND NOT `provider_ref_extra`
--
-- Exactly the argument 0019 makes for `return_url`, in the other direction.
-- `provider_ref_extra` is `vpay_provider::RefExtra` — *rail key material the
-- core persists in order to query status later* (Orange's `pay_token`), and
-- `docs/flows/crash-safety.md` allows a callback repair to overwrite that
-- document wholesale. A settlement identifier written into it would be
-- destroyed by such a repair, and it is the one field
-- `docs/runbooks/unresolved-charges.md` step 4 needs by name to reconcile a
-- charge against the rail's dashboard.
--
-- Nullable, and only ever written by the settlement transaction
-- (`vpay_db::settlement::apply_succeeded`): a charge that has not succeeded
-- has no transaction identifier, and inventing one would be a fabrication of
-- exactly the kind AGENTS.md rule 2 forbids. It is also not unique — vpay
-- has no basis for asserting a rail never reuses an identifier across its
-- own tenants, and a unique index that is wrong refuses a settlement that
-- actually happened.
ALTER TABLE charges ADD COLUMN provider_txn_id TEXT;

ALTER TABLE charges
    -- Bounded, and non-empty. The upper bound is generous (both rails vpay
    -- speaks to today return identifiers well under 40 characters) because
    -- the column's job is to be *recorded faithfully*, not to be validated;
    -- what the CHECK actually rules out is the two shapes that would be
    -- lies: an unbounded blob from a misparsed response, and `''`, which
    -- reads as "there is an identifier" while carrying none. A rail that
    -- genuinely returns no identifier leaves this NULL.
    ADD CONSTRAINT provider_txn_id_length
        CHECK (provider_txn_id IS NULL OR char_length(provider_txn_id) BETWEEN 1 AND 128);

COMMENT ON COLUMN charges.provider_txn_id IS
    'The rail''s own identifier for the settled payment (vpay_provider::ChargeStatus::Succeeded.provider_txn_id), written only by the settlement transaction. NOT rail key material — that is provider_ref_extra, which a callback repair may overwrite wholesale (0019).';
