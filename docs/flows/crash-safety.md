# Crash safety

## The invariant

> **Never let a payer act on a transaction you cannot later name.**

Everything below is an application of that one sentence. The two flow shapes
enforce it at different moments, because the payer becomes able to act at
different moments.

## Push rails (MTN MoMo)

MTN acknowledges `requesttopay` with **202 and an empty body**. There is no
transaction id in the response — *the id is the `X-Reference-Id` you sent*.

So the payer's handset starts buzzing before you learn whether your request
succeeded. Generate the reference in memory, call the rail, crash before writing
it down, and you have created a payment you can never observe.

```
BEGIN;
  INSERT INTO charges (…, provider_reference_id, state='submitting');
COMMIT;                        -- the reference is now durable
  INSERT INTO provider_requests (…, status_code = NULL);   -- "about to send"
  POST /collection/v1_0/requesttopay   (X-Reference-Id = provider_reference_id)
  UPDATE provider_requests SET status_code = …;            -- "heard back"
UPDATE charges SET state='submitted';
```

**Write first, network second. Always.**

### Retry rule

If submit times out or errors, **do not generate a new reference.** Retry with
the same one. The adapter contract requires a duplicate submission to be
reported as `Submitted` rather than an error — that is what makes this safe. A
fresh reference on retry is how you double-charge a customer.

### Recovering a `submitting` charge

`submitting` covers two physically different situations. Disambiguate with
`provider_requests`:

| Evidence | What happened | Action |
|---|---|---|
| Charge younger than 60 s | A confirm may still be running | **Wait.** Reschedule once, for the rest of the window, and touch nothing |
| No `provider_requests` row | Crashed before the POST | **Resubmit**, same reference |
| Row exists, `status_code IS NULL` | POST issued, response lost | **Poll**. On `NotFound`, retry the poll; only after 3 consecutive `NotFound` over ≥60s treat it as never-received and resubmit with the same reference |
| Row has a status code | Normal path | Advance state from the code |

**Added 2026-09-04 (Step 8, lane G). The table applies only to a charge that
has been `submitting` for at least `not_found_window` (60 s).** That state is
not only what a crash leaves: it is also the ordinary state of a confirm that is
still inside its rail call, because the charge and its poll job are committed
*before* the network call and the `submitting → submitted` compare-and-swap
happens after it. Younger than the window, nothing on disk distinguishes the
two, and every row above would move a charge out from under a live confirm —
which is what Step 8's demo observed four times in six runs. The age is read
from `charges.created_at`, because the first row of the table has no
`provider_requests` row to read a time from at all. **Amended 2026-09-04
(Step 8, lane H).** The wait costs the charge one rung of the ladder and not
six: `RecoveryAction::Wait` carries `not_found_window - age` and the poll
comes back once, when the age guard will pass, so a genuinely crashed charge
starts its real recovery at `poll_delay(1)` — twenty seconds — rather than at
`poll_delay(6)`. The age itself is measured by Postgres at both ends
(`Charges::get_by_id_as_of` selects `now()` beside the row), because a window
computed from the worker host's clock is a window a fast host does not have.

A bare `NotFound` is **never** on its own grounds to fail a charge. Resubmission
is always safe, so every ambiguity resolves toward "find out", never "give up".

The middle row needs **both** conditions, never either: three polls can happen
in under a second on the first rungs of the ladder, and a rail that is merely
slow to index a new charge would look identical to one that never got it.

**The table applies only while the charge is `submitting`.** Past that state the
rail has answered our submit and, on a redirect rail, the payer holds a URL —
so the "abandon it" row below would discard a live payment. A `NotFound` on a
`submitted` charge is therefore an ordinary pending answer: the ladder keeps
running and the streak is recorded for an operator rather than acted on. The
recovery decision is reached from two places, the crash-recovery step at the top
of a poll and a `NotFound` the poll itself received, and both go through the
same function ([../reference/vpay-worker.md](../reference/vpay-worker.md)
§"Recovering a `submitting` charge").

## Redirect rails (Orange Money)

The ordering is reversed, and it is safe for a reason worth stating plainly.

