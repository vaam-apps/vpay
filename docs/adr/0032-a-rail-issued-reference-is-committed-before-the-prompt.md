# ADR-0032: A rail-issued reference is committed before the prompt

- **Status:** Proposed — needs maintainer acceptance. The maintainer asked how
  to close Notch Pay's crash window ("#13: how do we do then?", 2026-10-23),
  the agent answered with the design below, and the maintainer replied "Fan
  out the work via sub-agents". That is an instruction to build it, not a
  review of this text: every decision below, D1–D6, was taken by the
  implementing agent and has not been put to the maintainer one by one.
- **Date:** 2026-10-23
- **Deciders:** the implementing agent (D1–D6, 2026-10-23), pending the vpay
  maintainer's acceptance.
- **Extends:** [ADR-0002](0002-provider-port.md), the port, by one optional
  method and one capability value. ADR-0002's body is not edited, and its
  rule — the core branches on capability values, never on a provider code —
  is the rule D1 follows.
- **Implementation:** the change that carries this ADR —
  `ProviderAdapter::prepare` and `Capabilities::submit_steps`
  (`vpay-provider`), the confirm path's `prepare_on_rail` (`vpay-api`), the
  worker's `prepare_again` (`vpay-worker`), `Charges::record_prepared`
  (`vpay-db`), migration `0052_provider-requests-prepare.sql`, and the
  `notchpay` adapter's split of initialize from process.
  [RFC-0012](../rfc/0012-notchpay-adapter.md) § 3 carries a dated amendment
  pointing here.
- **Number checked at branch time, not assumed:** `git ls-tree -r
origin/master docs/adr` at `be31abcd` returns `0001`…`0031`, and so do the
  three sibling branches of the 2026-10-23 fan-out (`wip/method-routing`,
  `wip/callback-alert`, `wip/fapshi-study`) as of this writing. A sibling
  that lands an ADR first takes `0032`; this one is renumbered on landing,
  not the other.

## Context

[crash-safety.md](../flows/crash-safety.md) states one invariant: **never let
a payer act on a transaction you cannot later name.** MTN meets it because
the name is ours (`X-Reference-Id`) and is committed before the request.
Orange meets it because the payer can act only through a URL vpay hands out
after committing the `pay_token`.

Notch Pay met it for neither reason. Its payment is addressed by **its own**
reference, `trx.…`, which exists only in the answer to call 1, initialize;
process and read take that reference alone, and `GET /payments?search=`
does not find a payment by ours
([2026-10-21](../status/verification/2026-10-21-notchpay-sandbox-run.md)).
The one-call `submit` sent initialize, then process — the USSD prompt — and
returned the `trx.…` for the core to commit afterwards. A crash after process
and before that commit left a payer holding a prompt for a payment vpay
could never read. The retry met `409` "Reference already existing", and the
best the adapter could say was `NeedsReconciliation`: an operator reconciles
it in Notch Pay's dashboard (the maintainer's decision, 2026-10-21; escalated
at once since vaam-apps/vpay#313). RFC-0012 § 10 item 19 recorded it as a
residual risk.

That window is not specific to Notch Pay. Any push rail that issues the
reference the payment is addressed by, and accepts the prompt as a separate
call, has it.

## Decisions

### D1 — The port gains an optional `prepare`, declared by a capability

`Capabilities::submit_steps` is `One` (every rail before this one, and the
default for any new adapter) or `PrepareThenSubmit`.
`ProviderAdapter::prepare(charge, config, prior) -> Result<RefExtra, _>`
creates the payment on the rail **without prompting anyone** and returns the
key material the rail will be addressed by. Its default is `Unsupported`,
and the core never calls it on a `One` rail. On a two-step rail, `submit`
is the prompt, by the key material the charge already holds.

The core branches on the capability value, never on `notchpay`.

### D2 — The core commits `prepare`'s answer before it writes the `submit` row

On a two-step rail the confirm path (`vpay-api`) and the resubmit
(`vpay-worker`) each:

1. write a `prepare` row to `provider_requests`, unanswered;
2. call `prepare`;
3. commit what it returned on the charge (`Charges::record_prepared`, which
   writes `provider_ref_extra` only while the charge is `submitting`), then
   answer the row;
4. write the `submit` row, and only then call `submit`.

A crash anywhere in 1–3 leaves no `submit` row. A crash after 4 leaves a
charge that holds the reference the read needs.

The `prepare` row carries the same attempt number as the `submit` it
precedes: the number counts submissions, and a retried `prepare` with no
submit between keeps it.

### D3 — No `submit` row is the proof that nobody was prompted

