# 2026-09-16 — a confirmed charge waited 61 s for its first poll, and the live refund suite is what found it

Branch `fix/live-refunds-e2e`, from `claude/orange-refund-transaction-2b95e5`
(`1e59e799`). CI on PR #178 was red in the `e2e (compose)` job, step
`cargo test -p vpay-sdk --features live-stack --test live_invoices --test live_refunds`:

- `live_invoices` — 2 passed in 0.11 s;
- `live_refunds` — **both** cases failed after exactly 60 s, both with
  `the intent was still` `processing` `after 60s: the vpay-worker container is
the most likely cause` (`sdks/rust/tests/live_refunds.rs`).

## The headline, first

**Nothing was wrong with the MTN stub, the scenario it rests in, the tenant,
or the worker's health.** A charge confirmed while the worker's claim loop is
busy had its first status query delayed by up to sixty-one seconds — on every
rail, for every merchant, in every deployment, since Step 4. The live suite's
sixty-second settlement ceiling is the only thing in this repository that has
ever been in a position to notice, and the first CI run that compiled that
binary noticed.

## The mechanism

1. `vpay_api`'s `insert_charge` commits the charge as `submitting` **and** its
   `poll_charge` job in one transaction, the job at `run_at = now()`.
2. The confirm then calls the rail, and only afterwards compare-and-swaps
   `submitting → submitted` (`persist_submitted`).
3. `vpay_worker::run_loop`'s claim tasks do **not** sleep between claims — only
   after an _empty_ one (`IDLE_SLEEP`, one second). A worker with any queue at
   all therefore claims that job within microseconds of its commit, which is
   inside the window at step 2.
4. `recovery_step` then does the correct thing and the only safe thing: a
   `submitting` charge younger than `RecoveryPolicy::not_found_window` is
   indistinguishable from a crashed confirm, so it answers
   `RecoveryAction::Wait` — which asks the rail **nothing** and reschedules the
   job for the rest of the window (`created_at + 60 s`, plus a one-second
   margin). It logs that at `DEBUG`, so at the container's `INFO` level the
   worker simply goes quiet.
5. The confirm's compare-and-swap lands milliseconds later. **Nothing pulls the
   job forward.** The charge is `submitted`, settleable, and unasked-about for
   another minute.

On an idle stack the worker's claim loop is asleep, the confirm wins, and the
payment settles in under a second — which is why every run against a fresh
stack passed and why CI, which runs the demo walk, both Cypress suites and the
stripe-compat suite first, did not.

## The evidence, in order

