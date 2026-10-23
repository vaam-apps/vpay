# Failure taxonomy

`charges.failure_code` is a **closed vocabulary owned by the core**. Adapters map
their rail's error strings into it. Merchants integrate against this list once
and it does not grow when a rail is added.

| Code                       | Meaning                                 | Payer can retry?    | Notch Pay (stub only)                                        | Whose problem                           |
| -------------------------- | --------------------------------------- | ------------------- | ------------------------------------------------------------ | --------------------------------------- |
| `insufficient_funds`       | Not enough balance                      | Yes, new intent     | —                                                            | Payer                                   |
| `payer_timeout`            | Never approved in time                  | Yes, new intent     | `expired`, `abandoned`                                       | Payer; Notch Pay `…0f04`, `…0f05`       |
| `payer_declined`           | Actively rejected the prompt            | Yes, new intent     | `canceled`                                                   | Payer; Notch Pay `…0f02`                |
| `invalid_payer`            | Identifier not valid on this rail       | No — fix the number | —                                                            | Payer/merchant                          |
| `payer_limit_reached`      | Wallet or KYC-tier limit                | Later               | —                                                            | Payer                                   |
| `payer_account_blocked`    | Payer account not active                | No                  | —                                                            | Payer                                   |
| `invalid_payee`            | Merchant's receiving account invalid    | No                  | —                                                            | Merchant config                         |
| `payee_account_blocked`    | Merchant's receiving account not active | No                  | —                                                            | Merchant config                         |
| `provider_account_blocked` | **Your** partner account is blocked     | No                  | HTTP 401/403                                                 | **Page yourself**, on all three rails   |
| `provider_unavailable`     | Rail down or timing out                 | Yes, later          | —                                                            | You                                     |
| `provider_error`           | Unmapped; carries the raw reason        | Unknown             | `failed`, `rejected`, a `400`/`422` on initialize or process | Investigate; Notch Pay `…0f01`, `…0f03` |

## Which rail can produce which code

