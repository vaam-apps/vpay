# Customers — the twelve-month retention sweep

_Split out of [docs/flows/customers.md](../customers.md) on 2026-09-11 by exp57, which broke a 888-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## The twelve-month retention sweep

`sweep_idle_customers` — a `jobs.kind` of its own (migration `0034`), seeded
at worker boot on the singleton dedupe key `sweep:customers`, rescheduling
itself hourly.

Each pass **erases** every customer idle for more than
`CUSTOMER_IDLE_AFTER` (365 days) and not already erased — hard-deleting the
ones nothing references and anonymising the rest, exactly as
`DELETE /v1/customers/{id}` does. One `customer.deleted` per erasure, in the
same transaction.

**Until 2026-09-10 it skipped every customer a payment intent, a checkout
session or an invoice referenced**, because the `NO ACTION` foreign keys made
deleting one impossible and offering one would have minted an `evt_…` for a
deletion Postgres was about to refuse. That was a consequence of the schema
rather than a decision, and its effect was that the twelve-month promise did
not apply to any payer a merchant had ever billed or taken money from.
Migration `0041` separates the two things it conflated: nothing is detached,
and the payer is still erased.

The guard that replaced it is `anonymized_at IS NULL`. Without it an
anonymised customer stays idle for ever and the sweep offers it again on the
next pass, and the one after — an hourly `customer.deleted` about a payer
already erased, for the life of the deployment.

"Idle" is measured by `customers.last_used_at`, and **"used" means**: created,
updated, or named by a payment intent, a checkout session or an invoice. Every one of
those paths calls `vpay_db::Customers::touch_last_used`. A path that is
_missing_ does not fail — it makes a live customer look idle, and the sweep
deletes it twelve months later with nothing in any log saying anything unusual
happened. That is why the confirm path stamps as well as the create path, and
why `a_confirm_stamps_the_customers_clock_and_keeps_it_out_of_the_sweep`
exercises it through the router rather than calling the method.

The stamp is **monotonic**: `touch_last_used` filters on `last_used_at < now`,
so a vpay process whose clock is behind cannot rewind a customer's retention
clock. Two processes do not share a clock and the horizon is twelve months, so
a rewind is not a rounding error — it is the difference between surviving a
pass and not.

The erasure and its `customer.deleted` event are **one transaction**, and so
are the four redaction statements. A crash between them would erase a
merchant's customer with nobody ever told, and there is no sweep over
"customers erased without an event" and no way to build one for the branch
where the row is gone.

### Why it is its own job kind

It runs on `sweep_expired`'s schedule and its healthy answer is zero too,
which is the argument that put checkout-session expiry _inside_ that job. What
separates this one is what a failure means: `sweep_expired`'s three statements
are bounded deletes of vpay's own bookkeeping, and this one erases a
merchant's personal-data records and tells them about it. Sharing a job would
report a failing customer sweep as "the housekeeping sweep is unhealthy", with
an idempotency-key count it has nothing to do with in the same log line, and
would put a merchant-visible event behind the same lease as an internal
delete.

### `customer.deleted` is the only way a merchant learns

A hard delete is unobservable by polling: the object is gone, and a `GET`
afterwards is byte-identical to a `GET` for an id that never existed. An
anonymisation is _observable_ — the object answers `deleted: true` — but only
to a merchant who thinks to re-read it, which nobody does on a schedule for
twelve months.

The event's `data.object` is the **redacted** customer: the ids, `created`,
`livemode`, the merchant's own `metadata` and `deleted: true`. It names which
`cus_…` was erased and nothing about the payer. See "What the merchant is
told, and what changed about it" above for why that is the reverse of what
this paragraph said until 2026-09-10 and what the reversal costs.

---