```
INSERT charge (state='submitting', provider_reference_id = order_id);  COMMIT
POST /webpayment  →  { pay_token, payment_url }
UPDATE charge SET provider_ref_extra = {pay_token…}, state='submitted';  COMMIT
                                     ↑
                    the payer is redirected ONLY after this commit
```

**If the submit response is lost, no payment can have occurred** — the payer was
never given the URL. That `order_id` is dead: abandon it and let the merchant
create a new PaymentIntent. This is the one place where "the response was lost"
is genuinely benign, and it is benign only because the payer's route in is a URL
you must hand them.

What is *not* safe is emitting `redirect_to_url` before `ref_extra` is
committed. Do that and a crash strands a payer mid-payment on the rail's page
against a charge you cannot query.

**The commit is the gate on the redirect.**

**Since Step 9 one more value is written before that first commit rather than
after it: the charge's `return_url`.** When a Checkout Session drives the
charge, `charges.return_url` holds vpay's own return page —
`{checkout.public_base_url}/c/{cs_id}/return?t={return_token}&key={pk}` — and
it is written *before* the charge is committed and read back from that row at
submit. So what the rail is told, what `next_action.redirect_to_url.return_url`
renders on every later read, and what a resubmit would send are **one column**
rather than three reads that could differ. The alternative — resolving it
between the write and the rail call — left a window in which the durable row
and the rail disagreed about where the payer would land
([hosted-checkout.md](hosted-checkout.md)).

## Why Orange is integrable at all

Orange's `transactionstatus` requires `order_id` + `amount` + `pay_token`, and
`pay_token` exists only in the submit response. Under a naive reading of the push
precondition ("status must be queryable by a reference you generated"), that
disqualifies it.

It does not, because of the asymmetry above. This is exactly why the
preconditions are stated **per flow shape** rather than universally.

## Tests

The three kill points are: after the charge insert and before any
`provider_requests` row; after that row and before the response; after the
response and before the state update. Each must resolve without
double-charging.

**They are exercised by writing the state a crash leaves, not by killing a
process — and since 2026-09-04 (Step 8, lane D) two of the three are *also*
exercised by killing one.** `backends/tests/integration/tests/worker_recovery.rs` builds each of
the three states directly against a real Postgres — commit the charge and no
attempt row; add an attempt row with `status_code IS NULL`; add one carrying a
status — then runs the real handler against a real WireMock rail and asserts
the recovery table resolves it. The decisive assertion in each is that **every
`provider_requests` row for the charge carries the same
`provider_reference_id`** (`assert_one_reference`), which is the property the
retry rule exists for.

Each of those three states is written **ninety seconds in the past**
(`support::age_the_crash`), which is not a convenience: since lane G a charge
inserted a millisecond ago is indistinguishable from a confirm that is still
running, and the recovery table refuses it. The suite's fourth and fifth cases
are that refusal — the same fixtures unaged, asserting nothing moves — and its
sixth (`a_confirms_compare_and_swap_wins_against_the_worker_that_claimed_its_poll_job`)
runs the shipping loop against the confirm's own compare-and-swap.

That was a weaker claim than "a `SIGKILL` at each point resolves cleanly", and
it was stated that way on purpose: it proves the recovery table, not the
process's behaviour under a signal. ~~Nothing in this repository kills a process
mid-confirm.~~ **Retired 2026-09-04 (Step 8, lane D):
`backends/tests/integration/tests/worker_kill9.rs` does.** It spawns the
shipping binary as real OS processes — `vpay-server worker` and `vpay-server`
(it spawned two different binaries, `vpay-worker-bin` and `vpay-server`, until
issue #77 made them one on 2026-09-07; the suite now spawns one binary twice
with different argv, and that is what makes both scenarios a test of the
subcommand as well as of the loop) —
against a real Postgres and a real WireMock rail, makes the rail slow at exactly
one point (a 30 s `fixedDelayMilliseconds` mapping armed by a documentation
MSISDN — longer than `vpay_provider::DEFAULT_REQUEST_TIMEOUT`, so a late kill
cannot let the request quietly settle the charge), and `Child::kill()`s the
process once two independent witnesses agree the request is in flight. The exit
status is asserted **signalled with 9** in both cases, and additionally
`code() == None` in the mid-poll one (`worker_kill9.rs:961-971`, against
`:1203-1207`), so a process that chose to `exit(1)` could not stand in for one
that was killed.

