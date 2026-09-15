# Adapter: MTN MoMo Cameroon

**Flow: push.** `supports_refunds: true` (via Disbursements).

## Preconditions

| Precondition                             | MTN                                                    |
| ---------------------------------------- | ------------------------------------------------------ |
| Caller supplies its own reference        | **Yes** — `X-Reference-Id`. It _is_ the transaction id |
| Final status queryable by that reference | **Yes** — `GET /collection/v1_0/requesttopay/{ref}`    |

Both hold, which is why MTN is a safe push rail.

## Credential hierarchy

Confusing these three is the most common onboarding bug.

1. **Subscription Key** (`Ocp-Apim-Subscription-Key`) — from the developer
   portal, **different per product** (Collections vs Disbursements).
2. **API User + API Key** — created once via `POST /v1_0/apiuser` (you supply a
   UUID and a `providerCallbackHost`) then `POST /v1_0/apiuser/{uuid}/apikey`.
3. **Access token** — `POST /{product}/token/` with HTTP Basic, `expires_in:
3600`, and a **JSON** body of `{"grant_type":"client_credentials"}` (see
   "The token grant" below — the spelling is load-bearing). Collections and
   Disbursements have **separate tokens**, hence the `scope` column on cached
   tokens.

**What vpay configures, per product** (`config/application.yml`,
2026-09-15). All three Disbursements keys are explicit and **none falls back
to its Collections twin** — sending Collections' Basic password to the
Disbursements mint is a 401 that reads as "MTN refused our partner
credentials", pages, and names nothing that is actually wrong.

| Product       | Subscription key                            | API user                         | API key                            |
| ------------- | ------------------------------------------- | -------------------------------- | ---------------------------------- |
| Collections   | `credentials.subscription_key`              | `settings.api_user`              | `credentials.api_key`              |
| Disbursements | `credentials.disbursement_subscription_key` | `settings.disbursement_api_user` | `credentials.disbursement_api_key` |

None of the Disbursements keys is in
`vpay_config::config::REQUIRED_RAIL_KEYS`: requiring them would stop every
existing deployment booting in order to enable a call none of them can make.
An empty value is a missing one, and `refund` answers
`ProviderError::Config` naming it.

**Whether MTN in fact issues one API user across both product subscriptions
is unverified** — it may well, in the sandbox — so an operator repeating the
same UUID and key in both halves is expected and harmless. Two mechanisms
keep the two bearers apart even then, and the review of 2026-09-15 corrected
which of them is load-bearing:

1. **The cache is two named fields, one per product**, and `Adapter::slot` is
   a match on the product through which both the read and the write go. A
   Collections entry is therefore never in the slot a `transfer` reads. This
   is the primary guard.
   (`a_products_bearer_is_stored_where_only_that_product_can_read_it`.)
2. **`Product` is part of the credential fingerprint**, so identical
   credentials still produce two distinct cache keys. This is defence in
   depth — it is what would carry the guarantee on the single-slot or map
   design this adapter rejected.
   (`a_collections_bearer_is_never_served_to_a_disbursement`.)

This page said (2) was the whole of it. Measured on 2026-09-15: removing the
discriminator, collapsing the slots, or doing both at once each leaves **all
67 conformance cases green**, because that suite configures a different
subscription key and API key per product and so never builds the
copy-pasted-configuration case the guards exist for. Those two unit tests are
the entire evidence for this paragraph; the container suite is not evidence
for it at all.

## The token grant

```http
POST /collection/token/          (or /disbursement/token/)
Authorization: Basic base64(api_user:api_key)
Ocp-Apim-Subscription-Key: <that product's key>
Content-Type: application/json

{"grant_type":"client_credentials"}
```

**The body and its content type are load-bearing, and this is the one thing on
this page a real call proved.** PR #177, on MTN's real sandbox on 2026-09-15:
a bodyless POST answers `411 Length Required`, and the same grant sent
**form-encoded** (`grant_type=client_credentials`) answers a `200` "Request
Rejected" HTML page from MTN's gateway. The sibling rail,
`vpay-adapter-orange-money`, posts the form-encoded spelling, and the two must
not be assumed interchangeable.

The conformance stubs enforce it with `equalToJson` plus a `Content-Type`
matcher. They matched `{"contains": "client_credentials"}` until 2026-09-15 —
a pattern the _form_ body also satisfies, so the exact regression #177 fixed
would have gone green. Measured on 2026-09-15 with the adapter posting the
form spelling: under the old matcher **67 of 67 conformance cases passed**;
under the tightened one **24 fail**
([status/verification/2026-09-15-mtn-disbursements-refund.md](../status/verification/2026-09-15-mtn-disbursements-refund.md)).

**The Disbursements mint is assumed to behave the same way. Nobody has called
it.**

## The transfer call (the refund)

