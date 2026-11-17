# Runbook: charges parked in `requires_review`

## Alert

**`VpayChargeRequiresReview`** — `deploy/helm/vpay/templates/prometheusrule.yaml`,
group `vpay.rules`, `severity: critical`, label `provisional: "true"`,
`runbook_url` pointing here.

```promql
(
  sum by (provider, from, to) (
    vpay_charge_transitions_total{to="requires_review"}
    unless vpay_charge_transitions_total{to="requires_review"} offset 1h
  ) > 0
)
or
(
  sum by (provider, from, to) (
    increase(vpay_charge_transitions_total{to="requires_review"}[1h])
  ) > 0
)
```

`for: 0m`. **Two halves, and the first is the one that matters most.** Nothing
registers this series at 0. It is created by its first increment, so a pod's
first parked charge appears as a series whose first sample is already 1, and
`increase()` over a series with no earlier sample is 0. The rule's first draft
was the second half alone, and it stayed silent on exactly that charge
(caught in review; `deploy/helm/vpay/ci/prometheusrule.test.yaml` now has a
case that starts at 1 and must fire). The first half fires on a series that
exists and did not an hour ago. The second fires on one that grew. Either way
the alert stops an hour after the last parking. The threshold is "more than none": every charge that reaches this
state is a person's problem. The rule is `critical` where
`VpayUnresolvedChargesRising` is a `warning`, because the outcome here is
known, and it is the bad one: **a payer was charged for a payment the merchant
had already cancelled.**

The metric carries `{provider, from, to}` and no charge id (a label per charge
would be unbounded cardinality). The charge id is on the log line the same
commit writes, at `ERROR` with `alert = true`:

```text
a rail-accepted payment landed on a canceled intent; the charge was parked in
requires_review for an operator — no refund was made and nothing was posted
```

with the fields `charge_id`, `payment_intent_id`, `provider_code`,
`provider_reference_id`, `provider_txn_id`, `amount` and `currency_code`. Both
the counter and the log line are written **after** the parking commits, and
only by the delivery whose compare-and-swap moved the charge. A duplicate rail
answer (the poll and a callback, or a retried job) finds the charge no longer
live and writes, counts and logs nothing.

> **Like every rule in the chart, this one has never been evaluated against a
> real series.** `just test-alert-rules` runs it against synthetic series in
> CI. No Prometheus has scraped a vpay process, and
> `metrics.prometheusRule.enabled` is `false` by default
> ([README.md](README.md)).

The query below is the primary tool regardless. The metric reports arrivals,
and reconciling money needs the rows:

```sql
SELECT c.id AS charge_id, c.payment_intent_id, c.provider_code,
       c.provider_reference_id, c.provider_txn_id, c.amount, c.currency_code,
       c.updated_at AS parked_at, p.merchant_id, p.status AS intent_status
FROM charges c JOIN payment_intents p ON p.id = c.payment_intent_id
WHERE c.state = 'requires_review'
ORDER BY c.updated_at;
```

## What it means

A cancel and a confirm raced. The cancel committed first, so the merchant was
sent `payment_intent.canceled`. The confirm's charge committed onto the
cancelled intent anyway (both requests were legal when each started; see
`docs/flows/payment-lifecycle.md`). The rail was called, the payer approved,
and the rail reported the charge paid. When that answer was settled
(`vpay_db::Settlement::apply_succeeded`), the intent was `canceled`.

What vpay did, in one committed transaction:

- the charge moved to `requires_review`, keeping its `amount`, its
  `provider_reference_id` and the rail's `provider_txn_id`;
- the intent was **left `canceled`**, with `amount_received = 0`;
- **no event was written.** The merchant has been told "canceled" and nothing
  else. vpay has no event type for this case;
- **nothing was posted to the ledger.** The capture posting a normal
  settlement makes would credit the merchant for money whose fate an operator
  has not decided;
- **no refund was attempted.** That was a decision (issue #96), not a gap:
  refunding automatically is a second money movement made without a person.

Until 2026-11-17 the same race rolled the settlement back whole. The charge
stayed live, the rail's id was discarded, and the poll job retried the same
refusal until it was dead-lettered. The only record was a page. A charge that
reached that state before this change is still live, not parked. Look for
`DbError::WriteMatchedNoRow` on `payment_intents` in the worker's logs, or for
a live charge whose intent is `canceled`.

## Steps

1. Run the query above. Note the charge id, the rail, the reference, the
   rail's transaction id and the amount.
2. Confirm the money moved, at the rail. Do not trust vpay alone: query the
   rail with `provider_reference_id`, or use the rail's dashboard, and check
   the day's settlement statement for the amount.
3. Tell the merchant. vpay has told them the payment was cancelled, and it
   will not tell them anything else. They may want to fulfil the order rather
   than refund it.
4. Decide what happens to the money, with the merchant:
   - **refund the payer.** vpay has no way to do this for a charge in this
     state today. `POST /v1/refunds` refuses an intent that is not
     `succeeded`, and `ProviderAdapter::refund` is a declared
     `NotImplemented` on some rails (`docs/status.md`). Refund through the
     rail's own tools and record it outside vpay;
   - **keep it as reconciled.** There is no action that records this either.

   **Neither outcome can be written to vpay today.** The charge stays in
   `requires_review` after you have dealt with it. The operator action that
   would move it out is issue #96's open remainder. Record your finding
   wherever your team keeps investigation notes. The `charges` table has no
   annotation column (see [unresolved-charges.md](unresolved-charges.md),
   step 5).

## Escalate when

- More than one charge is parked in a short window. A single race is two
  requests landing within a few hundred milliseconds of each other. A stream
  of them means something cancels intents that are being confirmed: a client
  bug or an expiry sweep running against live checkouts.
- The rail denies the payment that it reported paid. That is a rail
  contradiction. Follow [worker-queue.md](worker-queue.md).

## Do not

- Do not `UPDATE` the charge to `succeeded`, or the intent to `succeeded`, by
  hand. That writes a settlement with no event, no ledger posting and no
  session or invoice update. It is the half-committed state the settlement
  transaction exists to prevent.
- Do not move the charge back to a live state to "let the poll settle it". The
  next poll would meet the same `canceled` intent and park it again, with a
  second alert.
- Do not create a second charge on the intent. One charge per intent, forever.

**Status:** written 2026-11-17 from the code. Never followed against a
deployment, because none exists.
