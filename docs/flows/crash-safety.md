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
| No `provider_requests` row | Crashed before the POST | **Resubmit**, same reference |
| Row exists, `status_code IS NULL` | POST issued, response lost | **Poll**. On `NotFound`, retry the poll; only after 3 consecutive `NotFound` over ≥60s treat it as never-received and resubmit with the same reference |
| Row has a status code | Normal path | Advance state from the code |

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
process.** `backends/tests/integration/tests/worker_recovery.rs` builds each of
the three states directly against a real Postgres — commit the charge and no
attempt row; add an attempt row with `status_code IS NULL`; add one carrying a
status — then runs the real handler against a real WireMock rail and asserts
the recovery table resolves it. The decisive assertion in each is that **every
`provider_requests` row for the charge carries the same
`provider_reference_id`** (`assert_one_reference`), which is the property the
retry rule exists for.

That is a weaker claim than "a `SIGKILL` at each point resolves cleanly", and
it is stated this way on purpose: it proves the recovery table, not the
process's behaviour under a signal. Nothing in this repository kills a process
mid-confirm.

**Status: implemented, and driving payments. Updated 2026-09-03 (Step 4).**

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
  function over (flow shape, latest submit attempt, `NotFound` streak, window).
  `SubmitAttempt::{Never, Unanswered, Answered(code)}` are the three rows above;
  `Answered` includes migration `0020`'s `0` sentinel.
- **The flow shape decides first.** A **redirect** charge still in `submitting`
  is failed (`provider_unavailable`, intent back to `requires_payment_method`):
  the payer was never handed a URL, and the `pay_token` needed to ask the rail
  about the order was in the response that was lost, so that `order_id` is dead
  — this document's own conclusion, now executed
  (`a_redirect_charge_with_no_token_is_failed_without_ever_asking_the_rail`).
  The branch is on `Capabilities::flow`, a capability *value*, never a rail code
  (ADR-0002).
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

- **No `SIGKILL` test.** See the Tests section above: the states are written,
  not caused. A real kill-and-restart test would additionally prove the
  process's behaviour under a signal, and nothing does.
- **No callback route**, so a rail that tries to tell us about a charge is
  ignored and only the ladder finds out — see [reconciler.md](reconciler.md).
- **The rails are WireMock hosts.** Every recovery case above is proven against
  a stub speaking the documented protocol. No real rail has ever been called,
  so what is proven is that the recovery table is executed correctly, not that
  the rails behave as these documents claim.

See [../status.md](../status.md).