```http
POST /disbursement/v1_0/transfer
Authorization: Bearer <disbursement-scoped token>
Ocp-Apim-Subscription-Key: <disbursements key>
X-Target-Environment: sandbox | mtncameroon
X-Reference-Id: <the refund's provider_reference_id>

{ "amount": "5000", "currency": "XAF", "externalId": "<the same reference>",
  "payee": { "partyIdType": "MSISDN", "partyId": "237600000200" },
  "payerMessage": "…", "payeeNote": "…" }
```

Returns **202 with an empty body**, exactly as `requesttopay` does.

`payee`, not `payer` — the one structural difference from the collection body,
and the one that decides who gets the money. `payerMessage` and `payeeNote`
are documented by MTN and deliberately not sent: the port carries no
merchant-supplied text, and a constant string would put words nobody chose in
front of a payee. No `X-Callback-Url` either: the callback route parses a
_charge_ reference, so a Disbursements notification would be refused as
`Malformed` on every delivery.

`partyId` is the digits-only canonical form
`vpay_provider::RefundTarget::mobile_money` produces from
`destination[mtn_momo][msisdn]` — the same spelling `payer.partyId` takes on
the charge path. The adapter neither re-normalises nor re-validates it; the
constructor is the only way to obtain a `RefundTarget` at all.

| HTTP                         | →                                                                                                   |
| ---------------------------- | --------------------------------------------------------------------------------------------------- |
| `202`                        | `Refunded { ref_extra: {}, fee: None }` — **accepted, not settled**                                 |
| `409 RESOURCE_ALREADY_EXIST` | the same, and that is what makes a crash-retry safe rather than a second payout                     |
| `400`                        | `Rejected`, through the same failure table as the charge path (`PAYEE_NOT_FOUND` → `invalid_payee`) |
| `401` / `403`                | `Rejected { provider_account_blocked }` — our **Disbursements** credentials, and it pages           |
| `404`                        | `Config` — the endpoint, most likely a base URL with no Disbursements product behind it             |
| `500` with a config code     | `Config`, on the charge path's three-code table; otherwise `Transport`                              |
| any other 5xx                | `Transport`                                                                                         |
| any 3xx                      | `Malformed` — redirects are never followed, and this body carries the payee's number                |

**Three things about this call are unsettled, and are recorded rather than
decided for a handler that does not exist yet.**

1. **Which reference it carries.** The port hands `refund` one `ChargeRef`,
   and a refund needs its own rail reference — `refunds.provider_reference_id`
   (migration `0017`), minted before any rail call (RFC-0003 § 3 step 2). The
   adapter uses the reference it is given. If a future `POST /v1/refunds`
   passes the **charge's**, every partial refund after the first reuses a
   reference MTN has already seen, is answered `409`, and is reported
   **accepted with no money moved**. RFC-0003 carries it as an open question;
   `the_transfer_is_addressed_by_the_reference_the_core_supplied` pins what
   the adapter does.
2. **A 202 is accepted, not settled.** `transfer` is asynchronous like
   `requesttopay`, and its outcome is read from
   `GET /disbursement/v1_0/transfer/{referenceId}` — a call vpay does not
   make, because the port has no refund status read and `Refunded` has no
   status field. A write path that marked a refund `succeeded` on an `Ok`
   would be asserting something no MTN response has said.
3. **The failure vocabulary is Collections'.** MTN's portal serves **no
   OpenAPI schema document for the Disbursement API** (re-checked
   2026-09-11), so there is nothing to compare a Disbursements-specific table
   against. The adapter reuses `mapping::failure_code`, and a reason it does
   not know falls to `provider_error` carrying the rail's own word.

**RFC-0003 open question 6, decided here** because the RFC left it to "the
first adapter to make a real transfer call": a `Required` rail handed
`destination: None` answers `ProviderError::Config`. Not `Rejected` (no rail
was asked), not `Malformed` (there is no answer), not `Unsupported` or
`NotImplemented` (both lies). `Config` stops the poll ladder, pages, and never
reaches a payer as a decline.

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

## The account-holder call

```http
GET /collection/v1_0/accountholder/msisdn/{msisdn}/basicuserinfo
Authorization: Bearer <token>
Ocp-Apim-Subscription-Key: <collections key>
X-Target-Environment: sandbox | mtncameroon
```

```json
{ "given_name": "…", "family_name": "…", "birthdate": "…",
  "locale": "…", "gender": "…", "status": "…" }
```

**The same Collections subscription key and the same token scope as
`requesttopay`** — which is what makes this buildable where `refund` is not:
no deployment needs a credential it does not already hold. Transcribed from
the operation's own OpenAPI components on MTN's developer portal
(`GetBasicUserinfo`), retrieved 2026-09-05; the exact citation is in
[account-holder-lookup.md](account-holder-lookup.md) and in
`docs/plans/issue-47-notes/impl.md`.

