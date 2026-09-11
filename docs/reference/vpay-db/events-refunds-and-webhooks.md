# `vpay-db` — `events`, `refunds` and `webhook_deliveries`

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## `events`

**The row is written in the same transaction as the state change** — not
afterwards, and not by a trigger. An event committed separately from the
transition it describes is either a webhook for something that did not happen
(the transition rolled back) or a transition no merchant is ever told about (the
event write failed), and the second is the one that actually happens, because it
is the failure nothing retries. So `insert_in_tx` takes a connection, never a
pool, and there is deliberately no pooled variant.

Events are written for **terminal transitions only** —
`payment_intent.succeeded` and `payment_intent.payment_failed`, both from
`settlement`'s single transaction. The milestone types
[webhooks.md](../../flows/webhooks.md) also lists are not emitted by anything yet;
[../status.md](../../status.md) is the record of which types are live.

`pending_page` is the backlog query the drain runs; `list_page` and `get_by_id`
are `GET /v1/events` and `GET /v1/events/{id}`, the documented fallback for a
webhook a merchant missed. Those two are merchant-scoped in SQL and page exactly
as `payment_intents::list_page` does; the handlers and the `EventObject`
renderer they and the deliverer must share live in `vpay-api`.

## `refunds`

**One read, no write, and the write's absence is the point.** `GET
/v1/refunds/{id}` was made part of the `/v1` contract on 2026-09-05 (issue
#45) because a refund is the one money movement on this surface with no
authoritative read: it is asynchronous and non-terminal (`pending`), the two
documented refund event types are emitted by nothing, and webhook delivery is
at-least-once and unordered. **Creating** one is a different question and is
still unanswered — `ProviderAdapter::refund` is `NotImplemented` on MTN
(refunds are the Disbursements product) and `Unsupported` on Orange — so
`Refunds` exposes `get_for_merchant` and nothing else. A `create` here would
be a write path no shipping code calls, which is a feature this repository
would be claiming it has.

**The tenant is reached by a join, and migration `0017` was deliberately not
altered.** `refunds` has no `merchant_id`; it has a `NOT NULL` foreign key
onto `payment_intents (id)`, and the intent is where the tenant lives. So the
one statement is

```sql
SELECT … FROM refunds r
  JOIN payment_intents p ON p.id = r.payment_intent_id
 WHERE p.merchant_id = $1 AND r.id = $2
