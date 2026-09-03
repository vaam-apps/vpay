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

**Updated 2026-09-03 (Step 3): both adapters' mappings are implemented.**

The taxonomy itself is implemented and tested (`vpay-core::failure`).

- **MTN** transcribes the reason table above into
  `vpay_adapter_mtn_momo::mapping::FAILURE_REASONS`, asserted row by row and
  in both directions by `every_documented_reason_maps_to_its_documented_code`,
  `no_reason_appears_twice` and
  `an_unknown_reason_is_provider_error_and_never_a_guess`.
- **Orange** maps its four documented statuses in
  `vpay_adapter_orange_money::mapping`
  (`every_documented_status_maps_and_nothing_else_does`,
  `expired_is_the_payers_timeout_and_carries_a_raw_reason`,
  `an_unrecognised_status_is_an_error_never_a_failure`).
- **Over the wire**, both are proven by the shared conformance case
  `a_declined_charge_maps_to_the_documented_failure_code`, which drives a
  real `wiremock/wiremock` container per rail and asserts the taxonomy code
  the documented decline arrives as (MTN `NOT_ENOUGH_FUNDS` →
  `insufficient_funds`, `COULD_NOT_PERFORM_TRANSACTION` → `payer_timeout`,
  `NOT_ALLOWED` → `provider_account_blocked`; Orange `EXPIRED` →
  `payer_timeout`). Measured 2026-09-03: 26 conformance tests, 26 passed.
- **A decline reaches a merchant.** `POST …/confirm` on a rail that refuses
  the charge writes `charges.failure_code` + `failure_raw`, stamps the
  intent's `last_payment_error`, and answers `409 charge_declined`
  (`a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read`, an
  `invalid_payer` decline steered by the one field of the outgoing request a
  merchant controls — the MSISDN — and
  `credentials_the_rail_refuses_are_a_page_and_a_terminal_charge`, a
  `provider_account_blocked` one, which is a different code, a different
  severity and a different on-call answer).
  The rail's raw reason is stored and logged; only the taxonomy code and a
  generic message are public.

**`provider_error` is still the escape hatch, and it is now reachable from a
real response path** — an unmapped string arrives as `provider_error`
carrying the raw reason rather than being guessed at
([runbooks/provider-error-rate.md](../runbooks/provider-error-rate.md)).
**What none of this proves** is that the tables are faithful to the *rails*:
every decline above came from WireMock, and neither rail's real sandbox has
ever been called. Orange in particular documents no error vocabulary for
`webpayment` and no sub-reasons for `FAILED`, so both land in the
"unmapped, alert on it" bucket by design. See [../status.md](../status.md).