**vpay reads two of those six fields.**
`vpay_adapter_mtn_momo::wire::BasicUserInfo` deserialises `given_name` and
`family_name` and has no home for the rest, so serde drops `birthdate`,
`locale`, `gender`, `status` and anything MTN adds later, at the first point
the bytes become a Rust value. The port returns a
`vpay_provider::AccountHolder`, which carries a name and nothing else and
whose `Debug` redacts even that. [account-holder-lookup.md](account-holder-lookup.md)
is the policy; this is the wire.

| HTTP                                             | →                                                                                                                 |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| `200` with a `given_name` and/or a `family_name` | `Ok(Some(AccountHolder))`                                                                                         |
| `200` with neither                               | `Malformed` — an answer we cannot act on, and **not** `Ok(None)`                                                  |
| `404`                                            | `Ok(None)` — the rail has no record. **See the note below: MTN does not document this status for this operation** |
| `401` / `403`                                    | `Rejected { provider_account_blocked }` — our own credentials, and it pages                                       |
| `400`                                            | `Malformed` naming the status — our request, not the rail's health                                                |
| `500`                                            | the same three-configuration-codes table `requesttopay` uses; otherwise `Transport`                               |
| any other 5xx                                    | `Transport`                                                                                                       |
| any 3xx                                          | `Malformed` — redirects are never followed                                                                        |

**Two things about this endpoint are unverified and are recorded rather than
smoothed over.** First, the `accountHolderIdType` path segment: MTN's portal
declares the parameter's values **upper-case** (`MSISDN | Email | Alias |
ID`) while every published example spells the segment **lower-case**, and
vpay sends lower-case (`vpay_adapter_mtn_momo::ACCOUNT_HOLDER_ID_TYPE`, one
constant, so changing the answer is one edit). Second, the `404`: the portal
documents only `200`, `401` and `500` for `GetBasicUserinfo`, where it
documents "404 Resource not found" explicitly for
`RequesttoPayTransactionStatus`. Mapping it is a deliberate assumption, safe
in the direction that matters — if MTN never sends one the arm is dead code,
and if it sends one for another reason a caller's fail-closed rule still
refuses. **Neither has been checked against MTN's real sandbox, because
nothing in this repository has ever called it.**

## Failure mapping

**Where these strings come from, since 2026-09-10.** This table used to be a
reconstruction with nine rows and no stated source. It is now checked against
MTN's own vocabulary: the `ErrorReason.code` enum in the Collection API's
OpenAPI components document, retrieved 2026-09-10 by the same unauthenticated
portal read [issue #47's notes](../plans/issue-47-notes/impl.md) document for
`GetBasicUserinfo`:

```text
GET https://momodeveloper.mtn.com/developer/apis/Collection/schemas/\
    668d4753d54e6119240c675d?api-version=2022-04-01-preview
