# Runbook: unresolved charges

## Alert

A charge has been in `unresolved` for more than one hour — i.e. it passed 24
hours without the rail giving a terminal answer.

## What it means

**The payment is not lost.** It is escalated. The charge is still being polled,
hourly. The intent is still `processing`, which merchants are told means "not
yet, don't ship".

What it does *not* mean: that the payer was not debited. That is the question
you are here to answer.

## Steps

1. Open the charge in the dashboard. Note `provider_reference_id`, amount,
   `created_at` and the rail.
2. Read the `provider_requests` timeline. Did we ever get a response to submit?
   - **No response recorded** → on a redirect rail, the payer was never given a
     URL, so no payment can have occurred. Safe to fail the charge.
   - **Response recorded** → continue.
3. Query the rail directly with the reference (MTN: `GET
   /collection/v1_0/requesttopay/{ref}`; Orange: `transactionstatus` with
   `order_id` + `amount` + `pay_token`).
4. Reconcile against the rail's settlement statement for that day, by amount and
   timestamp.
5. Record the finding in the charge's annotation field. **Always** — the next
   person needs your reasoning, not just your conclusion.

## Escalate when

- The rail's statement shows a debit that its API denies. Contact the
  subsidiary; do not resolve the charge from the API alone.
- More than a handful appear at once. That is a rail incident, not a
  transaction problem — check the rail-health view first.

## Do not

- Do not force the charge terminal because it is old. Age is not evidence.
- Do not create a replacement charge on the same intent. The database will
  refuse it, and the reason it refuses is exactly this scenario.