**1. CI's own artefact** (run of 2026-09-16 03:10–03:14 UTC, `compose.log`).
At `03:13:02.657` WireMock received the live suite's two `POST
/collection/v1_0/requesttopay` — both answered `202` by
`requestToPay accepted (…)`, the ordinary catch-all, so the stub was not the
problem. Between that instant and the log's end at `03:14:01` there is **no
`GET /collection/v1_0/requesttopay/…` at all**, no webhook delivery, and one
`job loop gauge` at `03:13:16` reporting `queue_behind_seconds: 0` — i.e. no
claimable job was overdue while two charges sat unpolled. The only branch of
`poll_charge` that returns without querying the rail is the `Wait` arm.

**2. The jobs table, during a reproduction.** The stack was brought up under
its own compose project and ports, taken through `just demo-walk` (leaving
`mtn-e2e-poll` resting in `e2e-settled`, as CI's earlier steps do), and the
suite run against it. A runtime-only WireMock mapping made the MTN submit take
five seconds — the confirm's own window, which a two-core CI runner widens by
being slow and this host does not. Every seven seconds:

    poll:ch_z6q7pk9kad3mvb9dn80bmcpf | attempts 1 | due_in 55s | submitted | age 6s
    poll:ch_z6q7pk9kad3mvb9dn80bmcpf | attempts 1 | due_in 48s | submitted | age 13s
    …
    poll:ch_z6q7pk9kad3mvb9dn80bmcpf | attempts 1 | due_in  5s | submitted | age 56s

`attempts = 1` — one claim, the one that decided to wait. `state = submitted`
from six seconds in. `run_at = created_at + 61 s` throughout. The suite:

    test live_refund_destination_refusals has been running for over 60 seconds
    test live_refund_lifecycle has been running for over 60 seconds
    test result: FAILED. 1 passed; 1 failed; finished in 61.33s

which is CI's failure, reproduced with its own message.

## The fix

Two writes in `vpay-api`, neither of them in the worker and neither of them a
change to the recovery table, which was right:

- `insert_charge` commits the poll job at `now() + POLL_AFTER_CONFIRM_GRACE`
  (`vpay_provider::DEFAULT_REQUEST_TIMEOUT`, twenty seconds, which no
  deployment can change) instead of `now()`. The row is still committed with
  the charge — crash safety is untouched — it is simply not claimable while
  the confirm that created it is still inside the rail call.
- `persist_submitted` calls `pull_forward_in_tx` in the **same transaction** as
  the `submitting → submitted` compare-and-swap. That swap is the exact moment
  "a confirm may still be holding this charge" stops being true, so it is the
  only moment the job can be made runnable without racing anything; and being
  in that transaction means a rolled-back submit cannot leave a job pulled
  forward for a charge still in `submitting`.

The grace covers the ordinary confirm; the pull-forward covers a confirm slower
than the grace (a rail token fetch and a submit are two budgets, not one) and
is what makes settlement independent of how busy the worker was.

`pull_forward_in_tx` is the primitive `provider_callback` already uses for a
rail callback. The floor is `Duration::ZERO` here rather than the callback's
ten seconds: that floor exists to stop an unauthenticated caller turning a
burst of callbacks into a burst of rail requests, and this call site runs once
per charge inside the confirm that created it.

### The same measurement, after

Identical command, identical stack, the five-second submit still installed:

    test result: ok. 2 passed; 0 failed; 0 ignored; finished in 5.69s

and the poll jobs are gone from the table by the first seven-second sample —
the charges settled as soon as the confirm let go of them, rather than 56 s
later.

### Mutation

| deleted                                         | what fails                                                        |
| ----------------------------------------------- | ----------------------------------------------------------------- |
| `persist_submitted`'s `pull_forward_in_tx`      | `a_push_confirm_the_rail_accepts_moves_the_intent_to_processing`  |
| the `+ POLL_AFTER_CONFIRM_GRACE` on the enqueue | `an_unreachable_rail_leaves_the_charge_where_recovery_expects_it` |

Both are new assertions on existing cases in
`backends/tests/integration/tests/confirm_rails.rs`, and each targets the write
above it: the first reads the poll job's `run_at` after a confirm the rail
accepted (it must be claimable now), the second after a confirm that never
reached the rail (it must be a grace away, because nothing pulled it forward).

Run with `--no-fail-fast`, so "exactly one" is measured and not assumed: each
mutation gives **13 tests run: 12 passed, 1 failed, 0 skipped**, and the one
that failed is the one named above. Unmutated, the same binary is 13 passed, 0
skipped.

## The second failure, which the first was hiding

With settlement fixed, `live_refund_lifecycle` failed one line further on:

    a pending refund cancels: Api { status: 409, code: "invalid_state",
      message: "This refund has already been given to the payment rail, so it
      cannot be canceled…" }

That is `4bcf6491` (2026-09-16 01:23, on this same branch) working as
designed. It added `cancel_in_tx`'s `NOT EXISTS (… provider_requests …
operation = 'refund')` after measuring, before the guard existed, **two 5 000
transfers on the rail's journal against one 5 000 charge**: a refund accepted
by MTN, cancelled with a `200`, its reservation released, and a second full
refund accepted. `POST /v1/refunds` writes that attempt row before it sends
the transfer, so no refund the route creates is cancelable any more — the
cancel route now serves the crashed-create state only.

Both live refund suites were written by an earlier arm against the old
contract and were never re-run against a server carrying the guard, because no
CI job compiled either of them until this branch added one
(`verify-sdk-parity` proves a test _name_ exists, never that anything runs it
— issue #122). Both now assert the refusal, and assert what the old cases only
implied:

- the `409`, and that its message does not echo the payee's number;
- the refund is still `pending` afterwards — a cancel that failed _after_
  releasing the reservation would be the same double refund with an error code
  on it;
- a refund naming no `amount` is now **3 000**, not 5 000, because the first
  refund's 2 000 is still reserved;
- and a further 1 is a `409`, so the 5 000 charge is exactly spoken for.

The server was not changed for this. The tests were wrong and the guard is
right.

## What was ruled out, and is worth writing down

The `mtn-e2e-poll` scenario resting in `e2e-settled` after the demo walk — the
first suspect, and the reason this arm existed — is **not** implicated. The
two confirms were answered by `requesttopay.json`'s priority-10 catch-all
`202`, and every status mapping in that tree answers `SUCCESSFUL` in that
state anyway; the rail was never the thing that did not answer. Giving the
live suite its own MSISDN, serialising its two cases, or resetting the
scenario in setup would each have left the defect in place, and would have
left it looking fixed.

## Gates run

| gate                                                                                        | result                                                               |
| ------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `cargo nextest run -p vpay-tests-integration` (the whole package, 22 binaries)              | **329 passed, 0 skipped** (1 075 s)                                  |
| `cargo nextest run -p vpay-api -p vpay-worker -p vpay-sdk`                                  | **638 passed, 0 skipped**                                            |
| `cargo test --doc -p vpay-api -p vpay-worker -p vpay-db`                                    | 18 + 8 + 5 passed, 0 ignored                                         |
| `cargo clippy --all-targets` on `vpay-api`, `vpay-worker`, `vpay-sdk --features live-stack` | clean                                                                |
| `cargo +nightly fmt --all --check`                                                          | clean                                                                |
| `just verify`                                                                               | ok — the twelve gates, plus the advisory `verify-docs` report        |
| the live suite, CI's own command, on a stack taken through `just demo-walk` first           | `live_invoices` **2 passed**, `live_refunds` **2 passed**, 0 ignored |
| `pnpm --filter @vaam-apps/vpay-sdk test:live`                                               | 2 files, **5 passed, 0 skipped**                                     |

`just ci` was **not** run: this arm was asked not to, and no claim here rests
on it. Every live row was measured against a stack under its own compose
project (`vpayfix`) and its own ports — never the `vpay-demo` stack on 8080.