The table above says what each code _means_. It does not say whether anything
can produce it — and until 2026-09-10 one of them could not.
`payer_declined` was defined by the core, typed in both merchant SDKs and
given buyer copy by `examples/shop`, and no adapter emitted it
([issue #59](https://github.com/vaam-apps/vpay/issues/59)). Nothing failed,
because nothing compared a promise to a producer.

| Code                       | MTN MoMo                                    | Orange Money                | Notch Pay (stub only)                                        | Proven by                                                                               |
| -------------------------- | ------------------------------------------- | --------------------------- | ------------------------------------------------------------ | --------------------------------------------------------------------------------------- |
| `insufficient_funds`       | `NOT_ENOUGH_FUNDS`                          | —                           | —                                                            | `…0f01`                                                                                 |
| `payer_timeout`            | `COULD_NOT_PERFORM_TRANSACTION`, `EXPIRED`  | `EXPIRED`                   | `expired`, `abandoned`                                       | MTN `…0f02`, `…0f04`; Orange `…0f01`; Notch Pay `…0f04`, `…0f05`                        |
| `payer_declined`           | `PAYMENT_NOT_APPROVED`, `APPROVAL_REJECTED` | —                           | `canceled`                                                   | MTN `…0f05`, `…0f06`; Notch Pay `…0f02`                                                 |
| `invalid_payer`            | `PAYER_NOT_FOUND`                           | —                           | —                                                            | `…0f07`                                                                                 |
| `payer_limit_reached`      | `PAYER_LIMIT_REACHED`                       | —                           | —                                                            | `…0f08`                                                                                 |
| `payer_account_blocked`    | `SENDER_ACCOUNT_NOT_ACTIVE` †               | —                           | —                                                            | `…0f09`                                                                                 |
| `invalid_payee`            | `PAYEE_NOT_FOUND`                           | —                           | —                                                            | `…0f0a`                                                                                 |
| `payee_account_blocked`    | `PAYEE_NOT_ALLOWED_TO_RECEIVE`              | —                           | —                                                            | `…0f0b`                                                                                 |
| `provider_account_blocked` | `NOT_ALLOWED`, HTTP 401/403                 | HTTP 401/403                | HTTP 401/403                                                 | `…0f03`, and `bad_credentials_are_not_reported_as_a_payer_problem`, on all three rails  |
| `provider_unavailable`     | `SERVICE_UNAVAILABLE` on a `FAILED` body    | —                           | a `409` on initialize with no submit ever made (ADR-0032)    | MTN `…0f0c`; Notch Pay `notchpay_a_duplicate_no_submit_prompted_for_is_a_clean_decline` |
| `provider_error`           | anything unmapped                           | `FAILED`, anything unmapped | `failed`, `rejected`, a `400`/`422` on initialize or process | MTN `…0f0d`; Orange `…0f02`; Notch Pay `…0f01`, `…0f03`                                 |

`…0fxx` is the charge reference a WireMock mapping keys on, under
`backends/tests/conformance/wiremock/{mtn,orange,notchpay}/mappings/`.
`a_declined_charge_maps_to_the_documented_failure_code` drives every row
against a real container, and
`the_declines_prove_every_code_each_rail_can_produce` asserts the rows are
_all_ of them — it holds the cases against each adapter's
`PRODUCED_FAILURE_CODES`, so a code that gains a producer without a case, or a
case for a code the adapter does not declare, fails.

**No code is unreachable on every rail**, which is what changed on 2026-09-10.
Eight of the eleven are unreachable **on Orange**, and that is not a gap in a
table: Orange documents five statuses (`INITIATED`, `PENDING`, `SUCCESS`,
`EXPIRED`, `FAILED`) and no sub-reason for `FAILED` at all, so its protocol
cannot say "not enough funds" or "no such payer". A payer who clicks _Cancel_
on its hosted page arrives as `EXPIRED` — indistinguishable from one who
walked away — so `payer_declined` in particular is unreachable there, and
inventing a `CANCELLED` to make the rails look alike is refused rather than
done. `examples/shop`'s test-number panel states each of those eight, and its
`cannotExpress` rows are checked against
`vpay_adapter_orange_money::PRODUCED_FAILURE_CODES` so a claim cannot outlive
its truth.

**No variant is deleted, and none should be.** A code nothing produces today
is a documented reservation: the vocabulary is a wire contract, and removing a
variant would break a merchant deserialising it.

**Notch Pay produces five codes** (`PRODUCED_FAILURE_CODES` in
`vpay-adapter-notchpay`). Its read carries a status and no reason, so
`insufficient_funds` is unreachable there: the sandbox's "insufficient funds"
number read `complete` with a short `amounts.total`, not a decline. A payer
who never answers reads `expired` or `abandoned`, which is `payer_timeout`.
`canceled` is `payer_declined` (stub `…0f02`). That is evidence of a decline,
not proof of whose: the merchant's public key can cancel a payment too
(ADR-0031), and the read names no actor, so such a cancel is recorded as the
payer's ([adapter-notchpay.md](adapter-notchpay.md) § "Production risks",
item 4). A `503` or a timeout is `ProviderError::Transport`, never a decline.
`provider_unavailable` comes from one place only: `prepare`'s `409` for a
reference no submit was ever made for, an initialize whose answer was lost and
that prompted nobody, so the charge fails and the payer can try again
(ADR-0032). The column heads say "stub only" because each row's proof is a
WireMock case: the adapter's 2026-10-21 sandbox run
([step 5](../status/verification/2026-10-21-notchpay-step5.md)) witnessed
`failed` becoming `provider_error`, and no other row.

_(This paragraph said "Notch Pay produces three codes" and that `canceled` is
`provider_error` rather than `payer_declined`, from the 2026-10-20
transcription of RFC-0012 § 4, until 2026-10-21. The mapping, the conformance
case `…0f02` and `PRODUCED_FAILURE_CODES` had already moved `canceled` to
`payer_declined`; the tables above now say so too. It said "four codes", and
that `provider_unavailable` is a dash on Notch Pay because a `503` is never a
decline, until 2026-10-23: ADR-0032's `prepare` made the `409` with no prior
submit a `provider_unavailable` decline, and `PRODUCED_FAILURE_CODES` did not
declare it until review of
[#345](https://github.com/vaam-apps/vpay/pull/345) found the gap.)_

**† `SENDER_ACCOUNT_NOT_ACTIVE` is not in MTN's published `ErrorReason` enum**,
nor is `COULD_NOT_PERFORM_TRANSACTION`. Both are mapped and both are declared
in `vpay_adapter_mtn_momo::UNPUBLISHED_REASONS`; see
[adapter-mtn-momo.md](adapter-mtn-momo.md) § Failure mapping.

## `provider_error` is an alert, not a resting place

A rising `provider_error` rate means an adapter's mapping table has drifted
behind the rail's actual error strings. Alert on it. Do not tolerate it.

## Adapter mappings

Each adapter's mapping lives in its own flow doc:
[MTN](adapter-mtn-momo.md) · [Orange](adapter-orange-money.md) ·
[Notch Pay](adapter-notchpay.md).

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
- **Notch Pay** (2026-10-20) maps the eleven statuses Notch Pay documents in
  `vpay_adapter_notchpay::mapping` (`every_documented_status_maps_as_rfc_0012_says`,
  `every_status_the_client_names_has_a_row`,
  `a_status_notch_pay_never_documented_is_not_guessed_at`,
  `the_declared_codes_are_exactly_what_the_mapping_can_produce`), and the
  conformance suite's `notchpay` row drives five of them against WireMock.
  ~~**None has been read from Notch Pay itself.**~~ _(Struck 2026-10-22: wrong
  since 2026-10-21, when step 5 read Notch Pay's real sandbox through the
  adapter ([verification](../status/verification/2026-10-21-notchpay-step5.md)).
  Its reads mapped to `Pending`, `Succeeded` and, from `failed`,
  `Failed { provider_error }`, and a `complete` short of the charge's amount
  was refused as `Malformed`. No other failure row was read.)_
- **Over the wire**, both are proven by the shared conformance case
  `a_declined_charge_maps_to_the_documented_failure_code`, which drives a
  real `wiremock/wiremock` container per rail and asserts the taxonomy code
  the documented decline arrives as — **and, since 2026-09-10, that the
  rail's own word survives into `failure_raw`**, which the previous
  `!raw.is_empty()` did not: it passed for a table in which two reasons'
  stubs had been transposed. **Every mapped reason now has a case**, thirteen
  on MTN and two on Orange, where three and one had one before. Measured
  2026-09-10: 53 conformance tests, 53 passed, 0 ignored.
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
  generic message are public. **Since 2026-09-10 it also emits one
  `payment_intent.payment_failed`, inside that same transaction**
  ([issue #57](https://github.com/vaam-apps/vpay/issues/57)), so a merchant
  who only listens to webhooks hears about a decline made at _submit_ and not
  only about one the poll ladder found. Both paths use the same type on
  purpose — see [webhooks.md](webhooks.md).

**Updated 2026-09-10 (exp48, [issue
#59](https://github.com/vaam-apps/vpay/issues/59)): the taxonomy is now
checked against MTN's published vocabulary, not only against this
repository's own transcription of it.** MTN's `ErrorReason.code` enum
lists seventeen codes; the adapter mapped nine and had never been compared
against the list. Three became new rows (`PAYMENT_NOT_APPROVED` and
`APPROVAL_REJECTED` → `payer_declined`, `EXPIRED` → `payer_timeout`), four
stay `provider_error` with a written reason each, and two rows this
repository maps turn out **not** to be published by MTN at all. See "Which
rail can produce which code" above.

**`provider_error` is still the escape hatch, and it is now reachable from a
real response path** — an unmapped string arrives as `provider_error`
carrying the raw reason rather than being guessed at
([runbooks/provider-error-rate.md](../runbooks/provider-error-rate.md)).
**What none of this proves** is that the tables are faithful to the _rails_:
every decline above came from WireMock, and no real rail has ever declined a
vpay charge. _(Narrowed 2026-10-22: on 2026-10-21 Notch Pay's real sandbox
reported one vpay charge `failed`, mapped to `provider_error`. That is a
rail-reported failure, but `provider_error` is the catch-all, so it tests no
mapped reason: no real rail has yet declined a vpay charge with a reason these
tables map.)_ Valantine Suh (@Valsuh45), who performed the 2026-09-15 MTN sandbox tests, confirmed on 2026-10-20 that **over seven payments succeeded**. The initial
documented session had three attempts: the first two stopped at the token mint
on adapter bugs and the third settled its charge. Those three attempts are
not the day's total, and no rail-reported decline was observed
([../status/verification/2026-09-15.md](../status/verification/2026-09-15.md)).
Orange's has never been called. _(This read "neither rail's real sandbox has
ever been called" until 2026-09-23, and had been wrong about MTN since
2026-09-15. The three-run description was narrowed to its session on
2026-10-20.)_ Orange in particular documents no error vocabulary for
`webpayment` and no sub-reasons for `FAILED`, so both land in the
"unmapped, alert on it" bucket by design. See [../status.md](../status.md).
