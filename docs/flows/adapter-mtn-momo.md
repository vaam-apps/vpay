# Adapter: MTN MoMo Cameroon

**Flow: push.** `supports_refunds: true` (via Disbursements).

## Preconditions

| Precondition | MTN |
|---|---|
| Caller supplies its own reference | **Yes** — `X-Reference-Id`. It *is* the transaction id |
| Final status queryable by that reference | **Yes** — `GET /collection/v1_0/requesttopay/{ref}` |

Both hold, which is why MTN is a safe push rail.

## Credential hierarchy

Confusing these three is the most common onboarding bug.

1. **Subscription Key** (`Ocp-Apim-Subscription-Key`) — from the developer
   portal, **different per product** (Collections vs Disbursements).
2. **API User + API Key** — created once via `POST /v1_0/apiuser` (you supply a
   UUID and a `providerCallbackHost`) then `POST /v1_0/apiuser/{uuid}/apikey`.
3. **Access token** — `POST /collection/token/` with HTTP Basic, `expires_in:
   3600`. Collections and Disbursements have **separate tokens**, hence the
   `scope` column on cached tokens.

## The collection call

```http
POST /collection/v1_0/requesttopay
Authorization: Bearer <token>
Ocp-Apim-Subscription-Key: <collections key>
X-Target-Environment: sandbox | mtncameroon
X-Reference-Id: <the charge's provider_reference_id>
X-Callback-Url: https://<registered host>/provider/mtn_momo/callback

{ "amount": "5000", "currency": "XAF", "externalId": "<charge id>",
  "payer": { "partyIdType": "MSISDN", "partyId": "23767XXXXXXX" },
  "payerMessage": "…", "payeeNote": "…" }
```

Returns **202 with an empty body**. `X-Callback-Url` is per-request and its host
must match the registered `providerCallbackHost`.

Status: `GET /collection/v1_0/requesttopay/{ref}` → `PENDING` | `SUCCESSFUL` | `FAILED`.

## Failure mapping

| MTN `reason` | → core code |
|---|---|
| `NOT_ENOUGH_FUNDS` | `insufficient_funds` |
| `COULD_NOT_PERFORM_TRANSACTION` | `payer_timeout` (PIN not entered, ~5 min) |
| `PAYER_NOT_FOUND` | `invalid_payer` |
| `PAYER_LIMIT_REACHED` | `payer_limit_reached` |
| `SENDER_ACCOUNT_NOT_ACTIVE` | `payer_account_blocked` |
| `PAYEE_NOT_FOUND` | `invalid_payee` |
| `PAYEE_NOT_ALLOWED_TO_RECEIVE` | `payee_account_blocked` |
| `NOT_ALLOWED` | `provider_account_blocked` |
| `SERVICE_UNAVAILABLE` / 503 | `provider_unavailable` |
| anything else | `provider_error` + raw reason |

HTTP: `409 RESOURCE_ALREADY_EXIST` on a duplicate reference — **the adapter must
report this as `Submitted`**. `404` → `NotFound`, never a failure.

**MTN's biggest wart: several *logical* errors return HTTP 500** —
`INVALID_CURRENCY`, `NOT_ALLOWED_TARGET_ENVIRONMENT`, `INVALID_CALLBACK_URL_HOST`,
and an `INTERNAL_PROCESSING_ERROR` that can mean insufficient funds *or* the
wallet platform being down. Parse the body's `code` before deciding anything;
never treat 500 as blind-retry.

## Environment values (all just config)

| | Sandbox | Cameroon production |
|---|---|---|
| `base_url` | `https://sandbox.momodeveloper.mtn.com` | `https://proxy.momoapi.mtn.com` — **confirm** |
| `target_environment` | `sandbox` | `mtncameroon` — **confirm; subsidiary-specific** |
| `currency` | **EUR only** | XAF |

## Status

Capabilities are declared and tested. **No wire call is implemented** — see
[../STATUS.md](../STATUS.md).