```

It enumerates **seventeen** codes. The comparison, row by row, is in
[exp48's notes](../plans/exp48-failure-codes-notes/opus.md); the seventeen are
snapshotted in `vpay_adapter_mtn_momo::mapping`'s tests as
`PUBLISHED_ERROR_REASONS`, and a code that is neither mapped below nor
deliberately unmapped fails `every_published_reason_is_mapped_or_deliberately_not`.

**MTN binds this enum to the response this adapter polls**, so the rows below
are a citation and not an inference. In the same document
`RequestToPayResult.reason` is `$ref: "#/components/schemas/ErrorReason"`, and
the portal's `RequesttoPayTransactionStatus` operation
(`GET /v1_0/requesttopay/{referenceId}`) answers `RequestToPayResult` on its
`200`, described as "note that a failed request to pay will be returned with
this status too … the 'reason' field can be used to retrieve a cause in case
of failure". Two of that response's worked examples carry `PAYER_NOT_FOUND`
and `PAYEE_NOT_FOUND` in `reason.code`.

_(This paragraph said the reverse when the table was re-grounded on
2026-09-10 — that `ErrorReason` was "the whole Collection API's error schema,
not `requesttopay`'s", that MTN "publishes no per-operation subset", and that
each new row was a deliberate assumption in the safe direction. Re-retrieved
and corrected 2026-09-11: the schema and the operation document both say
otherwise. The rows were right; the reason given for them understated the
evidence, and understating it is what would let someone revert
`PAYMENT_NOT_APPROVED` as a guess.)_

What is **not** published, and remains the real caveat, is any statement that
a given reason will ever actually arrive: MTN types the field, it does not
promise the values. Nothing in this repository has called MTN.

| MTN `reason`                      | → core code                                      | Conformance case |
| --------------------------------- | ------------------------------------------------ | ---------------- |
| `NOT_ENOUGH_FUNDS`                | `insufficient_funds`                             | `…0f01`          |
| `COULD_NOT_PERFORM_TRANSACTION` † | `payer_timeout` (PIN not entered, ~5 min)        | `…0f02`          |
| `EXPIRED`                         | `payer_timeout` (the prompt's own window closed) | `…0f04`          |
| `PAYMENT_NOT_APPROVED`            | `payer_declined`                                 | `…0f05`          |
| `APPROVAL_REJECTED`               | `payer_declined`                                 | `…0f06`          |
| `PAYER_NOT_FOUND`                 | `invalid_payer`                                  | `…0f07`          |
| `PAYER_LIMIT_REACHED`             | `payer_limit_reached`                            | `…0f08`          |
| `SENDER_ACCOUNT_NOT_ACTIVE` †     | `payer_account_blocked`                          | `…0f09`          |
| `PAYEE_NOT_FOUND`                 | `invalid_payee`                                  | `…0f0a`          |
| `PAYEE_NOT_ALLOWED_TO_RECEIVE`    | `payee_account_blocked`                          | `…0f0b`          |
| `NOT_ALLOWED`                     | `provider_account_blocked`                       | `…0f03`          |
| `SERVICE_UNAVAILABLE` / 503       | `provider_unavailable`                           | `…0f0c`          |
| anything else                     | `provider_error` + raw reason                    | `…0f0d`          |

The case column is the reference a WireMock mapping in
`backends/tests/conformance/wiremock/mtn/mappings/requesttopay-status.json`
keys on; `a_declined_charge_maps_to_the_documented_failure_code` drives every
one of them against a real container and asserts both the code **and** that
the rail's own word survives into `failure_raw`. Until 2026-09-10 three of the
nine rows had a case and the rest were transcription.

**† Two rows are not in MTN's published enum**, and are kept because this
document and these stubs have carried them since Step 3 — dropping them would
turn two mapped declines back into `provider_error`. Nothing MTN publishes
confirms either: they are absent from `ErrorReason` — the enum MTN types
`RequestToPayResult.reason` with, i.e. the exact field this adapter reads —
and from every schema the portal serves for the Collection and Remittance
APIs (the Disbursements API publishes none). Re-checked 2026-09-11. They are declared in
`vpay_adapter_mtn_momo::UNPUBLISHED_REASONS` rather than left to pass for
documented, and `a_reason_mtn_does_not_publish_is_declared_as_such` fails if
that list and this table disagree.

**Four published codes stay `provider_error` on purpose**
(`vpay_adapter_mtn_momo::UNMAPPED_REASONS`, which also carries the three
`CONFIGURATION_CODES`):

| Code                        | Why not mapped                                                                                                                                                                                                                                                                                                                                          |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `INTERNAL_PROCESSING_ERROR` | On a 500 it is `Transport` and the poll ladder resolves it. On a terminal `FAILED` there is nothing left to retry and nothing to tell a payer — which is what `provider_error` means                                                                                                                                                                    |
| `RESOURCE_NOT_FOUND`        | About the _reference_, not the payment. The status query answers it as HTTP 404 → `ChargeStatus::NotFound`, which is the whole recovery story                                                                                                                                                                                                           |
| `RESOURCE_ALREADY_EXIST`    | The duplicate-reference answer, handled as HTTP 409 → `Submitted`. As a `FAILED` reason it would be MTN contradicting itself                                                                                                                                                                                                                            |
| `TRANSACTION_CANCELED`      | **The one genuinely open row.** `payer_declined` would read well, but MTN publishes no description saying _who_ cancels, and the Collection API's only cancel operations are `CancelInvoice` and `CancelPreApproval`, neither of which vpay calls. Guessing "the payer" would put a sentence in front of a buyer on the strength of a verb. **Ask MTN** |

HTTP: `409 RESOURCE_ALREADY_EXIST` on a duplicate reference — **the adapter must
report this as `Submitted`**. `404` → `NotFound`, never a failure.

**MTN's biggest wart: several _logical_ errors return HTTP 500** —
`INVALID_CURRENCY`, `NOT_ALLOWED_TARGET_ENVIRONMENT`, `INVALID_CALLBACK_URL_HOST`,
and an `INTERNAL_PROCESSING_ERROR` that can mean insufficient funds _or_ the
wallet platform being down. Parse the body's `code` before deciding anything;
never treat 500 as blind-retry.

## Environment values (all just config)

|                      | Sandbox                                                        | Cameroon production                              |
| -------------------- | -------------------------------------------------------------- | ------------------------------------------------ |
| `base_url`           | `https://sandbox.momodeveloper.mtn.com` ✅ (called 2026-09-15) | `https://proxy.momoapi.mtn.com` — **confirm**    |
| `target_environment` | `sandbox` ✅ (called 2026-09-15)                               | `mtncameroon` — **confirm; subsidiary-specific** |
| `currency`           | **EUR only** ✅ (the 2026-09-15 payment was EUR)               | XAF                                              |

