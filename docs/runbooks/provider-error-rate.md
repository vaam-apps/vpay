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

1. Group recent `provider_error` charges by `failure_reason_raw`.
2. If one raw string dominates, that is the new code. Map it to the right
   taxonomy entry in the adapter and ship.
3. If the raw strings are varied and include transport errors, check whether the
   rail is degraded — those may belong under `provider_unavailable`.
4. If the raw reason is empty, the adapter is failing to parse a changed
   response shape. Check `provider_requests` for the actual body.

## Do not

- Do not widen an existing mapping to swallow the unknown string. A wrong
  mapping is worse than an honest `provider_error`, because it tells the
  merchant something false about whether to retry.
