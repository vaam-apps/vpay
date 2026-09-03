# `vpay-worker` reference

Why the code in `backends/crates/vpay-worker` looks the way it does. The
crate's own doc comments say *what* each item is and link here; this page
carries the reasoning, the orderings and the history that a reader needs once —
not on every `cargo doc` build.

Tier: an [ADR](../adr/) records a decision, a [flow](../flows/) describes a
process, and a reference page like this one explains why a particular piece of
code is shaped the way it is. The processes this page supports are
[reconciler.md](../flows/reconciler.md),
[crash-safety.md](../flows/crash-safety.md) and
[webhooks.md](../flows/webhooks.md); what follows is the code's side of them.
[../status.md](../status.md) is the record of what is actually wired up — this
crate holds *handlers*, and a handler no loop calls is not a running worker.

- [Reading order](#reading-order)
- [The job loop](#the-job-loop)
  - [Why the loop owns the row and the handler does not](#why-the-loop-owns-the-row-and-the-handler-does-not)
  - [Why `run_loop` and `run_once` are public](#why-run_loop-and-run_once-are-public)
  - [The drain](#the-drain)
  - [Two lease reapers, on purpose](#two-lease-reapers-on-purpose)
  - [Where a job failure is logged, and how often](#where-a-job-failure-is-logged-and-how-often)
- [One poll](#one-poll)
  - [The horizon is evaluated above the crash-recovery block](#the-horizon-is-evaluated-above-the-crash-recovery-block)
  - [The one place a composite replaces a leaf's classification](#the-one-place-a-composite-replaces-a-leafs-classification)
  - [Resubmit and escalate is a real non-determinism](#resubmit-and-escalate-is-a-real-non-determinism)
- [Recovering a `submitting` charge](#recovering-a-submitting-charge)
- [The outbox drain](#the-outbox-drain)
- [Delivering one webhook](#delivering-one-webhook)
  - [What is not checked here](#what-is-not-checked-here)
  - [Two retry ladders, and why they do not share](#two-retry-ladders-and-why-they-do-not-share)
- [Signing](#signing)
- [How this crate is tested](#how-this-crate-is-tested)

---

## Reading order

`run_loop` owns the `jobs` row (claim, settle, drain); `handlers::handle` does
the work of one job; `recovery`, `vpay_core::settlement` and `webhooks` are the
decision tables it consults; `error` is the retry policy all of it derives from
`Classify` (ADR-0011).

Everything that touches the network happens in this crate, never in the API
process.

## The job loop

`run_once` is the whole claim/settle protocol:

1. `vpay_db::Jobs::claim` — one row, `FOR UPDATE SKIP LOCKED`, `attempts`
   incremented by the claim itself so a job that kills its worker still counts
   up;
2. `handlers::handle` — the work, which may call a rail and may commit;
3. exactly one of `finish` / `reschedule` / `dead_letter`, chosen from the
   `Outcome` or from `JobError::decision`, each guarded on this worker's
   `locked_by`.

There is no step between 2 and 3. A job whose handler committed a settlement
and then failed to write to `jobs` is re-run, and every handler is a
compare-and-swap for that reason — the second run matches no rows and answers
`Outcome::Done`.

### Why the loop owns the row and the handler does not

`handlers::handle` returns an `Outcome` instead of writing to `jobs` itself. A
handler that could delete its own row could delete one it had not finished, and
a handler that could park a row could dead-letter a charge by returning `Ok`.
Everything that ends a lease is in `run_loop.rs`, and every one of those writes
is guarded on `locked_by`, so a worker whose lease was reaped mid-run discards
its answer instead of stamping it over whoever holds the job now — that is what
`Disposition::Lost` counts, and any non-zero value of it in a `LoopReport` is a
lease shorter than a handler.

The retry policy is not decided here either. It is `JobError::decision`, derived
from `Classify`, so the worker and the API cannot disagree about whether a
Postgres failure is transient (ADR-0011). Not the poll ladder — that is
`poll_delay`. Not what a rail's answer means — that is `vpay_core::settle`.
This module maps a `Decision` onto one of three writes and counts how often
each happened.

### Why `run_loop` and `run_once` are public

`vpay-worker-bin` calls `run_loop` and so does
`backends/tests/integration/tests/worker_e2e.rs`. There is no second
implementation, no `#[cfg(test)]` variant and no injected clock: the integration
suite drives *this* loop, against a real Postgres and a real WireMock rail,
which is the only way a claim/settle protocol can be proven at all — `SKIP
LOCKED`, the `locked_by` guard and the drain are properties of Postgres and of
concurrency, not of Rust types.

`run_once` is public for the same reason and is **not** a seam: it is the
loop's own body, called by `run_loop` N times per task. A test that wants to
observe exactly one job's disposition calls it directly rather than racing a
background loop and scraping logs for the answer.

### The drain

On shutdown the tasks stop *claiming*; each finishes the job it is on. That is
the whole of a clean drain, and it is why `LoopReport::released` is zero on one:
a task only re-checks shutdown between jobs, so a claimed job is always settled
before its task exits, and there is no lease left to hand back.

When the grace period elapses first the remaining tasks are aborted and
`vpay_db::Jobs::release_all` clears every lease this worker still holds. Without
that call those rows stay leased until a reaper frees them — at best half a
lease, and only if a worker is running at all — of a live charge going undriven
for no reason other than that a pod was rolled. The release is safe against the
aborted tasks rather than racing them: every write that ends a lease is guarded
on `locked_by`, so an abort that lands mid-flight either committed before the
release (and its guarded write matched) or after it (and matched nothing,
leaving the job for another worker to re-run — which every handler is a
compare-and-swap to make safe).

**The grace clock starts when the shutdown signal arrives, not at boot.**
Waiting for the signal before racing the drain against `grace` is the whole
difference between a bounded drain and a worker that aborts every in-flight job
`grace` seconds after it started, forever. The shape mirrors `vpay-server`'s
`grace_clock`, which waits for draining to have *begun* before it sleeps.

`run_loop` returns a `LoopReport` and not a `Result`, because after the seed
there is nothing left whose failure should stop a worker. A claim that fails is
logged and retried after `IDLE_SLEEP` — Postgres being briefly unreachable is
the case the retry exists for, and exiting the process instead would turn a blip
into a restart loop. A seed that fails is logged with `alert` and the loop
starts anyway: the deployment loses its sweeps, which matters, but the charges
it is driving matter more.

### Two lease reapers, on purpose

`run_loop` reaps **before** it seeds, unconditionally, and then again on its own
timer at half a lease. The hourly `sweep_expired` job also reaps. That is not
redundancy: a worker that was SIGKILLed leaves every job it held with
`locked_at` set, `Jobs::claim` matches only `locked_at IS NULL`, and if the dead
worker was holding `sweep:expired` then the job that used to be the only reaper
is itself among the stranded leases — a deadlock the queue cannot leave on its
own. A job cannot recover the lease on itself.

Half a lease is the period, floored at `IDLE_SLEEP`: the reaper can only free a
lease already older than `lease`, so a period of `lease` would put the worst
case at two. Anything finer buys nothing — the thing being recovered from is a
dead process, and the charge behind it is on a ladder whose fastest rung is ten
seconds. The floor exists so that a deployment (or a test) with a very short
lease does not turn the reaper into a hot loop against Postgres.

### Where a job failure is logged, and how often

Twice, deliberately, and the two lines say different things.

`handlers::log_failure` reports the *error* at `Classify::severity`, sets
`alert = true` for `Severity::Page` and nothing else, and is where a job failure
reaches `vpay_error_events_total` and `vpay_alert_events_total` through
`vpay_core::metrics::record_error_event`. It is the frame that still knows which
job and which charge failed.

`run_loop::log_disposition` reports the *disposition* and carries `alert` from
`Decision::RetryAfter`, which fires at `Severity::Error` and above — a wider net,
and the one the 24-hour `unresolved` escalation needs (`JobError::Exhausted` is
`Severity::Error`, not `Page`, so the handler's line does not flag it and this
one must). It deliberately does **not** increment the counters: it would
double-count the first line and add others that carry no classification to
label.

So a `Page`-severity failure produces two events carrying `alert = true` — "the
rail refused our credentials" and "so this job is parked forever". An alerting
rule that deduplicates on `job_id` sees one incident either way.

Both are written as four `match` arms rather than a `Level` variable because
`alert` is an event *field* and has to be written at the macro call site; see
`error::tracing_level`.

The queue's own failures — a `claim` that could not run — are counted in
`claim_loop` as well as logged. That `DbError` never reaches `log_failure`, it
is not a job's failure, so without that call a queue nobody can read would page
in the JSON logs and be invisible to `VpayPageableErrorEvents`.

## One poll

`handlers::poll_charge` is six steps: the terminal guard, the horizon, the
crash-recovery block, the rail, `vpay_core::settle`'s answer, and what to do
with it. Two of the orderings are load-bearing.

**The attempt row is written before the call and answered after it.** That is
`docs/flows/crash-safety.md`'s requirement, and it is the single fact the
recovery table branches on: an attempt that got no answer
(`provider_requests.status_code IS NULL`) is distinguishable from one that was
never made.

### The horizon is evaluated above the crash-recovery block

`past_the_horizon` is a property of the charge and the clock alone, so it is
evaluated once and carried down to the two frames that can conclude "this charge
is still live and still unanswered" — `keep_polling` and
`resubmit_then_escalate_if_late`. Two evaluations of a predicate over a clock
that has moved between them can disagree, and this one decides whether a human
is told about a charge.

It does **not** decide whether to ask the rail. Past the horizon the charge is
still polled — hourly rather than on the ladder — because "a late success —
minute 40, or hour 30 from `unresolved` — is the normal transition"
([reconciler.md](../flows/reconciler.md)), and a poll that stopped asking would
be the thing that lost it. What the horizon decides is what happens to every
answer *short of* a terminal one: escalate, instead of taking another rung.

Its placement *above* the crash-recovery block is the part that is easy to get
wrong. That block can return without ever reaching the horizon line — and one of
its arms, `Resubmit`, returns a rung of the ladder. A `submitting` charge whose
resubmit job is dead-lettered comes back to it on every poll
(`SubmitAttempt::Never` → `Resubmit`, forever), so a horizon evaluated
afterwards would never be evaluated at all for exactly the charge that has been
stuck longest.

Escalation is measured from `charges.created_at`, which is written *before* the
rail is called by construction — so it is the age of the payer's exposure and
not of our bookkeeping.

### The one place a composite replaces a leaf's classification

When the rail will not answer a charge that is already past the horizon,
`rail_did_not_answer` logs the rail's error and escalates with
`JobError::Exhausted` instead of returning it. ADR-0011 permits it here for one
reason: `Exhausted` says something *truer* about a rail that will not answer a
day-old charge than the rail's own transient error does. Nothing else the
composite wraps is like that.

The guard is `JobError::Provider` and nothing else, and that is a fix rather
than a style choice. Written as a wildcard, the arm swallowed `Poisoned` — a row
this build cannot interpret, `Retry::Never`, a bug — and re-published it as
`Category::Rail`, retried hourly with an alert, forever, on work no retry can
complete; and it swallowed `Db`, whose own retry policy exists so the worker and
the API cannot disagree about whether Postgres is transient. Both now propagate
untouched.

Without the escalation at all, `ProviderError::Unavailable` is only
`Severity::Warn`, so a status endpoint answering `503` on every rung rode the
ladder quietly past the horizon forever: no `unresolved`, no alert, nobody
reconciling a charge a payer may have paid.

`escalate_to_unresolved` never returns `Ok`. Its `Result<Outcome, _>` return
type exists so a caller can `return escalate_to_unresolved(…).await;` from a
function that otherwise produces outcomes, and so the state write can propagate
a real `DbError` with `?` instead of being swallowed into the exhaustion — a
database failure here is a database failure, not an escalation. The write is
skipped when the charge is already `unresolved`, which is what makes the hourly
re-escalation idempotent: the alert repeats, the row does not move, and
`charges.updated_at` keeps naming the last time anything actually changed.
`a_second_hourly_poll_of_an_unresolved_charge_re_alerts_without_writing_it_again`
asserts the timestamp *and* the alert — a no-op that also stopped alerting would
satisfy half of that sentence.

### Resubmit and escalate is a real non-determinism

Both `Resubmit` arms — the one in the crash-recovery block and the one in
`recover` — go through `resubmit_then_escalate_if_late`, because they are the
same decision reached from different evidence and only one of them used to know
about the horizon. Neither did the right thing at 24 hours: a `submitting`
charge whose rail answers `404` forever cycled resubmit → ladder → resubmit,
never `unresolved` and never alerting, which is the one outcome
[reconciler.md](../flows/reconciler.md) rules out for a charge that has been
live for a day.

The resubmit row is committed first and the escalation second, in two
transactions, and the escalation moves the charge to `unresolved`. So the
resubmit job usually finds the charge outside `submitting` and returns
`Outcome::Done` without calling the rail: past the horizon the escalation
ordinarily *supersedes* the resubmit rather than running alongside it. A
concurrent worker that claims the resubmit between the two commits does send it,
under the charge's existing reference. Both orders are safe — the reference
never changes, and `escalate_to_unresolved` is idempotent — but this is a real
non-determinism and not a detail: what the horizon guarantees is the alert and
the hourly poll, not that a 25-hour-old charge is resent. That is deliberate.
Once a human is reconciling a charge against the rail's settlement statement,
whether to push another submission at it is their call, not a queue's.

## Recovering a `submitting` charge

`recovery::recovery_step` is
[crash-safety.md](../flows/crash-safety.md)'s "Recovering a `submitting`
charge" table as one pure function. The whole of it is a disambiguation:
`submitting` covers two physically different situations — "we crashed before the
POST" and "the POST went out and the answer was lost" — and `provider_requests`
is the only evidence that tells them apart.

**The flow shape decides first.** On a redirect rail the payer cannot act until
they are handed a URL, and the URL is handed over only *after* the rail's key
material is committed ("the commit is the gate on the redirect"). So a redirect
charge still in `submitting` is one nobody could have paid, and — because the
`pay_token` needed to ask the rail about it was in the response we lost — one
nobody can ever ask about either. That reference is dead: fail it and let the
merchant open a new PaymentIntent. Polling it instead produces
`ProviderError::Config` on every rung of the ladder forever, which is a dead
letter dressed up as an outage. The branch is on `ProviderFlow`, a capability
*value*, never on a rail code (ADR-0002).

**The precondition is that the charge is in `submitting`**, and it is
`RecoveryAction::FailDeadOrder` that makes it a precondition rather than a
preference: on a redirect rail that answer is correct only while the payer has
not been handed a URL. Once the charge is `submitted` the URL has been handed
over, the payer may have paid, and failing the charge would discard a live
payment. A `NotFound` on a `submitted` charge is therefore handled as an
ordinary pending answer by the caller (`vpay_core::settle` answers
`Settlement::Stay` for the states past this one).

**A count alone is not enough** for the "the rail never received it" conclusion.
Three polls can happen in under a second on the first rungs of the ladder, and a
rail that is merely slow to index a new charge would look identical to one that
never got it. `RecoveryPolicy::not_found_streak` and `not_found_window` are both
required, never either.

`RecoveryPolicy` is a plain struct with a `Default`, deliberately not a
`#[cfg(test)]` seam: AGENTS.md's first rule is that no test double may be
reachable from a shipping binary, and "the tests override the policy" is only
honest if the tests override the *same* value production uses. An integration
test asking for `not_found_window: 50 ms` exercises the identical code path a
deployment runs at 60 s, with no sleeps.

## The outbox drain

The process — the two transactions, the abandonment after five passes, the
alert-once property, the backstop and what it does not recover — is
[webhooks.md](../flows/webhooks.md), which states all of it in operator terms.
What belongs here is the code's own shape.

**One transaction per event, never one for the page.** A crash mid-page loses
nothing and duplicates nothing: the events already committed are `done`, the one
in flight rolls back whole, and the rest are still `pending`. The replay that
follows is absorbed by two database objects — the unique index
`webhook_deliveries_event_endpoint` means a re-run creates no second row, and
`jobs_dedupe_key` with `jobs::webhook_dedupe_key` means it enqueues no second
job — and both are absorbed *inside* that transaction together with
`mark_fanned_out_in_tx`. Nothing here compensates, retries by hand, or
reads-then-writes.

The closing compare-and-swap is what makes the whole transaction safe to
replay. `mark_fanned_out_in_tx` answering `false` means another drain claimed
this event while we were building ours, so everything above it was computed
against a backlog entry that is no longer ours to claim: the transaction is
abandoned (`TxOutcome::Abandon`), and that is not a failure.

**Each event is attempted independently**, and its failure is counted in a
*separate* statement — the transaction whose failure is being counted has rolled
back, so a counter inside it would roll back too and the event would be retried
forever at zero. A failure to *count* a failure is logged at `warn` and
swallowed: the pass has already decided to continue, and it must not become the
thing that stops the drain.

`FanOutDisposition` has three values rather than a `bool`, because the third one
is real: a concurrent drain can fan the event out between this pass failing on
it and this pass counting the failure, and that is neither a retry nor an
abandonment. Splitting the disposition out of the handler is what makes "an
alert happens only on the transition to `failed`, and a row that was no longer
`pending` produces no alert at all" pinnable without a database — and those two
properties together are what make "99 poisoned events cost 99 alerts, not 99 per
pass" true.

**The immediate reschedule is conditional on progress**, not only on a full
page: a page of 100 events that all fail is not a backlog draining at full
speed, it is a tight loop against Postgres.

**The backstop's page has no per-row failure mode to isolate**, unlike the
drain's, and that is why it shares one transaction across its page. The read is
one statement, and every enqueue writes the same three fixed shapes — a `kind`
from `JobKind`, a `dedupe_key` built from a `Uuid` (and `jobs.dedupe_key`
carries no length CHECK), and a two-field payload. There is no operator-authored
value in any of them, so there is no equivalent of the 65-character
`endpoint_id` that made one merchant's configuration a total outage for the
fan-out. If a row-shaped CHECK is ever added to `jobs`, that argument stops
holding and this pass needs the fan-out's per-row isolation.

`handle_scan_deliveries` wraps its own pass only so that every one of the pass's
`?`s reaches the `alert = true` line — an early return added later cannot bypass
it. The loop's own disposition line reports the *job*; that one reports what the
failure means: the backstop behind every webhook delivery is not running, and a
backstop nobody notices has stopped is a backstop that is not there.

The parked-key `warn` names at most twenty deliveries; the count is always
exact and only the list is cut. A pass can read 500 rows, and 500 UUIDs on one
line is a log record most collectors truncate in the middle — losing the count
as well as the tail. The runbook's query enumerates the rest.

## Delivering one webhook

The order is what makes an attempt auditable: the body is rendered and hashed
*before* the request, the digest is compared against the one the first attempt
that rendered and signed a body stored, and the outcome is written whether the
receiver answered or not.

`webhook_deliveries.payload_sha256` is written on the first attempt that
**rendered and signed** a body and `COALESCE`d thereafter. The body itself is
deliberately not stored: this is what makes "we sent exactly what we signed, on
every attempt" an observable invariant rather than a hope, at the cost of one
64-character column instead of a copy of every event body per endpoint. The
consequence for the code is the split between the two failure recorders —
`record_unsigned` stores no digest because nothing was signed, while
`record_no_response` stores one because the bytes *were* signed and only the
answer is missing. Stamping the column on an attempt that never left the process
would make every later mismatch check run against a body no receiver ever saw.

A transport failure is recorded with `status_code = NULL`, which is how the row
says "the request went out and nothing came back" — distinct from a heard
refusal, and the one distinction this row must never blur. Since Step 7 the
excerpt carries the `reqwest` error's whole `source()` chain, exactly as
`jobs.last_error` does (ADR-0011's amendment): without it the column repeated a
URL the operator already had and omitted "connection refused". `vpay-db`'s own
`bounded_excerpt` keeps it inside migration `0022`'s `excerpt_length` CHECK.

An endpoint configuration no longer describes, or describes without a secret, is
recorded as an ordinary failed attempt rather than exhausted on the spot: a
rollout that briefly serves an older configuration then heals, and a removal
that is permanent exhausts through the ladder anyway. Sending unsigned is not an
option — an unsigned webhook is one no receiver may act on.

The event is rendered through `vpay_api::model::EventObject`, which is also what
`GET /v1/events` serves. That sharing is the point and not an economy: a
merchant who missed a webhook is told to re-read the event from the API, and two
renderers would let the fallback answer a different question from the one the
webhook asked.

`Endpoint`'s and `EndpointRegistry`'s `Debug` redact every secret down to a
count. The registry is held by the job loop for the process's whole life, so it
lands in any `{:?}` of the loop's state; a webhook secret in a log is a forged
webhook, because anyone holding it can sign a `payment_intent.succeeded` the
merchant's handler will believe. The count is kept because "is this endpoint
mid-rotation?" is a real question a runbook asks, and answering it needs no
secret.

The registry is keyed on `events.merchant_id` and **not** on `client_id`, which
is the key `vpay_api::v1::ResourceConfig` uses: `merchant_id` is the fan-out
key, one merchant may hold several OAuth clients, and a registry keyed the other
way would silently fan out to the endpoints of whichever client happened to be
looked up. Within a merchant the endpoints are sorted by id and duplicates
dropped, keeping the first — defence in depth, not the guard (boot-time
validation refuses a duplicate `id`), and it matters only because the
alternative is worse: two endpoints sharing an id collide on
`webhook_deliveries_event_endpoint`, so exactly one of them would be delivered
to and *which one* would depend on iteration order.

### What is not checked here

**The URL.** Validation is boot-time (`vpay_config::validate_webhook_url` —
https-only under livemode, no stub markers) and there is **no runtime
private/link-local filtering**, so a livemode operator who configures
`https://169.254.169.254/…` gets exactly that. A resolve-then-connect check is
TOCTOU unless `reqwest` is given a custom connector, so the honest options were
"nothing" or "a custom connector", and the second is out of scope (decision 4 of
the Step 5 plan). [webhooks.md](../flows/webhooks.md) states the residual as
what it is: not SSRF protection.

**Redirects** are refused by the client, not by this code
(`vpay_provider::http`): a 3xx therefore arrives as an ordinary non-2xx and
becomes a failed attempt, instead of replaying a signed event body at a host the
operator never configured.

### Two retry ladders, and why they do not share

`delivery_delay`, not `JobError::decision`. Polling asks a *rail* what happened
to money; delivering tells a *merchant* what already happened. The two have no
failure vocabulary in common: a merchant's `500` is not a `ProviderError`,
nothing about it is classified by ADR-0011's table, and pushing it through the
poll ladder's decision table would give a webhook receiver the rail's 24-hour
horizon and its hourly `unresolved` escalation. So delivery keeps its own ladder
and never consults `Classify::retry`.

`delivery_delay` returns `Option` rather than a final rung because "the ladder
ran out" is the `exhausted` transition of a `webhook_deliveries` row and must
not be expressible as another delay: a `Duration` return would make the seventh
failure and the eighth indistinguishable at the type level, and a delivery that
keeps rescheduling forever is a queue that never drains.

The exhaustion is `Severity::Error` with `alert = true` and **not** a
`JobError`: a merchant will never be told about this transition by vpay, so a
human has to tell them — but nothing is broken here, the receiver is, and the
*job* did exactly what it was asked to. The row is the durable record
(`state = 'exhausted'`); the log line is what gets someone to look at it.

The budget constants (`WEBHOOK_CONNECT_TIMEOUT`, `WEBHOOK_REQUEST_TIMEOUT`) live
beside the handler that spends them rather than in the binary that builds the
client. They were once spelled in three places — the binary and the integration
suite's two helpers — and a change to the binary's pair would have left every
test exercising a client that no longer ships, with the suite staying green
saying so.

## Signing

`signing::signature_header` is the *sending* half of the scheme
[webhooks.md](../flows/webhooks.md) names; the two SDK verifiers
(`sdks/rust/src/webhooks.rs`, `sdks/nodejs/src/webhooks.ts`) are the
specification it is held to, and it is the only place in the workspace that
produces the header.

**The signed payload is `t_text || "." || body`, and `t_text` is the literal
text written into the header** — not a number re-rendered on either side. The
verifiers pin this deliberately
(`the_hmac_covers_the_literal_t_text_not_a_re_rendered_number`), because a
sender whose `t` does not round-trip through an integer would otherwise produce
genuine deliveries that every merchant silently rejects. The module writes the
decimal rendering of `now.unix_timestamp()` once and signs that same `String`,
so the two cannot diverge here even in principle.

**`body` is the bytes that go on the wire**, hence `&[u8]`: the caller renders
once, signs those bytes, and sends those bytes. A signature computed over a
re-serialisation of the same JSON is a signature over different bytes the moment
a key order or a float rendering differs.

**One `v1=` per configured secret, in configuration order.** Rotation is "add
the new secret, wait, remove the old one"; during the overlap a receiver holding
*either* secret verifies, because both verifiers try every `v1=` value. Emitting
only the newest would make rotation a flag-day.

Two things it deliberately does not do. **No constant-time anything**: signing
produces a value and never compares one, so there is nothing here for `subtle`
to protect — which is why this crate does not depend on it while the SDK does.
**No tolerance and no replay window**: those are the receiver's checks, and a
sender that tried to enforce them would only be describing its own clock.

Two edge cases are answers rather than defects. An empty `secrets` produces
`t=…` alone, which every verifier calls a *malformed header* — there is no such
thing as an unsigned delivery a receiver should accept, so that is exactly the
refusal an endpoint configured with no secret deserves (and `handle_deliver`
records a failed attempt instead of reaching here). A pre-epoch clock writes
`t=-…`, which fails both verifiers' `^\d+$` rule; clamping to zero would be
worse, because the delivery would then be signed with a timestamp that is not
the one the sender believes and would fail the *tolerance* check instead — the
same rejection, reported as something the merchant could plausibly debug.

`sign`'s error arm returns an empty string. `Hmac::new_from_slice` is infallible
for HMAC (it accepts a key of any length), but its signature is fallible for the
`KeyInit` trait's sake, `unwrap` is denied in this crate (ADR-0007), and an
empty signature cannot match any 32-byte HMAC and is dropped by both verifiers
as an empty `v1=` rather than treated as a candidate. Unreachable in practice;
the arm exists so the impossible case cannot become a panic in a payment path.

## How this crate is tested

The **handlers are not unit-tested**, and that is deliberate. Every one of them
is a sequence of writes against real Postgres and a real rail; the only way to
test one in-process would be to introduce a fake pool or a fake
`ProviderAdapter`, and AGENTS.md's first rule forbids a test double reachable
from a shipping binary (ADR-0006 — the stub rail and the stub receiver are hosts
in configuration, reached over HTTP exactly as the real ones are). So the proofs
live in `backends/tests/integration/tests/`, which drives *these functions*
against a Postgres container and a WireMock container, reproduces each
crash-safety kill point by writing the state a crash leaves behind, and reads
the deliveries back from the receiver's own `GET /__admin/requests`.

The **pure parts are unit-tested and doctested here**, which is why they are
separate modules at all: the two ladders, the recovery table, the settlement
table (in `vpay-core`), the payload encoding, the signature, the rendering and
digest, `EndpointRegistry`, `fan_out_disposition` and `past_the_horizon`.

`vpay-worker` has **no `sqlx` dependency and cannot grow one**. Every statement
lives behind a `vpay_db` repository trait reached through `&dyn Repositories`,
and where two writes must commit together the handler calls
`UnitOfWork::transaction` and writes through the `&mut dyn TxRepositories` the
closure receives — so no `sqlx` type is nameable: the pool never leaves
`vpay-db` and the open transaction is opaque. See
[vpay-db.md](vpay-db.md#the-repository-seam).