- **`a_worker_killed_mid_poll_settles_the_charge_exactly_once_after_its_lease_is_reaped`**
  — the worker dies mid-status-query. The lease, held by the dead process's own
  `worker_id`, is the only trace; a second worker reaps it at boot, re-runs the
  poll, and the charge settles exactly once by four independent counts,
  including the rail's own journal showing one submit and two status queries.
- **`a_server_killed_mid_submit_leaves_a_charge_the_worker_settles_without_a_second_submit`**
  — kill point 2, staged against the shipping server with no test-only seam. The
  worker recovers by polling and never resubmits: **one** submit in the journal,
  which is what the retry rule is actually about.
- **`a_worker_sigtermed_mid_delivery_drains_it_and_the_merchant_is_told_exactly_once`**
  — the third scenario, added 2026-09-10 for issue #85, and about the *other*
  signal. It is described under "The graceful stop" below.

**Two clocks are simulated in that file, and nothing else is.**
`age_the_dead_workers_lease` moves `jobs.locked_at` ten minutes back, guarded on
the dead worker's own `worker_id`, because `RecoveryPolicy`'s five-minute lease
has no CLI override and the test cannot wait it out.
`age_the_crashed_charge` moves `charges.created_at` ten minutes back, guarded on
`state = 'submitting'`, because lane G's minimum charge age means a
freshly-killed server's charge is — correctly — indistinguishable from a live
confirm. That second clock is load-bearing and was measured to be: added on the
integration branch after lane G merged, because without it the server-kill case
failed there with the worker correctly waiting. The processes, the signal, the
reap, the claim and the re-run are all the shipping binaries' own.

**Kill point 1 is still written rather than caused**, and for a reason rather
than for want of trying: it is the moment *before* the reference is minted, so
there is no network call for a signal to land during. `worker_recovery.rs`
remains the only proof of that case. Neither kill case exercises Orange, and the
rail is a WireMock container in both.

**Status: implemented, and driving payments. Updated 2026-09-03 (Step 4);
re-verified 2026-09-07 twice — for issue #77, and again for migration `0037`
(S5, the money tables through CrateStack); extended 2026-09-10 (issue #85)
with the SIGTERM scenario, which is the first automated case in this
repository that signals a shipping process holding outstanding work.**

**Nothing in this document's behaviour changed for `0037` either, and the
migration is the reason to say so explicitly.** It converted
`charges.state`, `charges.failure_code`, `payment_intents.status`,
`payment_intents.last_payment_error_code`, `refunds.status` and
`refunds.failure_code` from native Postgres enums to `TEXT` plus a membership
CHECK carrying exactly the same labels — a change to the *type* of the columns
every rule on this page is written in terms of. Every write-before-network
ordering, every compare-and-swap and the settlement statement itself are
byte-for-byte what they were apart from dropped casts; **no write on any money
table moved to the generated data layer**, so the one-statement settlement,
the confirm path's two-row transaction and the `provider_requests` audit trail
are untouched (`docs/reference/vpay-db.md` § "The money tables: what moved,
and what stays raw forever" says why, and it is a `jsonb` blocker rather than
a choice).

What was re-run to say so rather than assume it: `worker_kill9`'s two
scenarios, both green — one `charges` row, one `payment_intent.succeeded`
event, `amount_received` written once, and no second submit at the rail after
a real `SIGKILL`. `worker_recovery`'s 23 are green as well, including the
three that reach `unresolved`, which is the state the `charge_state`
conversion would have broken first.

**One ordering rule this migration adds, and it is an operator's rule rather
than a payer's:** `0037` is not backward compatible with the previous binary,
in two different ways, both measured on 2026-09-08 against real databases by
building 889d045's `vpay-db`. A previous-release process that **restarts**
fails at boot in `run_migrations()`
(`sqlx::MigrateError::VersionMissing(37)`) and never serves — the same thing
0032 does on a restart. A previous-release process that **keeps running**
serves reads normally and fails money **writes** with `42704 type
"intent_status"/"charge_state" does not exist`; that in-flight window is what
is new here, and it is quiet, because nothing a merchant can see goes dark.
Drain the previous version before it lands, and do not roll past it. The
migration's own header carries the detail and the measured statements.

**Nothing in this document's behaviour changed on 2026-09-07**, and that is
the claim the update is here to make. Issue #77 folded `vpay-worker-bin` into
`vpay-server` as a `worker` subcommand: one package, one binary, one image.
`vpay-worker` — the crate holding every handler, the claim, the lease, the
drain and the recovery table — was not touched, and neither was the boot order
the loop runs behind. What was re-run to say so rather than assume it:
`worker_kill9`'s two scenarios (both green through `vpay-server worker`),
`worker_recovery`'s 23 (green, and green *even against a `worker` subcommand
mutated to do nothing at all* — it drives `run_loop`/`run_once` in-process and
never spawns a binary, so it is `worker_kill9` and the ten
`vpay-server::cli worker::*` cases that hold the subcommand honest, not this
suite).