Every build, before and after this one, writes a `submit` row before every
submit call; this build additionally writes it only after the reference is
committed. So the core can tell the adapter, from evidence it already keeps,
whether any call ever asked the rail to prompt this charge's payer:
`PriorSubmits::None` when there is no `submit` row, `PriorSubmits::Some`
when there is one. No new column and no marker: the absence of a row
`recovery_step` already reads.

### D4 — A `409` with no submit behind it fails the charge cleanly

On Notch Pay, `prepare`'s `409` means an earlier initialize created the
payment and lost its answer. With `PriorSubmits::None` nothing was prompted,
so the charge is failed — `Rejected { provider_unavailable }`, through the
same declined path a refused submit takes (charge `failed`, intent back to
`requires_payment_method` with the reason, one
`payment_intent.payment_failed`), and the payer can try again on a new
charge. With `PriorSubmits::Some` it stays `NeedsReconciliation`, escalated
at once as vaam-apps/vpay#313 built.

### D5 — Charges in flight across the deploy keep the old answer

A charge the one-call build left with an unanswered `submit` row and no
`trx.…` may have been prompted. Both paths that reach it keep it with an
operator: its poll asks the rail, the adapter cannot read a charge with no
`trx.…`, and the answer is `NeedsReconciliation`; its resubmit, if one is
queued, calls `prepare` with `PriorSubmits::Some`, and the `409` is
`NeedsReconciliation`. Neither path prompts it, fails it, or initializes it
again. A one-call charge with no attempt row at all was never sent, under
either build, and is simply prepared and prompted.

The discriminator is durable because it is the `provider_requests` table,
which outlives every process; no flag in memory and no configuration says
which build wrote a row.

### D6 — The orphan expires, and its exposure is ADR-0031's

A crash between `prepare` and the commit leaves a payment initialized on
Notch Pay and never processed. vpay fails its charge and never touches it
again; Notch Pay expires it ("Transaction has expired after 3 hours").
Until then, anyone holding the merchant's **public** key could process it to
a number of their choosing. That is the exposure
[ADR-0031](0031-a-status-read-authenticated-by-a-server-side-key.md)
accepted for every payment on the account — whoever holds the public key can
initialize, process and cancel payments — and an orphan adds nothing a
fresh initialize under the same key would not.

## Consequences

- **The window is closed for prompts.** The one state that let a payer act on
  a payment vpay could not name — prompted, `trx.…` lost — can no longer be
  produced. What a crash leaves is one of: nobody prompted and nothing held
  (the charge fails, D4); nobody prompted and the reference held (the
  resubmit prompts by it); possibly prompted and the reference held (the
  read decides, as on every rail).
- **One more request per charge is recorded, not sent.** Notch Pay always
  took two calls; the change is a second `provider_requests` row and a
  commit between them. MTN and Orange are `One`: no extra row, no extra
  call, and their paths are unchanged.
- **A lost process answer is resolved by the read.** A resubmit of a charge
  that holds its `trx.…` and has an unanswered `submit` row is not sent:
  the table's middle row polls. If the first process never reached Notch
  Pay, the read finds `pending`, and the charge waits for Notch Pay's own
  expiry and fails `payer_timeout` — a three-hour wait for one payer, and no
  money. Re-sending process from the read is refused, as RFC-0012 § 3 refused
  it: a prompt minutes later reaches a payer who has left.
- **A migration.** `provider_requests.operation` admits `prepare`
  (`0052_provider-requests-prepare.sql`); the column's CHECK is reopened, not
  dropped.
- **The adapter's `submit` refuses a charge with no reference** as
  `NeedsReconciliation`, before any request. The core never makes one; one
  that arrives has lost the only address Notch Pay answers for.
- **Not proven against Notch Pay.** Every case runs against the WireMock
  stub. No sandbox run has exercised the split, so whether Notch Pay keeps a
  never-processed payment `pending` until its expiry, and whether its `409`
  arrives as the stub answers it, are the sandbox's to confirm.

## Alternatives considered

- **Keep the one-call submit and escalate (vaam-apps/vpay#313).** That is
  where this started: correct, but every crash in the window is an operator's
  task and a payer who may have paid a payment vpay cannot show them.
- **Find the payment by our reference.** Not offered: `search` matches Notch
  Pay's reference only, and list items carry no `trxref` (2026-10-21).
- **Persist from inside the adapter.** An adapter that writes to the
  database is a core concern in a rail crate, against ADR-0002.
- **A per-charge "prepared" flag.** Redundant with D3: the `submit` row's
  absence is already durable, already written by every build, and already
  read by recovery.
- **Process from inside the read when it finds `pending`.** Refused in
  RFC-0012 § 3 and still refused: a prompt sent by a poll minutes later
  reaches a payer who has left.