## Status

**Updated 2026-09-15 (RFC-0003 § 5, arm D): `refund` is written, and MTN's
Disbursements product has still never been called.** The
`ProviderError::NotImplemented("mtn_momo::refund")` token is retired —
`verify-status` now prints **1 unimplemented item**, `orange_money::refund`,
which RFC-0003 § 5 added the same day — and that is a statement about tokens,
not about a rail. See "`refund` IS implemented … and it has
never been called" below, "The transfer call" above for the wire, and
[../status.md](../status.md), which says the same thing in the same words.
The same change tightened both token stubs from
`{"contains": "client_credentials"}` to `equalToJson` plus a `Content-Type`
matcher: the loose pattern was also satisfied by the **form-encoded** body PR
#177 had just proved MTN rejects, so the regression #177 fixed would have gone
green. Mutation numbers are in "The token grant" above.

`submit`, `query_status` and `parse_callback` are implemented and proven
against a real `wiremock/wiremock` container by the shared conformance suite
(`backends/tests/conformance/tests/adapter_conformance.rs`, mappings in
`backends/tests/conformance/wiremock/mtn/mappings/`). The failure table above
is transcribed into `mapping::FAILURE_REASONS` and every row is asserted, in
both directions, by a unit test.

**Updated 2026-09-15 (the first real call): `submit`, `query_status` and the
token mint were exercised against MTN's **real sandbox** and a payment
settled.** A EUR `mtn_momo` PaymentIntent (`pi_xxd2xj1e914e16c6m63gezag`) was
created, confirmed and reached `succeeded` through the worker's authenticated
status query. That run also found and fixed a real bug the WireMock suite
could not see: the token mint posted **no body** to `POST /collection/token/`,
and MTN's gateway answered `411 Length Required`; when the grant was then sent
form-encoded (`grant_type=client_credentials`) the gateway answered a 200
"Request Rejected" HTML page, so the body must be the **JSON** spelling
`{"grant_type":"client_credentials"}` with `Content-Type: application/json`.
The conformance stub now enforces that body
(`backends/tests/conformance/wiremock/mtn/mappings/token.json`).