### The graceful stop is a different property from `kill -9`, and since 2026-09-10 it is in the suite

Everything above is about a process that is **killed**. A SIGTERM with work
outstanding is the other half. ~~and no automated case reaches it: the suites
send SIGTERM only through `stop_worker_cleanly`, whose own assertion string
says "a worker with nothing in flight" […] **This is a measurement, not a
test.** Nothing re-runs it, and a regression in the drain would be caught by
no gate. Turning it into a third `worker_kill9` scenario is the obvious answer
and was not done in this pass.~~ **Retired 2026-09-10 (issue #85): it was
done.**

- **`a_worker_sigtermed_mid_delivery_drains_it_and_the_merchant_is_told_exactly_once`**
  — an ordinary payment settles, its `payment_intent.succeeded` webhook is
  claimed by one of **two** running workers, and that worker is sent a real
  `SIGTERM` while its POST is inside the merchant's receiver. It exits **0**
  with the drain log line, having **finished the delivery**: one delivery row
  `succeeded` with `attempt = 0`, its job deleted, and **one** signed POST in
  the receiver's own journal — which
  `vpay_sdk::webhooks::verify` accepts under the configured secret. The
  four-record exactly-once invariant the two `SIGKILL` cases assert holds too.

The receiver is made slow the same way the rail is in the cases above — a
`fixedDelayMilliseconds` mapping (`slow-ack.json`, 6 s) — and the number is
chosen against two shipping budgets rather than against a stopwatch: **below**
`vpay_worker::webhooks::WEBHOOK_REQUEST_TIMEOUT` (10 s), so the
acknowledgement always beats the delivery client's own deadline, and **below**
the worker's `--shutdown-grace-seconds` (20 s), so the drain can never run out
of time. Both comparisons are `const` assertions in `worker_kill9.rs`, so
lowering either budget breaks the build rather than making the case flaky. And
because a margin is not a proof, the case reads the signalled worker's own
transcript afterwards and **fails** unless `shutdown signalled; draining
in-flight jobs` precedes `webhook delivered` in it: a signal that arrived
after the receiver had already answered makes the run say so rather than pass
while proving nothing.

**Why two workers rather than a restart.** The hand measurement restarted the
container; the test cannot, and the reason is the property it is trying to
prove. A worker spawned *after* the signal has connected, migrated and
reconciled by the time the drain is over, so the job it must not steal no
longer exists and the lease would be respected vacuously — on some runs and
not others, with nothing able to tell which. A worker already running when the
signal lands overlaps by construction, and that is what makes the second
decisive mutation bite: **remove `locked_at IS NULL` from `Jobs::claim`** and
the surviving worker claims the delivery the draining one still holds, sending
the merchant a second identical signed POST. Measured: `left: 2  right: 1` on
the receiver's journal. **Remove the drain** (return `Drain::Clean` the
instant the signal is seen) and the same case fails with *the worker never
logged `webhook delivered`* — the send was cut off after the receiver had
already *received* it (WireMock journals a request when it matches it and
answers `slow-ack.json`'s 200 six seconds later, so "received" is the exact
word and "accepted" would not be), which is the lost delivery that becomes a
double send as soon as anything retries.

