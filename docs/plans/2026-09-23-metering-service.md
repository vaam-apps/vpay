# Usage metering: a separate service beside vpay

- **Date:** 2026-09-23
- **Request, verbatim:** "plan another service (not vpay) for tier 4" — tier 4
  being usage-based billing in the gap list drawn up against Lago's API
  (v1.53.0, <https://swagger.getlago.com/>): `billable_metrics`, usage
  `events`, `fees`, `current_usage` / `past_usage` / `projected_usage`,
  `lifetime_usage`, usage `alerts`, charge `filters`.
- **Status:** A proposal. No repository, no code. **Working name: `vmeter`**,
  a placeholder beside `vpay` and `vsms`. When the service gets its own
  repository, this brief moves there, and this file keeps a pointer.

This is a plan for a service that is not vpay. It lives here because the
boundary it proposes is written against vpay's surface, and that surface is
what this repository can check.

## What already existed, read rather than assumed

- **vpay invoices** take lines that are a description, a quantity and a unit
  amount. `amount` is `quantity * unit_amount`, enforced by the database
  (`amount_is_the_product`, [invoices.md](../flows/invoices.md)).
- **[RFC-0004](../rfc/0004-billing-on-top-of-invoices.md)** proposes the
  three seams this service needs, none of them built:
  - § 1: prices are flat per unit, with no tiers and no metered usage type.
  - § 2: a subscription renewal writes a **draft** invoice and finalizes it
    after a per-merchant **draft window**.
  - § 3: **pending invoice items** carry a line onto the next invoice.
- **vpay's `/v1/events`** is its record of domain events. The name collides
  with Lago's usage-event ingest, which is one reason ingest does not belong
  on vpay's surface.
- **Merchant authentication** (ADR-0010) is `private_key_jwt`, registered by
  pull request, with a two-scope vocabulary: read and write.
  `Idempotency-Key` is kept for 24 hours.
- **Webhooks** are per merchant, in YAML, signed with `Vpay-Signature`.
- **ADR-0022** already treats Postgres connections, not CPU, as vpay's
  scaling constraint.

## Why a separate service

1. **A different load.** Usage ingest is high-rate, append-only, and
   tolerant of seconds of lag. vpay's database is the money path, where every
   write is a compare-and-swap under a connection budget (ADR-0022). An ingest
   spike must not be able to take a checkout down.
2. **Different failure semantics.** If metering is down, payments must still
   work, and vpay should not have to know metering exists except as a
   merchant client that writes lines.
3. **vmeter never touches money.** It produces _quantities and prices_; vpay
   turns them into documents, and rails turn documents into money. Keeping
   that line makes vmeter a service with no payment-regulation surface at all.
4. **It may want other storage** (a columnar store for rollups) without that
   choice entering vpay's `deny.toml`, image or backups.

## The shape

```text
   merchant app ──usage events──▶ vmeter ──(1) aggregate per metric, per customer, per period
                                    │
   vpay ──invoice.created (draft)──▶│  webhook, per merchant
                                    │
                                    │  (2) close the period: cutoff = period_end + grace
                                    │  (3) rate: quantities × price tiers → priced lines
                                    ▼
   vpay ◀──POST /v1/invoice_items (onto the draft, or pending)── vmeter
     │
     └─ finalizes at the end of the draft window; the rails collect
```

### M1. Metrics

- A **billable metric** has a `code`, an `aggregation` (`count`, `sum`,
  `max`, `unique_count`, `latest`), a `field` in the event properties, and
  optional `filters` (property equals value).
- Metrics are versioned. Changing a metric's aggregation creates a new
  version from a date, never a rewrite of closed periods.

### M2. Ingest

- `POST /v1/meter_events` and `POST /v1/meter_events/batch`. The spelling is
  Stripe's Billing Meters, not Lago's `events`, to keep one mental model for
  merchants already on vpay's Stripe shape. It is checked against Stripe's
  current reference when written.
- An event has an `identifier` (the merchant's, unique per merchant), a
  `metric`, a `customer` (vpay's `cus_…`), a `timestamp` and a `payload`.
- **Exactly once:** a unique index on `(merchant, identifier)`, held for as
  long as the event is retained, not for 24 hours. A replay is acknowledged
  and not counted.
- Storage is Postgres, with the events table partitioned by month and rollup
  tables per `(metric, customer, hour)`. Whether a columnar store is needed is
  measured, not assumed.

### M3. Rating

**vmeter rates; vpay does not.** Tiered, graduated, volume and package pricing
live in vmeter's own price plans, keyed by vpay `price_…` ids for traceability.
Their output is always lines vpay already understands: description, quantity,
unit amount. A graduated tier becomes one line per tier.

The alternative is teaching vpay's `price` tiers and a metered usage type, so
vmeter only reports quantities (Stripe's own split). It is not proposed. It
would put a rating engine inside the money path, and RFC-0004 § 1 keeps prices
flat on purpose. That choice can be revisited without changing the boundary,
because vmeter could still send quantities to a future metered price.

### M4. Period close and the push to vpay

1. vmeter receives `invoice.created` for a subscription invoice (it is a
   registered webhook endpoint for the merchant).
2. It closes that subscription's period at `period_end + grace`, where
   `grace` must be shorter than vpay's draft window. The closed rollup is
   frozen, and every line pushed records the snapshot it came from.
3. It `POST`s one invoice item per rated line onto the draft, with an
   idempotency key derived from `(subscription, period, metric, tier)`.
4. **vmeter's own push ledger is the dedupe**, not vpay's 24-hour
   idempotency window: a line recorded as pushed is never pushed again.
5. **Late events** (after close) are counted in the period they belong to,
   and billed as a pending item on the **next** invoice, labelled with the
   period they are for. **Missed windows** (vpay finalized first) go the same
   way. Usage is never lost and never billed twice; it is sometimes billed
   late, and the line says so.

### M5. Reads and alerts

- `GET /v1/customers/{id}/usage` returns the current period so far, and
  `GET …/usage/projected` a linear projection. Both are labelled as
  estimates.
- Past periods come from the frozen snapshots, and equal what was invoiced.
- Threshold alerts are delivered as vmeter's own signed webhooks. Its event
  types are its own, because vpay's "Stripe types only" rule is vpay's.

## Decisions this plan needs, and whose they are

| #   | Decision                                                                                                                                                                                                                      | Whose          |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------- |
| D1  | **How vmeter authenticates to vpay.** Per merchant, as a merchant client registered by pull request, is proposed. With two scopes it holds full write for that merchant; resource-scoped tokens are RFC-0004 open question 10 | Maintainer     |
| D2  | vmeter rates (proposed) or vpay grows metered prices                                                                                                                                                                          | Maintainer     |
| D3  | Ingest spelling: Stripe Billing Meters (proposed) or Lago `events`                                                                                                                                                            | Maintainer     |
| D4  | Stack: Rust on the same toolchain and engineering standards (ADR-0016), Postgres, CrateStack optional                                                                                                                         | Maintainer     |
| D5  | Grace versus draft window defaults, which RFC-0004 open question 3 decides from vpay's side                                                                                                                                   | Maintainer     |
| D6  | Retention of raw usage events. They are per-customer behavioural data and fall under the same ADR-0020 inventory discipline vpay applies to itself                                                                            | RFC-0002 owner |
| D7  | Repository and name                                                                                                                                                                                                           | Maintainer     |

## Milestones

| Step | Contents                                                                                                                                  | Needs from vpay                                      |
| ---- | ----------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| V0   | Repository, CI, gates copied in spirit from vpay: no mocks in the shipping binary, `NotImplemented` over plausible success, a status page | —                                                    |
| V1   | Metrics and exactly-once ingest, with rollups; a load test that states its numbers                                                        | —                                                    |
| V2   | Rating over flat and graduated tiers, pure and property-tested                                                                            | —                                                    |
| V3   | Period close and the push onto a draft, end to end against a real `vpay-server`                                                           | RFC-0004 steps B and C (pending items, draft window) |
| V4   | Late and missed-window carry-over; a kill-9 test at every step of the close                                                               | RFC-0004 § 3                                         |
| V5   | Usage reads, projections, alerts                                                                                                          | —                                                    |

## What will not be true when this ships

- **vpay will not know what usage is.** It will see invoice lines from a
  merchant client, like any other.
- **No real-time spend cap.** Alerts inform; nothing stops a customer's usage
  or refuses a payment.
- **No money moves because of vmeter** except through a vpay invoice a payer
  then pays.
- **Estimates are estimates.** A projected usage figure is not an amount owed.
