# exp48 — every rail reason maps to a `FailureCode` (issue #59)

**Branch** `claude/exp48-failure-codes`, base `bdbfd2a`. Written 2026-09-10.

What this file is for: the MTN documentation citation the new mapping rows
rest on, the decision taken for every one of MTN's seventeen published
reasons, the places where this work **contradicted its own brief** because the
repository's evidence said otherwise, and the mutations that were run to check
the tests actually fail. The taxonomy itself is
[../../flows/failures.md](../../flows/failures.md); the per-rail tables are
there and in each adapter's flow doc.

---

## 1. The MTN citation

Retrieved **2026-09-10** from MTN's MoMo developer portal,
`https://momodeveloper.mtn.com`, by the method
[issue #47's notes](../issue-47-notes/impl.md) established — the portal is an
Azure APIM single-page app and renders nothing to a fetch, so the definitions
come from the data API that app itself calls:

```text
GET https://momodeveloper.mtn.com/developer/apis?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/operations?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/operations/RequesttoPayTransactionStatus?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/schemas/668d4753d54e6119240c675d?api-version=2022-04-01-preview
```

Unauthenticated reads of the public developer portal. `2022-04-01-preview` is
the only `api-version` the endpoint accepts.

### 1.1 `ErrorReason`, verbatim

From the Collection API's OpenAPI components document
(`schemas/668d4753d54e6119240c675d`):

```json
{ "type": "object",
  "properties": {
    "code": { "type": "string",
              "enum": ["PAYEE_NOT_FOUND", "PAYER_NOT_FOUND", "NOT_ALLOWED",
                       "NOT_ALLOWED_TARGET_ENVIRONMENT",
                       "INVALID_CALLBACK_URL_HOST", "INVALID_CURRENCY",
                       "SERVICE_UNAVAILABLE", "INTERNAL_PROCESSING_ERROR",
                       "NOT_ENOUGH_FUNDS", "PAYER_LIMIT_REACHED",
                       "PAYEE_NOT_ALLOWED_TO_RECEIVE", "PAYMENT_NOT_APPROVED",
                       "RESOURCE_NOT_FOUND", "APPROVAL_REJECTED", "EXPIRED",
                       "TRANSACTION_CANCELED", "RESOURCE_ALREADY_EXIST"] },
    "message": { "type": "string" } } }
```

Seventeen values. Snapshotted in
`vpay_adapter_mtn_momo::mapping`'s tests as `PUBLISHED_ERROR_REASONS`, spelled
out rather than fetched: a test that reached the network would fail on a
train, and the point of the list is to be a snapshot someone re-retrieves
deliberately.

### 1.2 The load-bearing caveat

**`ErrorReason` is the whole Collection API's error schema, not
`requesttopay`'s.** The API has twenty-three operations, including
`PreApproval`, `CreateInvoice`, `CreatePayments` and `RequestToWithdraw`; MTN
publishes no per-operation subset of the enum. So "MTN publishes
`PAYMENT_NOT_APPROVED`" is a fact, and "`requesttopay` returns
`PAYMENT_NOT_APPROVED`" is an inference.

The inference is made deliberately and in the safe direction — the same call
this repository already documents for the `404` on `basicuserinfo`
([adapter-mtn-momo.md](../../flows/adapter-mtn-momo.md)): if MTN never sends
it the row is dead code, and if MTN does send it a payer reads an accurate
sentence instead of `provider_error`. Nothing has ever called MTN, and these
rows do not change that.

### 1.3 Two reasons this repository maps that MTN does not publish

`COULD_NOT_PERFORM_TRANSACTION` and `SENDER_ACCOUNT_NOT_ACTIVE` are **absent
from `ErrorReason`**, and from every schema on the Collection and Remittance
APIs (checked; the Disbursements API publishes no schema through this
endpoint). They have been in `docs/flows/adapter-mtn-momo.md` and in this
repository's stubs since Step 3, and `COULD_NOT_PERFORM_TRANSACTION` is what
the demo's prompt-expiry number produces.

They are **kept** — dropping them would turn two mapped declines back into
`provider_error` — and declared in `vpay_adapter_mtn_momo::UNPUBLISHED_REASONS`
rather than left to pass for documented.
`a_reason_mtn_does_not_publish_is_declared_as_such` fails if that list and the
table disagree in either direction.

This is a finding, not a fix: someone should ask MTN whether those strings are
real. If they are not, the two rows are mapping a string that never arrives,
and the outcomes they name (`payer_timeout` on a real prompt expiry,
`payer_account_blocked`) have no producer at all.

---

## 2. The decision for all seventeen

| MTN reason | → | Where |
|---|---|---|
| `NOT_ENOUGH_FUNDS` | `insufficient_funds` | mapped (was) |
| `PAYER_NOT_FOUND` | `invalid_payer` | mapped (was) |
| `PAYER_LIMIT_REACHED` | `payer_limit_reached` | mapped (was) |
| `PAYEE_NOT_FOUND` | `invalid_payee` | mapped (was) |
| `PAYEE_NOT_ALLOWED_TO_RECEIVE` | `payee_account_blocked` | mapped (was) |
| `NOT_ALLOWED` | `provider_account_blocked` | mapped (was) |
| `SERVICE_UNAVAILABLE` | `provider_unavailable` | mapped (was) |
| **`PAYMENT_NOT_APPROVED`** | **`payer_declined`** | **new** |
| **`APPROVAL_REJECTED`** | **`payer_declined`** | **new** |
| **`EXPIRED`** | **`payer_timeout`** | **new** |
| `INTERNAL_PROCESSING_ERROR` | `provider_error` | deliberate |
| `RESOURCE_NOT_FOUND` | `provider_error` | deliberate |
| `RESOURCE_ALREADY_EXIST` | `provider_error` | deliberate |
| `TRANSACTION_CANCELED` | `provider_error` | deliberate — **open question** |
| `INVALID_CURRENCY` | `provider_error` (a 500 → `Config`) | deliberate |
| `NOT_ALLOWED_TARGET_ENVIRONMENT` | `provider_error` (a 500 → `Config`) | deliberate |
| `INVALID_CALLBACK_URL_HOST` | `provider_error` (a 500 → `Config`) | deliberate |
| *(unpublished)* `COULD_NOT_PERFORM_TRANSACTION` | `payer_timeout` | mapped, §1.3 |
| *(unpublished)* `SENDER_ACCOUNT_NOT_ACTIVE` | `payer_account_blocked` | mapped, §1.3 |

Reasons for the "deliberate" rows are in
`vpay_adapter_mtn_momo::UNMAPPED_REASONS`'s doc comment, one paragraph each.
`every_published_reason_is_mapped_or_deliberately_not` fails if a published
code is in neither list or in both.

### `TRANSACTION_CANCELED` is left for the maintainer

`payer_declined` would read well and was tempting. It is **not** taken,
because:

* MTN publishes the code and **no description** — nothing says who cancels.
* The Collection API's only cancel operations are `CancelInvoice` and
  `CancelPreApproval`. vpay calls neither. There is no cancel on
  `requesttopay` at all, which is an argument that a cancellation reaching a
  vpay charge would be the payer's — and an argument is not a citation.
* The cost of being wrong is a sentence in front of a buyer ("The payment was
  refused on the handset") describing something they did not do.

It stays `provider_error`, carrying `TRANSACTION_CANCELED` in `failure_raw`,
with conformance case `…0f0d` proving the raw word survives so the row can be
added the day someone asks MTN. **This is a decision reserved for the
maintainer, not one this branch should have made either way.**

---

## 3. Where this contradicted the brief, and why

The brief asked for `payer_declined` "where MTN's `PAYMENT_NOT_APPROVED` **or
Orange's cancel by the payer** arrives". The MTN half is done. **The Orange
half is refused**, on the repository's own evidence:

* `docs/flows/adapter-orange-money.md` documents five statuses — `INITIATED`,
  `PENDING`, `SUCCESS`, `EXPIRED`, `FAILED`. No `CANCELLED`.
* `the_payers_exit_from_the_hosted_page_decides_the_charge` in the conformance
  suite already asserts that a payer clicking **Cancel** on the rail's page
  arrives as `EXPIRED` → `payer_timeout`, and its doc comment says why in as
  many words: "`EXPIRED` for a cancel, and not a `CANCELLED` of this
  repository's own invention".
* `examples/shop`'s README says the same thing to a merchant.

Giving Orange a `payer_declined` would have required inventing a rail status,
which is the one thing the brief forbade in its last line. So Orange is
**unchanged in behaviour**, and instead gains an explicit `cannotExpress` row
saying it cannot produce `payer_declined` and why — a claim now checked
against `PRODUCED_FAILURE_CODES` rather than asserted in prose.

The brief also asked for the reachability table in
`docs/flows/payment-lifecycle.md`. It is there, as a two-rail summary with the
lifecycle-relevant point. The **authoritative** code-by-code table is in
`docs/flows/failures.md`, which is where the taxonomy already lives and what
both adapters, the shop and both SDKs already cite; splitting the two would
have created a second copy to drift.

---

## 4. What "no rail can produce it" turned out to mean

Issue #59 asked for the codes no rail can produce to be marked as such. After
the MTN rows, **there are none** — every one of the eleven has at least one
producer, and the honest answer is that the gaps are *per rail*:

* **MTN** reaches all eleven (`mtn_reaches_every_code_the_core_defines`).
* **Orange** reaches three; the other eight are unreachable, not unmapped
  (`the_codes_orange_cannot_express_are_unreachable_and_not_merely_unmapped`).

So the deliverable is the per-rail table rather than a list of orphan
variants. No variant is deleted: the vocabulary is a wire contract.

The one caveat worth repeating is §1.3's — `payer_account_blocked`'s only
producer is a string MTN does not publish.

---

## 5. Mutations run

| Mutation | Expected | Result |
|---|---|---|
| Swap the codes of `EXPIRED` and `PAYMENT_NOT_APPROVED` in `FAILURE_REASONS` | conformance fails | **FAILED** as required: `a_declined_charge_maps_to_the_documented_failure_code::case_1_mtn_momo`, "EXPIRED (…0f04) mapped to payer_declined; left: PayerDeclined, right: PayerTimeout" |
| Promise `insufficient_funds` on Orange in **both** `README.md` and `test-numbers.ts` (so the existing both-directions check cannot be what catches it) | the shop's reachability check fails | **FAILED** as required: "orange_money 237600000101 promises insufficient_funds, which vpay-adapter-orange-money's PRODUCED_FAILURE_CODES does not list" |

Both were reverted and the suites re-run green.

The `raw.contains(reason)` strengthening in
`a_declined_charge_maps_to_the_documented_failure_code` is what makes the
first mutation decisive at the *reason* level rather than only at the code
level: the previous assertion was `!raw.is_empty()`, which passes for a table
in which two reasons' stubs have been transposed.

---

## 6. Not done

* **Nothing has called MTN or Orange.** Every assertion here is against
  `wiremock/wiremock`. A mapping faithful to this document but not to the rail
  would pass.
* **`TRANSACTION_CANCELED` is unresolved** (§2) and is a maintainer decision.
* **`COULD_NOT_PERFORM_TRANSACTION` and `SENDER_ACCOUNT_NOT_ACTIVE` are
  unverified** (§1.3) and should be put to MTN.
* **No new `xtask` gate.** The three cross-checks added are ordinary tests
  (two Rust, one vitest). Making "a promise must name a producible code" a
  `just verify` gate would cover the frontend copy tables too
  (`frontends/apps/checkout/src/lib/failures.ts`), which this branch did not
  touch.
