# Failure taxonomy

`charges.failure_code` is a **closed vocabulary owned by the core**. Adapters map
their rail's error strings into it. Merchants integrate against this list once
and it does not grow when a rail is added.

| Code | Meaning | Payer can retry? | Whose problem |
|---|---|---|---|
| `insufficient_funds` | Not enough balance | Yes, new intent | Payer |
| `payer_timeout` | Never approved in time | Yes, new intent | Payer |
| `payer_declined` | Actively rejected the prompt | Yes, new intent | Payer |
| `invalid_payer` | Identifier not valid on this rail | No — fix the number | Payer/merchant |
| `payer_limit_reached` | Wallet or KYC-tier limit | Later | Payer |
| `payer_account_blocked` | Payer account not active | No | Payer |
| `invalid_payee` | Merchant's receiving account invalid | No | Merchant config |
| `payee_account_blocked` | Merchant's receiving account not active | No | Merchant config |
| `provider_account_blocked` | **Your** partner account is blocked | No | **Page yourself** |
| `provider_unavailable` | Rail down or timing out | Yes, later | You |
| `provider_error` | Unmapped; carries the raw reason | Unknown | Investigate |

## `provider_error` is an alert, not a resting place

A rising `provider_error` rate means an adapter's mapping table has drifted
behind the rail's actual error strings. Alert on it. Do not tolerate it.

## Adapter mappings

Each adapter's mapping lives in its own flow doc:
[MTN](adapter-mtn-momo.md) · [Orange](adapter-orange-money.md).

## Status

The taxonomy is implemented and tested (`vpay-core::failure`). Neither adapter's
mapping is implemented — see [../status.md](../status.md).
