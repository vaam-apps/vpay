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
   Read it off `status_code`, which has **three** meanings — the pairing is
   enforced by the `response_is_paired` CHECK and documented on the column
   itself (migrations `0016` and `0020`):
   - **`status_code IS NULL`** (with `responded_at IS NULL`) → **no answer was
     received.** `error_kind` says what went wrong on our side of the wire:
     `provider_unavailable` (transport — connection refused, TLS failure,
     deadline expired) or `provider_error` (bytes came back and did not
     parse). On a redirect rail the payer was never given a URL, so no
     payment can have occurred; on a **push** rail this is exactly the
     ambiguous case — the rail may be holding the request. Do not fail the
     charge on this evidence alone; go to step 3.
   - **`status_code = 0`** → the rail **answered**, but the provider port
     carries no HTTP status for that answer. This is a sentinel, not an HTTP
     code. It appears for an accepted submit (`error_kind IS NULL`) and for a
     decline (`error_kind = 'charge_declined'`).
   - **A real HTTP status** → the rail answered with it.

   The `error_kind` vocabulary is `vpay_core::Classify::code` on the
   adapter's error, i.e. the same tokens a merchant sees in the error
   envelope: `provider_unavailable`, `provider_error`, `charge_declined`,
   `misconfigured`, `operation_unsupported_by_rail`, `not_implemented`.
   `misconfigured` is the one that means *stop and fix the deployment* — the
   adapter refused before or because of a bad credential, header or
   `base_url` — not a rail problem.

   **`provider_requests` stores no request or response body.** Columns are
   the charge and provider, the operation, the reference, the attempt
   number, `status_code`, `error_kind`, `sent_at` and `responded_at` — by
   design, so a rail's payload can never end up in a table an operator
   browses. If you need the body, you need the application logs for that
   request, and even there the raw failure reason is truncated.
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
