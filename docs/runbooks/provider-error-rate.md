# Runbook: rising `provider_error` rate

## Alert

The share of failures mapping to `provider_error` on one rail crosses its
threshold.

## What it means

`provider_error` is the escape hatch in the failure taxonomy: the adapter saw a
rail error string it does not recognise. A rising rate almost always means the
rail changed its error vocabulary and the adapter's mapping table has drifted.

It is **not** a payment problem in itself. It is a *visibility* problem, and it
degrades every downstream decision — merchants cannot tell a retryable failure
from a permanent one.

## Steps

1. Group recent `provider_error` charges by **`charges.failure_raw`** (the
   column's name; there is no `failure_reason_raw`). `charges.failure_code`
   is the taxonomy value; `failure_raw` is the rail's own words, capped at
   2000 characters by a CHECK.
2. If one raw string dominates, that is the new code. Map it to the right
   taxonomy entry in the adapter and ship.
3. If the raw strings are varied and include transport errors, check whether the
   rail is degraded — those may belong under `provider_unavailable`.
4. If the raw reason is empty, the adapter is failing to parse a changed
   response shape. **`provider_requests` will not show you the body — it
   stores none**, by design: only the charge, the operation, the reference,
   the attempt, `status_code`, `error_kind`, `sent_at` and `responded_at`.
   Use it to establish *whether and how* the rail answered, then go to the
   application logs for the (truncated) body.

   Reading `provider_requests` for that: `status_code IS NULL` means no
   answer was received; `status_code = 0` is a sentinel meaning the rail
   answered but the port carries no HTTP status (an accepted submit, or a
   decline) — see migration `0020`; anything else is the real HTTP status.
   `error_kind` is the error's own classification code, the same vocabulary
   the merchant's envelope uses: `provider_unavailable` (transport),
   `provider_error` (unparseable answer), `charge_declined` (the rail
   decided), `misconfigured` (our YAML or credentials — fix the deployment,
   not the mapping), `operation_unsupported_by_rail`, `not_implemented`.

5. **A rising `provider_error` rate with `status_code = 0` and
   `error_kind = 'charge_declined'`** is the mapping-drift case this runbook
   is about: the rail decided, and the adapter could not name the reason. A
   rising rate with `status_code IS NULL` and
   `error_kind = 'provider_unavailable'` is not — that is the rail being
   unreachable, and it belongs to the unresolved-charges runbook.

## Do not

- Do not widen an existing mapping to swallow the unknown string. A wrong
  mapping is worse than an honest `provider_error`, because it tells the
  merchant something false about whether to retry.
