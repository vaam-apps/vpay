# `vpay-api` — the rail callback route (`provider_callback.rs`)

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## The rail callback route (`provider_callback.rs`)

`POST /provider/{code}/callback`, mounted since Step 8 lane C. Before it, both
adapters implemented `parse_callback`, nothing in a running vpay called
either, and the `X-Callback-Url`/`notif_url` every submit carried pointed at a
host that answered 404 — so settlement was polling-only and the poll ladder's
first rung is ten seconds (`vpay_worker::poll_delay(0)`).

### Why the only thing it may do is move a `run_at`

AGENTS.md: "Callbacks are hints. `parse_callback` returns identifiers only,
never a status. The authenticated status query is the only thing that moves
money." The port enforces the first half — `CallbackRef` has no status field
to put one in, and `parse_callback` is deliberately synchronous so an adapter
cannot fetch one either (ADR-0002, [provider-port.md](../../flows/provider-port.md)).
This route is the second half: it resolves the adapter, parses identifiers,
finds the charge, and then runs **two statements in one transaction** —
`enqueue_in_tx`, which is `ON CONFLICT DO NOTHING` and writes nothing in the
ordinary case, and `vpay_db::TxRepositories::pull_forward_in_tx`, an `UPDATE
jobs SET run_at = now()` on that charge's existing `poll:<charge id>` job.
(This page said "exactly one write" until Step 8's review; the enqueue is
there for what the ladder cannot cover — a job an operator deleted, or one
already finished — and it is a write a reader counting statements against an
unauthenticated route needs to know about.)

That write is new, and it is deliberately **not** `enqueue_in_tx` growing a
`DO UPDATE`. The argument against the upsert
([vpay-db.md](../vpay-db/jobs.md#enqueue_in_tx-exists-only-in-the-transactional-form))
is unchanged: the backstop scan re-enqueues every live charge's key every ten
minutes, and an upserting enqueue would drag a job scheduled a quarter of an
hour out back to now on every pass — a ladder that silently becomes a hot
loop. So a caller has to ask for the pull-forward, and exactly one does. It
refuses three states, each for its own reason: a **leased** job is being
polled right now and that poll will see the answer; a job already at or before
`now()` needs nothing (which is what makes a burst of duplicate callbacks free
rather than a row-lock queue); and a **parked** job — `run_at = 'infinity'` —
stays parked, because the whole point of a dead letter is that its
`dedupe_key` keeps scans _and callbacks_ from re-creating work a human has to
look at first.

Since Step 8's review it refuses a fourth: a job **already due within
`PULL_FORWARD_FLOOR`** — ten seconds, which is the poll ladder's own fastest
rung, `vpay_worker::poll_delay(0)`. The number is written out in
`provider_callback` because `vpay-api` cannot name `poll_delay` (the
dependency runs the other way), and
`the_pull_forward_floor_is_the_poll_ladders_first_rung` in
`backends/tests/integration/tests/provider_callback.rs` is the join that fails
if the two drift.

### What an anonymous caller can and cannot get out of it

This page used to say that everything such a caller could gain was "bounded by
what the ladder was going to do anyway". **That was wrong**, and the review
that found it is the reason the floor exists. What is true:

- A flood of callbacks about one charge is one row, forever: the `dedupe_key`
  carries a unique index.
- A charge the queue is about to ask about anyway now costs a caller nothing.
  A poll due inside the floor is left where it is, so the POST is two
  statements against `jobs`, no row changed, and **no rail request**. That
  covers the common case, because the ladder's first rung is where a charge
  sits immediately after its first poll.
- A charge parked _further out_ than the floor is still brought forward by
  every callback, and that is what the route is for. It is also the residual:
  the rungs grow (20 s, 30 s, 45 s, …) while the floor stays at ten, so a
  caller repeating against one live charge can hold it at roughly one
  authenticated `query_status` per worker claim. **There is no rate limit** —
  not per charge, not per source — and [status.md](../../status.md) says so.
- What actually stands between the route and rail traffic is therefore: the
  caller must know a v4 `provider_reference_id` for a live charge _on this
  deployment_; the work each accepted POST buys is one authenticated status
  query, which settles the charge the rail names or nothing at all; the body
  is bounded at 16 KiB; and nothing here writes charge or intent state under
  any circumstances.

The cost of the floor is stated where it is paid: a rail's callback arriving
while the charge sits on the ladder's first rung no longer settles it early —
it settles at that rung, up to ten seconds later than it would have before.
`a_callback_does_not_accelerate_a_poll_that_is_already_about_to_run` is that
behaviour, asserted rather than implied, and the headline case parks its job
at a later rung for the same reason.

The read behind it, `Charges::get_by_provider_reference`, is scoped by
`provider_code` as well as by the reference. The rail is named by a path
segment and the reference by a body anyone can write, so a lookup that ignored
the code would let a POST to one rail's callback path name another rail's
charge. Migration `0027` indexes exactly that pair, because this is the one
read in the system an unauthenticated caller can trigger at will and a
sequential scan over `charges` would be a denial-of-service surface that grows
with the deployment's own success.

### Why an unknown reference is a 202 and an unknown rail code is a 404

Both rails retry a non-2xx on their own schedules, so answering `404` to a
reference this deployment has no charge for buys a retry loop that can never
succeed. It would also make the endpoint an oracle for "does this charge
exist", which is the argument [`browser`](../vpay-api.md#the-router)'s uniform 404 already
makes about an unauthenticated surface. So the two 202s — "queued" and "never
heard of it" — are the same response, and the second is logged at `info` where
an operator debugging a misregistered callback host will find it.

An unknown _rail code_ is different and stays a 404: it is a statement about
this deployment's route table, which is public (a merchant learns the same set
from `payment_method_types`), and a rail whose code nobody linked is not a rail
that is going to retry. The handler produces it by calling `crate::not_found`
rather than by building its own `ApiError`, so "byte-identical to a mistyped
path" is structural instead of two literals that happen to agree —
`the_provider_nest_is_unauthenticated_and_its_404_is_the_routers_own` compares
the bodies.

### What it deliberately does not repair

`CallbackRef::ref_extra` is **discarded**. Orange's `parse_callback` carries a
`notif_token`, and sometimes a `pay_token`, out of the notification, and
`docs/flows/adapter-orange-money.md` names repairing a charge whose
`ref_extra` write was lost as a thing a callback _could_ do. It is not done
here, and doing it would need the stored `notif_token` compared against the
received one first — which nothing implements. Merging an unauthenticated
request's rail key material onto the row would corrupt the token the next
status query is addressed by, so the honest state is "not built", and
`docs/status.md` says so. `a_callback_writes_no_charge_or_intent_state` is
what holds it there.

---