The original hand measurement is unchanged and still recorded — `docker kill
-s TERM` on the demo stack, exit **0**, `Drain::Clean` rather than the
`Drain::TimedOut` branch that exits `1`, an undelivered webhook that stayed
undelivered while no worker ran and was delivered in ~6 s, signed, by the
same container restarted. Details in
[../plans/exp30-single-binary-notes/opus-review.md](../plans/exp30-single-binary-notes/opus-review.md).

**What the new case does not cover, plainly.** It asserts the *clean* drain
only. `Drain::TimedOut` — the grace period elapsing with a job still in
flight, exit `1`, leases handed back — is proven by
`a_drain_that_runs_out_of_grace_releases_every_lease_it_still_holds`
(`worker_e2e.rs`) against `run_loop` in-process, not by a signalled shipping
binary; and no case restarts a worker after a graceful stop, which is the one
half of the hand measurement that is still only a measurement.

**What was already true (Step 3), unchanged.**
`POST /v1/payment_intents/{id}/confirm`
(`backends/crates/vpay-api/src/v1/payment_intents.rs`) performs exactly the
ordering this document requires, and in this order:

1. mint the `provider_reference_id`;
2. **commit** the charge row in `submitting` carrying that reference — and,
   since Step 4, **the `poll_charge` job that will drive it**, in the *same*
   transaction — before any network call;
3. insert a `provider_requests` row with `status_code IS NULL` (migration
   `0016`);
4. call the adapter's `submit`;
5. record what came back on that row.

The redirect half is unchanged too: the merchant's `return_url` is committed on
the charge row at step 2 (`charges.return_url`, migration `0019`), and the
rail's `pay_token` + `redirect_url` are committed at step 5 in **one**
transaction — after which, and only after which, `next_action` is built, from
the committed row rather than from the adapter's return value
(`redirect_confirm_commits_the_rails_material_before_it_answers`;
`an_unreachable_rail_leaves_the_charge_where_recovery_expects_it`;
`a_second_confirm_cannot_produce_a_second_charge`).

**What Step 4 adds: the recovery table is now read by code.**

- **The job comes with the charge.** `vpay_db::TxRepositories::enqueue_in_tx` runs inside
  step 2's transaction, so all three kill points leave a job behind. The
  alternative — enqueueing beside step 5 — would have left kill points 1 and 2,
  precisely the recovery cases, with a committed charge and nothing that would
  ever ask the rail about it. The hourly `scan:live` backstop covers only what
  that transaction cannot: charges written before the queue existed, and a job
  lost to operator error.
- **The table itself** is `vpay_worker::recovery::recovery_step`, a pure
  function over (flow shape, latest submit attempt, `NotFound` streak, **charge
  age**, window).
  `SubmitAttempt::{Never, Unanswered, Answered(code)}` are the three rows above;
  `Answered` includes migration `0020`'s `0` sentinel. **Amended 2026-09-04
  (Step 8, lane H): those ages are `Duration`s, not instants**, and they are
  computed from `Charges::get_by_id_as_of`, which selects `now()` on the same
  statement that reads the row. `recovery_step` and `past_the_horizon` used to
  subtract `charges.created_at` from the *worker host's* clock, so a worker a
  minute fast measured every charge as a minute older than it was and the guard
  above became a silent no-op; taking durations leaves no parameter for a
  caller to read off the wrong clock
  (`the_age_is_measured_by_the_database_and_not_by_this_host`,
  `the_charge_read_carries_the_databases_own_clock_beside_the_row`).
- **The charge's age decides first, then the flow shape.** A **redirect** charge still in `submitting`
  is failed (`provider_unavailable`, intent back to `requires_payment_method`):
  the payer was never handed a URL, and the `pay_token` needed to ask the rail
  about the order was in the response that was lost, so that `order_id` is dead
  — this document's own conclusion, now executed
  (`a_redirect_charge_with_no_token_is_failed_without_ever_asking_the_rail`).
  The branch is on `Capabilities::flow`, a capability *value*, never a rail code
  (ADR-0002). **The redirect branch is unconditional *within* the table but no
  longer unconditional overall (2026-09-04, lane G):** a redirect charge younger
  than the window is left alone, because `FailDeadOrder` is correct only for a
  submit response that was genuinely lost and catastrophic for one that is about
  to arrive
  (`a_young_redirect_charge_is_not_failed_as_a_dead_order_until_it_is_older`).
