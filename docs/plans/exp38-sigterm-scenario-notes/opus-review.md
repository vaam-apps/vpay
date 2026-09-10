# exp38 — the sabotage review of the SIGTERM scenario (issue #85)

Written 2026-09-10 on branch `claude/exp38-sigterm-scenario`, rebased onto
`master` at `ded879d` (which carries #95 and #97; the branch's original base
`ff1f507` predates #97). Reviewer tier `opus`. The implementation notes it
reviews are [opus.md](opus.md).

The verdict first: **the third scenario was safe as delivered — it proves what
it says, and both of its decisive mutations were reproduced here — and it was
not complete.** Five findings, three of them fixed by writing more test rather
than by changing anything the delivered case asserts. Nothing was weakened.

## What was re-run, not taken on trust

Every number the implementation notes claim was re-measured on the rebased
head. Two of them came out differently, and both differences are the rebase's
(`master` moved from 1626 tests to 1648).

| Claim in `opus.md` | Re-measured here |
|---|---|
| ten consecutive runs of the new case, 30.4–99.8 s, 10 passed | **10 passed**, 30.1–53.3 s, on `e3a2782` |
| remove the drain → *the worker never logged `webhook delivered`* | reproduced, byte for byte |
| remove `locked_at IS NULL` from `Jobs::claim` → double send | reproduced: `left: 2  right: 1`, two byte-identical signed POSTs of one `evt_…`, both carrying the same `t=` |
| `just ci` 1626/1626 | **1650/1650** on the review's final head (`master` moved to 1648, and this review adds two cases); the entry in `docs/status.md` was written against the pre-rebase count and is corrected |

## The attacks, and what each proved

| # | Attack | Result |
|---|---|---|
| A1 | `slow-ack.json`'s delay to **11 s**, over `WEBHOOK_REQUEST_TIMEOUT` (10 s), constants untouched | **FAIL** in 20 s: *the worker never logged `webhook delivered`*. It does not pass by another path — but the message reads as a drain regression and is not one. **Finding F1** |
| A2a | `SHUTDOWN_GRACE_SECONDS` to **5 s**, under the 6 s delay | **BUILD FAILURE**, `error[E0080]: evaluation panicked: the drain must outlast the receiver, or the exit code under test changes`. The `const` assertion is real and fires at compile time exactly as claimed |
| A2b | the same, with that `const` assertion deleted | exit **1**, `released=1`, `the shutdown grace period elapsed …`; the case fails `left: Some(1)  right: Some(0)`. This is the `Drain::TimedOut` the notes list as *not covered* — and it is reachable, deterministically. **Finding F2** |
| A3 | every `request` block of every mapping in the receiver tree, read for overlap with `/slow-ack` | none. `any-post-200.json` is `urlPattern: ".*"` at priority **10**, `flaky-500-then-200.json` is `/flaky` at 1, `slow-ack.json` is `/slow-ack` at 1. `tests/webhooks.rs` delivers to `/webhooks` and `/flaky`, and each test starts its own receiver container. The claim that the new mapping cannot affect the other two scenarios or `webhooks.rs` holds — **but nothing enforced it**. **Finding F1** |
| A4 | the tree and the scratchpad, read for a pattern kill after the implementer's disclosed `pkill -f "just ci"` | nothing. `git grep -E "pkill|killall"` over the worktree is empty; the only scratchpad hits are two busybox applet lists from an unrelated experiment. The one signal this suite sends is `kill -TERM <pid>` on a pid it spawned |

## The findings

### F1 — `RECEIVER_ACK_DELAY` was pinned to the shipping budgets and to nothing else (gate-hole)

The two `const` assertions compare the **constant** against
`WEBHOOK_REQUEST_TIMEOUT` and `--shutdown-grace-seconds`. The constant is
transcribed by hand from `slow-ack.json`, and its own doc comment says so.
Nothing checked the transcription, and nothing checked that the mapping keeps
`/slow-ack` to itself.

Fixed by `the_slow_receiver_mapping_is_the_delay_both_sigterm_cases_are_built_on`,
a container-free scan in the same file and the same style as the MSISDN guard
already there. It fires in **6 ms** on a mapping that would otherwise cost a
20-second container run to discover — measured, both directions.

### F2 — `Drain::TimedOut` under a real signal was disclosed as a gap and was one line away (correctness)

The notes are accurate: no signalled shipping process reached that branch in
any suite, and `worker_e2e.rs` proves the function rather than the binary. A2b
showed the branch is reachable from this staging deterministically, so the gap
is now
`a_drain_that_runs_out_of_grace_under_a_real_signal_exits_1_and_hands_the_lease_back`:
the same case with `--shutdown-grace-seconds 2` against the 6 s
acknowledgement, and both workers signalled at once.

It closes the notes' *second* disclosed gap in the same run. A restart after a
**clean** drain is vacuous — there is nothing left to claim — which is why the
implementer was right not to stage one; after a **timed-out** drain there is,
so the restarted worker is the only thing that can finish the delivery, and
the case waits for it to.

It also writes down something no case had: **a timed-out drain is
at-least-once at the receiver.** The merchant gets two byte-identical signed
POSTs of one event, and `webhook_deliveries.attempt` is still `0` afterwards,
because an aborted task records no failure — the only trace of the first
attempt is `jobs.attempts`. Both numbers are asserted so the gap is written
down rather than discovered by an operator.

Ten consecutive runs, 10 passed, 17.8–23.6 s. Two mutations: skip
`release_all` on the timed-out path → fails on `released=0`; make that arm
exit `0` → fails `left: Some(0)  right: Some(1)`.

### F3 — "the receiver accepted the POST" was the wrong verb, in the place it matters most (misleading-claim)

The double-send assertion's message, the module header and
`crash-safety.md` all said the receiver had **accepted** the in-flight POST
before the signal. It had not. WireMock journals a request when it *matches*
it and answers `slow-ack.json`'s 200 six seconds later, so at the moment of
the signal the merchant holds the bytes and no acknowledgement — which is
exactly why a cut-off send is a *duplicate* rather than a nothing. The
distinction is the case's whole subject. Corrected to "received", in all
three.

### F4 — a doc comment named a test that does not exist (nit)

`SIGTERM_MSISDN`'s comment pointed at
`an_ordinary_msisdn_arms_no_mapping_in_the_shared_rail_tree`; the guard is
`the_sigterm_scenario_confirms_with_an_msisdn_that_arms_no_rail_mapping`.
Now an intra-doc link, so rustdoc would object to the next such drift. The
guard is renamed to the plural by the fourth scenario, which shares it.

### F5 — the in-flight probe was about to be copied (nit, ADR-0016 §4)

Three witnesses have to line up before either case may signal anything: a POST
in the receiver's journal, exactly one `pending` delivery row, and a lease on
its job. The fourth scenario needs the same three, and a second copy of a
condition this subtle is how two cases come to test two different things.
Extracted as `delivery_in_flight`; the failure message is unchanged and still
prints both transcripts.

## The co-running worker, read off its own transcript

The brief asked for proof that worker B did not take the delivery while A held
it, and that B was idle rather than racing. The transcripts say:

- **A claimed everything.** `job loop stopped … claimed=8`, and the delivery
  is in the middle of that.
- **B logged nothing at all after `job loop running`.** It never won a claim,
  for anything, in the whole run.

So B's silence proves it did not take the delivery — and does **not** prove it
ever asked for one while the lease was held. Nothing in the shipping loop logs
an empty claim, so no assertion can witness that directly. What witnesses it is
the second mutation: with `locked_at IS NULL` removed, B claims the leased
delivery and sends the merchant a second identical POST within the same second.
**The lease property is non-vacuous, and the mutation is the proof, not the
assertion.** That is worth saying plainly, because the case reads as though the
assertion were the proof.

The restart the brief also asked about: after A exits 0, nothing remains for a
fresh worker — the delivery row is `succeeded` and its job is deleted. A
restart there could only prove that a worker with nothing to do does nothing.
That is why the restart lives in the timed-out case instead.

## A signal before the claim, and why it is not a fifth case

Signalling a worker that has not claimed anything is exactly
`stop_worker_cleanly`, whose own assertion string says "a worker with nothing
in flight" and which every scenario in the file already runs at teardown —
including both SIGTERM cases, on a worker that has just watched a delivery
happen. The other half ("the co-runner delivers it once") is the ordinary
delivery path, covered by `tests/webhooks.rs` and `worker_e2e.rs`. A fifth
container-backed scenario would buy a third copy of two covered facts and
another ~30 s of gate.

## One flake, and what it actually was

The clean case failed once in this review's runs, at *"no webhook delivery was
in flight within 50s … the receiver saw 0 POSTs"*. It was not a defect in the
case: the victim's own transcript in the failure message carries
`WARN sqlx::query: slow statement: execution time exceeded alert threshold
summary="UPDATE jobs SET run_at …"`, on a host at load average 15 running two
other agents' suites. The failure is loud, self-describing and carries both
transcripts — it took about a minute to diagnose from the message alone, which
is the property that matters. `DELIVERY_IN_FLIGHT_TIMEOUT` is left at 50 s:
raising it to survive a thrashing host trades a clear failure for a longer
hang, and CI runs on a dedicated VM.

A one-worker staging of the *timed-out* case was tried first and abandoned:
it failed to reach the in-flight state within the same bound on four attempts
out of six on this host, with the settlement poll waiting tens of seconds
behind the singleton jobs the single `--worker-concurrency 1` task also has to
run. The mechanism was not chased down — it is a property of the shipping loop
and not of this case — and the two-worker staging, which is the delivered
case's own, does not have it: 10 runs, 10 passed.

## The review's own case failed the gate once, and why that is in here

The first full `just ci` on the head carrying the fourth scenario failed it —
*"a succeeded delivery's job must be deleted"* — after ten green runs of the
case on its own. The race is the review's, not the shipping code's:
`vpay_worker::webhooks` records the receiver's answer and then finishes the
job, two statements, so a delivery row reads `succeeded` a moment before its
job disappears. Standalone the moment never landed; inside a full suite,
sharing a host with 1649 other tests, it did.

Fixed by bounding that one read (`JOB_DELETION_TIMEOUT`, 2 s) rather than
deleting it: the assertion is unchanged and a job genuinely left behind still
fails the case. The clean case reads the same fact without a wait and is
correct to, because there the process holding the job has already exited,
which is strictly later than the delete.

It is recorded here because it is the exact failure this review exists to
catch, and it was caught by running the project's own gate rather than by
reading the code.

## The gate, on the head this review ends at

`just ci`, exit code read from a file: **exit 0** on `ccc97ba` (the code head)
and **exit 0** again on `fa1d257` (the head the status entry is in), Node
22.23.2, rustc 1.98.0, cratestack 0.12.0. `test-rust` **1650 run, 1650
passed, 0 skipped** across 45 binaries; `test-doc` 111 passed, 1 ignored;
`verify` all twelve (`verify-links` 1039 links in 195 files, `verify-errors`
19 types, `verify-serde` 85 types, `verify-migrations` 38 files);
`verify-ignored` 0 ignored (expected 0), 45 binaries (expected 45), 1650
total; `lint-web`, `test-web`, `deny` all clean. Identical counts on both
heads.

The first of those took three attempts. The first found the race above; the second and third
failed a `vpay-db` test on `failed to create a container: Timeout error` and
`container startup timeout`, on a host carrying two other agents' full suites
at load average 10. Neither is anything this branch touches, and the run
recorded here is a complete one.

## What this review did NOT do

- **No shipping code was changed.** Every mutation above was applied to
  `run_loop.rs`, `jobs.rs` or `worker.rs`, measured, and reverted; `git status`
  was checked clean after each.
- **`SIGINT` is still signalled by nothing**, though `vpay_config::signal`
  handles it identically.
- **Orange Money is still untouched** by all four scenarios.
- **The one-worker starvation above was not diagnosed**, only measured and
  avoided.
- **`cargo xtask verify-citations` was not run** — it needs the network and a
  GitHub token, and is not part of `just ci`.