```

A denormalised `merchant_id` column was the alternative and was rejected: it
would be a _second_ answer to "whose refund is this?", and two answers to a
tenancy question is how one of them ends up stale — for the cost of one
primary-key lookup per read. It would also have collided with the migration
numbering of two other branches in flight the same day, which is a reason to
notice the choice rather than a reason to make it.

`RefundRow` is a **projection**, not the whole table: `charge_id`,
`failure_code`, `failure_raw`, `provider_reference_id` and `updated_at` are on
the row in Postgres and on no wire object, and the writer that would fill them
does not exist. `fee` (migration `0031`, issue #46, 2026-09-06) **is** in the
projection, for the mirror-image reason: it is on the wire object as the tenth
key, so leaving it out would make the renderer invent a value. It is
`Option<i64>` all the way through — the column has no `DEFAULT`, `NULL` means
"the rail reported no fee" and `0` means "the movement was free" — and, since
nothing writes a `refunds` row at all, every value this repository can read
today is `NULL`. That is `events::EventRow`'s rule for `fanout_attempts`, not
`checkout_sessions::CheckoutSessionRow`'s one-to-one rule, and it is the right
one here precisely because guessing at the shape of code nobody has written is
what this repository calls claiming a feature.

## `webhook_deliveries`

**One row per (event, endpoint), created by the fan-out transaction.** The drain
reads the backlog and, per event, opens one transaction that creates a delivery
row per configured endpoint, enqueues a `deliver_webhook` job per created row,
and marks the event fanned out. All of it commits together, which is the only
arrangement in which a crash is harmless: an interrupted pass leaves the event
`pending` and the next pass redoes the whole of it, absorbed by
`webhook_deliveries_event_endpoint` and `jobs_dedupe_key`. Splitting the flip
from the inserts gives the two failures that matter — an event marked delivered
that has no delivery rows (a webhook nobody will ever send), or a second set of
rows for an event already fanned out (every webhook sent twice).

That is why `create_in_tx` and `mark_fanned_out_in_tx` take a connection and
there is deliberately no pooled variant of either.

**Why the `events` write lives in this module.** `mark_fanned_out_in_tx`
updates `events`, not `webhook_deliveries`. It is here rather than in `events`
because it is the _fan-out's_ closing write and is meaningless without the
inserts it commits beside: a caller that could reach it from the events module
could mark a backlog fanned out without creating a single delivery, which is
precisely the failure the shared transaction exists to make unreachable.

**Every column but `created_at` describes the most recent attempt.** `attempt`,
`state`, `status_code`, `response_excerpt`, `sent_at`, `responded_at` and
`next_attempt_at` are all rewritten by `record_attempt` and `record_success`.
This is a _state_ row with the latest attempt's outcome on it, not an
append-only attempt log — the per-attempt forensic trail is the worker's
structured log — and `payload_sha256` is the one column that deliberately does
not move.

The excerpt is truncated to migration `0022`'s `excerpt_length` ceiling here
rather than trusted from the caller, so an over-long excerpt cannot arise. The
worker cuts a receiver's body far shorter than that before it ever arrives; the
bound in this crate is the backstop, and it is what keeps the transport-failure
excerpt (which carries a whole `source()` chain) inside the CHECK.

### `record_attempt`: what each column is allowed to say

`status` is `None` for a transport failure and `responded_at` is cleared to
match. `status_code IS NULL AND responded_at IS NULL` with a `sent_at` set is
the encoding for "the request went out and nothing came back", which is why
migration `0022` deliberately carries no CHECK pairing those three columns.
Recording a heard refusal as an unheard one, or the reverse, is the one thing
this row must not do — the same argument `ProviderRequests::record_response`
makes for a rail.

`exhausted` is the caller's decision, not this layer's: the retry ladder lives
in `vpay_worker::delivery_delay` and `next_attempt_at` is the instant it
produced. The write is guarded on `state = 'pending'`, so a second call after
exhaustion changes nothing and a replayed job cannot walk `attempt` past the end
of the ladder.

`sha` is an `Option` because not every failed attempt rendered and signed
anything, and `None` leaves `payload_sha256` exactly as it was — including
`NULL`. The column records the digest of the bytes that were **rendered and
signed**, so an attempt abandoned before rendering must not stamp a digest for a
body that was never produced; the next attempt's mismatch check would then be
comparing against a body that never existed. _Rendered and signed_, not
_received_: a transport failure passes `Some`, because the signature was
computed over those exact bytes before the socket was ever opened.

When `sha` is `Some` it is `COALESCE`d rather than assigned, so the digest of
the _first_ attempt that rendered and signed a body survives — which is not
necessarily attempt 1. The handler compares its freshly rendered body against
the stored digest before sending and treats a mismatch as poisoned, so in every
non-buggy path the two are equal; keeping the earlier value means that if that
check is ever missed, the row still says what was originally signed instead of
quietly agreeing with whatever was sent last.

### `pending_due` is a backstop, never a scheduler

Delivery is driven by the `jobs` queue: the fan-out enqueues a job in the same
transaction that creates the row, and each failed attempt reschedules that job.
In a healthy deployment this query returns nothing. It is also the query an
operator runs to answer "what is outstanding right now?".

Two shapes qualify, and the second is why it takes a `lease`:
`next_attempt_at <= now()`, and
`next_attempt_at IS NULL AND created_at < now() - lease` — a delivery that has
**never** been attempted and whose job is not simply young. That second clause
was deliberately absent before migration `0023`, on the argument that a
never-attempted row's job was written in its own transaction so the two cannot
disagree. They can: the transaction makes the job _exist_, and nothing makes it
survive an operator's `DELETE` or a `jobs` truncation. Such a row was
unrecoverable, and the merchant is never told.

The `lease` is what keeps the scan from racing the queue rather than backing it
up: a delivery created moments ago has a job that has simply not been claimed
yet, and `RecoveryPolicy::lease` is the longest a claim may legitimately be
outstanding.

A returned row whose job was **dead-lettered** is a different case and is not
recovered by re-enqueuing — see
[vpay-worker.md](../vpay-worker.md#the-outbox-drain) and
[webhooks.md](../../flows/webhooks.md).