- **The resubmit rule holds.** `resubmit_charge` reads
  `charges.provider_reference_id` and never mints one
  (`a_charge_whose_submit_never_left_is_resubmitted_under_the_same_reference`).
  The `NotFound` threshold is both conditions, never either: 3 consecutive
  answers **and** ≥60 s (`three_not_founds_over_the_window_resubmit_and_two_do_not`,
  which drives the policy at 50 ms so it needs no sleeps).
  `charges::mark_submitted` **merges** `provider_ref_extra` (`||`) rather than
  assigning it, so a push rail's empty answer on a second submit cannot erase
  key material a first answer wrote.
- **An answered submit advances rather than resubmits**
  (`an_answered_submit_advances_the_bookkeeping_rather_than_submitting_again`):
  kill point 3 is a bookkeeping repair, not a rail call.
- **A settlement lands on the intent a crashed confirm left behind.** Because
  the confirm moves the intent only *after* the rail answers, all three kill
  points leave a live charge against an intent still reading
  `requires_payment_method` — so the settlement transaction's intent guard
  accepts that status alongside `processing` and `requires_action`
  (`vpay_db::payment_intents::SETTLEABLE_STATUSES`). Excluding it, as the first
  implementation did, made the recovered charge dead-letter instead of settling
  (`a_settlement_lands_on_the_intent_a_crashed_confirm_left_behind` — a review
  finding, and the reason that constant is documented at length).
- **A worker's own crash is recovered too.** Leases are reaped at worker boot,
  every `lease / 2` on a dedicated timer, and inside the hourly sweep
  (`a_lease_stranded_by_a_crash_is_freed_at_boot_before_any_sweep_runs`,
  `a_lease_that_expires_while_the_worker_runs_is_reaped_on_its_own_timer`).

**Updated 2026-09-03 (Step 7, Phase A review): a rolled-back transaction cannot
turn a report into a `503`.** `vpay_db::UnitOfWork::transaction` rolls back
explicitly on `TxOutcome::Abandon`, and a rollback that fails is logged at
`warn!` and swallowed rather than returned. `ROLLBACK` is best-effort by
construction — a transaction whose connection died is aborted by the server
either way — so the failure changes nothing about the database and only about
what the caller may say, and the two abandoning call sites are exactly the ones
whose answer matters most here: the confirm path's duplicate-charge recovery
owes the merchant its `409`, and `persist_submitted` owes an operator the
`Internal` alert saying *the rail may hold a live payment*. Losing either to a
storage error would lose the only report of it
(`an_abandoned_transaction_survives_a_rollback_it_cannot_send`, `vpay-db/tests/
postgres.rs`, which stages it by terminating the backend that holds the open
transaction and uses the commit path as its control).

**What is still not built.**

- ~~**No `SIGKILL` test.**~~ **Narrowed 2026-09-04 (Step 8, lane D) to kill
  point 1 only.** Kill points 2 and the mid-poll crash are proven by a real
  signal to a real shipping process (`worker_kill9.rs`, Tests above). Kill
  point 1's state is still written rather than caused, because there is no
  request to interrupt at that instant. **Orange Money is not exercised by
  either kill case**, and a redirect-rail kill test would need its own scenario
  (the ordering is reversed — see "Redirect rails" above).
- ~~**No callback route**, so a rail that tries to tell us about a charge is
  ignored~~ — **retired 2026-09-04 (Step 8, lane C): a rail that tells us about
  a charge is now heard.** The callback route pulls that charge's poll forward
  instead of leaving it to the ladder's next rung. It changes nothing about
  recovery — the authenticated status query is still the only thing that settles
  anything, and every kill point above resolves identically whether a callback
  arrives or not. What is still missing is a rail that has actually called it.
  See [reconciler.md](reconciler.md).
- **The rails are WireMock hosts.** Every recovery case above is proven against
  a stub speaking the documented protocol. No real rail has ever been called,
  so what is proven is that the recovery table is executed correctly, not that
  the rails behave as these documents claim.

See [../status.md](../status.md).