**Updated 2026-09-10 (exp48, [issue
#59](https://github.com/vaam-apps/vpay/issues/59)).** The table went from nine
rows to twelve and, for the first time, was compared against MTN's own
`ErrorReason.code` enum instead of only against itself — see "Failure
mapping" above for the citation and for the two rows MTN does not publish.
`PAYMENT_NOT_APPROVED` and `APPROVAL_REJECTED` → `payer_declined` is the
change that matters outside this crate: that code had buyer copy in
`examples/shop`, a type in both merchant SDKs and a place in
`FailureCode::ALL`, and no adapter emitted it. **MTN now reaches all eleven
codes** (`vpay_adapter_mtn_momo::PRODUCED_FAILURE_CODES`, pinned by
`mtn_reaches_every_code_the_core_defines`), and every mapped reason has a
conformance case where three of nine had one before. A new demo MSISDN,
`237600000103`, reaches `payer_declined`: proven by
`a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin`, which drives it
through `submit` and `query_status` against a real WireMock container. **No
browser has typed it** — `checkout.cy.ts` drives the hex family, and
`just demo-walk` does not send this number at all (see
[../runbooks/demo.md](../runbooks/demo.md) § "The test numbers"). It is a
number a payer _can_ type that the adapter is proven to honour, which is not
the same claim as an exercised browser path.

**`refund` IS implemented as of 2026-09-15 (RFC-0003 § 5), and it has never
been called.** Those are two separate facts and neither one is the other.
MTN refunds are the _Disbursements_ product — a different subscription key, a
separately-scoped token and a `transfer` call — and the adapter now makes that
call: `POST /disbursement/v1_0/transfer`, addressed to the payee the merchant
nominated in `destination[mtn_momo][msisdn]`. The `NotImplemented` token is
retired, `verify-status` prints zero items, and **none of that is the claim
"MTN refunds work".**

**No deployment of this system holds a Disbursements subscription key**, and
**nothing in this repository has ever called MTN's Disbursements product** —
not in production, not against the sandbox, not once.
`config/application.yml` carries `disbursement_subscription_key`,
`disbursement_api_key` and `disbursement_api_user`; every deployment leaves
them empty, and `refund` answers `ProviderError::Config` naming the first one
that is missing. `POST /v1/refunds` is still unrouted, so no caller can reach
it either way. `supports_refunds` stays `true` for the reason it always did:
the _rail_ refunds, and answering `Unsupported` would be a lie about MTN.

See "The transfer call" above for the wire and "Not proven" below for what a
first real call has to check.

**`account_holder_name` IS implemented** (issue #47, 2026-09-05), and
`supports_account_holder_lookup` is `true` — a claim about the rail _and_
about this code. Five conformance cases run it against a real WireMock
container, parameterised over both rails out of one body
(`an_account_holder_lookup_returns_a_name_and_nothing_else`,
`a_number_the_rail_has_no_record_of_is_not_an_error`,
`a_lookup_that_cannot_reach_the_rail_is_never_reported_as_a_missing_holder`,
`an_oversized_account_holder_body_is_refused_at_the_cap`,
`an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing`) —
the last of those asserting against **captured `tracing` output** that
neither the holder's name nor the payer's number reaches a log line. The
same "never against the real sandbox" caveat below applies, and two specific
things about the endpoint are unverified: see "The account-holder call"
above.

### What the token cache does

One in-memory bearer per `Adapter`, minted from `POST /collection/token/`,
treated as expired a minute before MTN's `expires_in` says. It is keyed by a
SHA-256 fingerprint of `subscription_key` + **`api_key`** + `api_user`, each
field length-prefixed so a boundary cannot be shifted into a collision
(`a_field_boundary_cannot_be_shifted_into_a_collision`). A second merchant's
configuration passed to the same adapter therefore mints its own token
instead of reusing the first's — the port hands `&ProviderConfig` per call,
so that is a real cross-tenant path, not a hypothetical one
(`a_second_configuration_never_reuses_the_first_configurations_token`).

**`api_key` is in the fingerprint because it is the token's password.**
Leaving it out — as this did until the Step 3 security review — meant a
deployment that rotated only the API key kept serving calls with the bearer
minted from the _old_ one, until the cached token aged out (up to an hour)
or the rail answered 401. A key is rotated precisely when it must stop
working immediately. Hashing it is safe because the cache key is a SHA-256,
never the credential (`different_credentials_fingerprint_differently`,
`rotating_only_the_secret_evicts_the_cached_bearer` on the Orange twin).

A 401 re-mints exactly once and then reports `provider_account_blocked`;
nothing else is ever retried, and a 500 is never retried at all.

Neither the token nor the credentials can reach a log: `Debug` on the
adapter, on `Credentials` and on `CachedToken` all redact
(`debugging_the_adapter_does_not_print_the_token`,
`debugging_credentials_does_not_print_them`,
`debugging_a_token_does_not_print_it`).

### The callback is unsigned and unauthenticated

MTN signs nothing and sends no shared secret. Anyone who can reach the
callback URL can post anything to it, so `parse_callback` returns identifiers
only and the body's `status` is deliberately not read: the authenticated
status query is the only thing that moves money
([reconciler.md](reconciler.md)).

The reference is recovered from `referenceId` when MTN echoes it, and
otherwise from `externalId`. `externalId` works because it is what _we_ set on
submit: `ChargeRef` carries no charge id — `reference_id` is the only
identifier the port gives an adapter — so the "charge id" in the request body
above is that reference, rendered. A body with neither field is refused as
`Malformed` rather than guessed at.

`ChargeRef::return_url` is ignored, and that is asserted rather than assumed
(`a_return_url_is_not_carried_on_a_push_rails_body` in `wire.rs`, and the
push half of `the_submit_tells_the_rail_where_to_send_the_payer_back` in the
conformance suite, which checks the stub's whole request journal). The core
fills the field for any charge whose merchant sent a URL and leaves it to
each adapter to decide whether its rail has a use for one; `requesttopay`
has no browser step, and a field MTN does not document would at best be
dropped.

### What the transport refuses, and why

Every call this adapter makes goes through `vpay_provider::http`, so it
inherits three refusals that are not MTN-specific:

- **Redirects are returned, never followed.** A 3xx from a rail host is an
  answer to look at, not a hop to take — following one would let a
  compromised or misconfigured DNS entry move an authenticated payment
  request to another host (`redirects_are_refused_and_never_followed`,
  conformance, both rails).
- **`HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` are ignored.** A payment
  gateway's own egress is not a merchant's corporate network. (The merchant
  SDK's copy of this client deliberately keeps proxy support.)
- **Response bodies are capped at 256 KiB** (`bounded_body`,
  `MAX_RAIL_BODY_BYTES`) rather than read to end of stream, so a load
  balancer's HTML error page or a captive portal cannot choose how much
  memory a worker task allocates. Proven live by
  `an_oversized_rail_body_is_refused_at_the_cap`; the truncation of a body
  that _does_ fit but is long is proven by
  `a_rails_error_body_is_bounded_before_it_reaches_a_message`.
- Each request carries `ProviderConfig::request_timeout` explicitly, because
  one `reqwest::Client` is shared across rails and a client-level deadline
  could only ever be one rail's.

### Not proven

- **Until 2026-09-15 nothing here had ever called MTN; one real sandbox call
  has now been made, and it was on Collections.** Every wire assertion in this
  document is against a `wiremock/wiremock` container except the live-sandbox
  run of 2026-09-15 (Status above), which exercised `submit` and
  `query_status` against `https://sandbox.momodeveloper.mtn.com` and settled a
  payment. Both **confirm** rows in the environment table above are still
  unconfirmed, and a mapping faithful to this document but not to MTN would
  pass.
- **MTN's Disbursements product has never been called at all** — not by the
  2026-09-15 run, which was a Collections payment, and not since. So every
  line of "The transfer call" above, and the Disbursements half of "The token
  grant", is a transcription of MTN's published `Transfer` operation rather
  than an observation, and the portal serves **no OpenAPI schema document for
  that API** to have compared it against. Specifically unverified, and what a
  first real call has to check:
  - that `POST /disbursement/token/` wants the same JSON grant and
    `Content-Type` that Collections' mint does;
  - that the transfer path is `/disbursement/v1_0/transfer` and the member is
    spelled `payee`;
  - that a `202` really is empty and really carries no fee, which is what
    keeps `Refunded::fee` at `None` (issue #46);
  - whether `409 RESOURCE_ALREADY_EXIST` is the duplicate-reference answer on
    this product as it is on Collections — the whole crash-retry story rests
    on it;
  - whether MTN in fact issues one API user across both product
    subscriptions, which decides whether requiring three separate
    Disbursements keys is prudence or friction.
- **`refund`'s conformance cases prove the adapter, not the rail.** Seven of
  them run against a real container, and the strongest — that the transfer
  carries a bearer minted from `/disbursement/token/` and the per-product
  subscription key — works because the stub answers `202` for nothing else. A
  stub written from the same documentation as the adapter cannot disagree
  with it.
- **The conformance suite cannot see the credential-separation guards at
  all**, added on review 2026-09-15. It configures a different subscription
  key and API key per product, so the copy-pasted-configuration case is never
  constructed; removing `Product` from the token fingerprint, collapsing the
  adapter's two cache slots, or both together, each leaves it green at 67/67.
  Two unit tests in `vpay-adapter-mtn-momo` are the whole of that evidence
  and a green container run says nothing about it.
- **The 401 → re-mint → retry path is not covered by a test.** The logic is
  there and is bounded at one retry, but no mapping in the conformance suite
  returns 401 from `requesttopay` after a good token, and the adapter's own
  crate may not stand up an in-process HTTP double (ADR-0006). What _is_
  proven is the 401 on the token endpoint itself
  (`bad_credentials_are_not_reported_as_a_payer_problem`).
- The submit mappings for a 400 (`…0400`), a 500 with a code (`…0500`) and a
  500 with an HTML body (`…05ff`) exist and are correct per this document, but
  no conformance case drives them yet; their outcomes are proven instead by
  `submit_outcome`'s unit tests, which take the same status and body.
- ~~**No callback route exists.**~~ **Corrected 2026-09-04 (Step 8, lane C):
  the callback route exists, and nothing has ever called it but this
  repository's own tests.** `POST /provider/mtn_momo/callback`
  (`vpay_api::provider_callback`) parses this document's notification body into
  identifiers and pulls the charge's poll job forward; MTN signs nothing, so it
  is still a hint and the route writes no charge state.
  `backends/tests/integration/tests/provider_callback.rs` POSTs the body
  transcribed above to the URL MTN was handed on the submit, so **a body
  faithful to this document but not to MTN would pass**.
- **One real call settled a payment, and none of it exercised the decline
  vocabulary — so this bullet is unchanged in substance.** `PAYMENT_NOT_APPROVED`,
  `APPROVAL_REJECTED` and `EXPIRED` are real strings
  from MTN's published enum, and that enum is the declared type of
  `RequestToPayResult.reason` — so the shape and the vocabulary are cited, not
  assumed. What no document can tell us is whether MTN's Cameroon deployment
  ever _emits_ a given one. A stub answering a string MTN may never send
  proves the mapping row, not the rail; the 2026-09-15 sandbox payment reached
  `succeeded` and so proved none of the failure codes against the real rail.

  _(This bullet said the enum "is the whole Collection API's and says nothing
  about which operation returns which code" until 2026-09-11; see "Failure
  mapping" for the correction. The conclusion — that only a real call settles
  this — is unchanged: the one real call so far was a success path, which is
  why "Real sandbox" remains ⛔ for the failure vocabulary even though a
  sandbox payment has now settled.)_

- The crate runs **87 tests, 87 passed, 0 skipped**
  (`cargo nextest run -p vpay-adapter-mtn-momo`, measured 2026-09-15; 62 on
  2026-09-10, 48 on 2026-09-03). The shared conformance suite runs **67, 67
  passed, 0 skipped** (`cargo nextest run -p vpay-tests-conformance`, same
  date, against real `wiremock/wiremock` containers).

## Documentation MSISDNs (steering table)

Every one of these is a WireMock scenario key, not a real subscriber. Where a
row has **two** MSISDNs they enter the **same** scenario by the **same**
mapping (one `matches` regex, not two mappings) and therefore the same walk —
proven by `a_digits_only_msisdn_reaches_the_same_walk_as_its_hex_twin` in
`backends/tests/conformance/tests/adapter_conformance.rs`.

**A row with no hex MSISDN is not an omission.** The hex family predates
`frontends/apps/checkout`'s MSISDN validator and is kept only because it was
already working; a number added since is digits-only, because that is the only
form a payer can type. `237600000103` (2026-09-10) is such a row, as
`237600000503` is — and **both are in the twin test**, which despite its name
is the case that drives each of these numbers against a real WireMock
container and asserts the code and the rail's own reason that come back.

_(This paragraph ended "and neither is in the twin test, because there is no
twin to agree with" when `237600000103` was added on 2026-09-10. That was
false of `237600000503`, which had had a case since exp22, and it was the
stated reason `237600000103` was given none — leaving the only number that
reaches `payer_declined` proven by nothing but this sentence. Corrected
2026-09-11 with the missing case.)_

| Outcome                       | Hex MSISDN (not a valid E.164 number — `examples/merchant-demo`, `checkout.cy.ts`) | Digits-only MSISDN (a real Cameroon E.164 number — `frontends/apps/checkout`) | Scenario                                      | First status query                                           | Second status query             |
| ----------------------------- | ---------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | --------------------------------------------- | ------------------------------------------------------------ | ------------------------------- |
| The payer approves            | `237600000ce0`                                                                     | `237600000100`                                                                | `mtn-e2e-poll` (`requesttopay-scenario.json`) | `PENDING`                                                    | `SUCCESSFUL`                    |
| The payer has no balance      | `237600000f01`                                                                     | `237600000101`                                                                | `mtn-demo-decline` (`demo-outcomes.json`)     | `FAILED` / `NOT_ENOUGH_FUNDS` → `insufficient_funds`         | — (terminal on the first query) |
| The prompt expires unanswered | `237600000f02`                                                                     | `237600000102`                                                                | `mtn-demo-expiry` (`demo-outcomes.json`)      | `FAILED` / `COULD_NOT_PERFORM_TRANSACTION` → `payer_timeout` | — (terminal on the first query) |
| The payer refuses the prompt  | — (digits only)                                                                    | `237600000103`                                                                | `mtn-demo-refused` (`demo-outcomes.json`)     | `FAILED` / `PAYMENT_NOT_APPROVED` → `payer_declined`         | — (terminal on the first query) |

The hex family is what `examples/merchant-demo` (`Steering::Msisdn`) and
`frontends/tests/e2e/cypress/e2e/checkout.cy.ts` (`MTN_E2E_POLL_MSISDN`) send —
both build the confirm request body themselves, never through a phone-number
form, so the letters never have to survive validation. The digits-only family
is what a real payer typing into `frontends/apps/checkout`'s MSISDN field
(`src/lib/msisdn.ts`, Cameroon E.164: `237` + `6` + eight digits) can actually
send — that validator correctly refuses the hex family, as it refuses any other
non-digit input, which is why this table has two columns and not one.
`shop-hosted.cy.ts` types the digits-only ones.

**There is no Orange equivalent, and there is no gap.** Orange Money is a
redirect rail whose submit body carries an `order_id` and no MSISDN, so there
is no payer-typed field on that rail for a documentation number to steer by;
its outcomes are steered by the amount.

**The account-holder MSISDNs are a separate, digits-only family, and
deliberately so.** `basicuserinfo.json` stubs `237600000200` (a named
holder), `237600000404` (no record), `237600000560` (a delayed answer),
`237600000616` (an oversized body) and `237600000700` (a body full of
personal data), plus the demo pair `237600000100` / `237600000199`. None of
them carries a hex letter, because `GET /v1/account_holders` validates
Cameroon E.164 **server-side** and would refuse one before the rail was
called — so a hex steering number is unreachable through that route by
construction, which
`a_number_that_is_not_e164_is_refused_before_the_rail_is_asked` asserts.
`237600000100` is shared with the settling row above on purpose: a merchant's
real question is whether the number they are about to pay belongs to the
person they think it does.

**The demo stack settles both rails in XAF, and this table is unaffected by
that**: no MTN mapping matches on a currency at all, in a request body or
anywhere else, and `vpay_adapter_mtn_momo::wire::StatusResponse` never
deserialises one. `config/application.yml` still puts `mtn_momo` on EUR
because **MTN's real sandbox rejects XAF** ([money.md](money.md)); only the
generated demo overlay diverges, and it says so in its own comment.
