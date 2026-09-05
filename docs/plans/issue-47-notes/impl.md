# Issue #47 — account-holder name lookup: implementation notes

**Branch** `claude/issue-47-account-holder`, base `65a5952`. Written
2026-09-05.

What this file is for: the MTN documentation citations the implementation
rests on, the decisions taken and the ones deliberately left open, and the
mutations that were run to check the tests actually fail. The *process* lives
in [../../flows/account-holder-lookup.md](../../flows/account-holder-lookup.md);
this is the working record behind it.

---

## 1. MTN documentation citations

Retrieved **2026-09-05** from MTN's MoMo developer portal,
`https://momodeveloper.mtn.com`. The portal's own pages are an Azure APIM
single-page app and render nothing useful to a fetch, so the operation
definitions were read from the data API that app itself calls:

```text
GET https://momodeveloper.mtn.com/developer/apis?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/operations?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/operations/GetBasicUserinfo?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/operations/ValidateAccountHolderStatus?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/operations/RequesttoPayTransactionStatus?api-version=2022-04-01-preview
GET https://momodeveloper.mtn.com/developer/apis/Collection/schemas/668d4753d54e6119240c675d?api-version=2022-04-01-preview
```

`2022-04-01-preview` is not a guess: any other `api-version` answers
`MissingOrIncorrectVersionParameter` and names it as the only supported one.
These are unauthenticated reads of the public developer portal.

### 1.1 The `Collection` API's own shape

```json
{ "id": "collection", "name": "Collection", "path": "collection",
  "subscriptionRequired": true,
  "authenticationSettings": { "subscriptionKeyRequired": true },
  "subscriptionKeyParameterNames": { "header": "Ocp-Apim-Subscription-Key",
                                     "query": "subscription-key" } }
```

The `path` is what makes the full URL `/collection/v1_0/...` — the same
prefix `requesttopay` already carries in this repository, and the same
`Ocp-Apim-Subscription-Key` header. **This is the load-bearing fact for the
whole issue:** the lookup sits under the credential a vpay deployment for MTN
already holds, unlike refunds, which are the Disbursements product with a
different key.

### 1.2 `GetBasicUserinfo`, verbatim

```text
method:      GET
urlTemplate: /v1_0/accountholder/{accountHolderIdType}/{accountHolderId}/basicuserinfo
description: "This operation returns personal information of the account
              holder. The operation does not need any consent by the account
              holder."
```

Template parameters:

| Name | Documented values |
|---|---|
| `accountHolderIdType` | `MSISDN`, `Email`, `Alias`, `ID` — "Type of account holder identity passed in accountHolderId path param" |
| `accountHolderId` | "ID of the account holder." |

Required headers: `Authorization` ("Bearer Authentication Token generated
using CreateAccessToken API Call") and `X-Target-Environment`.

Documented responses — **and this is exhaustive, which matters below**:

| Status | Description |
|---|---|
| `200` | OK |
| `401` | Unauthorized |
| `500` | Error |

The `200` body, from the operation's example and confirmed against the
`BasicUserInfoJsonResponse` schema in the API's OpenAPI components document:

```json
{ "given_name": "string", "family_name": "string", "birthdate": "string",
  "locale": "string", "gender": "string", "status": "string" }
```

The schema's own field descriptions are OIDC's, word for word — `given_name`
is "Given name(s) or first name(s) of the End-User… all can be present, with
the names being separated by space characters", `family_name` is the same for
surnames and notes that "in some cultures, people can have multiple family
names **or no family name**". Neither is in a `required` list. That sentence
is why `wire::BasicUserInfo::name()` accepts either half alone and only
refuses when both are absent.

### 1.3 What the portal says about *not found*, which is nothing

`GetBasicUserinfo` documents no `404`. The comparison that makes this a
finding rather than an omission-by-me:

```text
RequesttoPayTransactionStatus  GET /v1_0/requesttopay/{referenceId}
  200  "OK. Note that a failed request to pay will be returned with this status too…"
  400  "Bad request, e.g. an incorrectly formatted reference id was provided."
  404  "Resource not found."
  500  "Internal Error…"
```

MTN documents a `404` explicitly where it means one. It does not, for
`basicuserinfo`.

There is a *third* operation on the same API that answers the existence
question directly, and it does not use a `404` either:

```text
ValidateAccountHolderStatus  GET /v1_0/accountholder/{accountHolderIdType}/{accountHolderId}/active
  description: "Operation is used to check if an account holder is registered
                and active in the system."
  200  "Ok. True if account holder is registered and active, false if the
        account holder is not active or not found"
  400  "Bad request…"
  500  "Internal error…"
```

**Decision D4 below is what this repository does about that.**

---

## 2. Decisions

### D1 — `AccountHolder`, a struct with one private field, not `String`

The issue's proposal returns `Result<Option<String>, ProviderError>`. The
implementation returns `Result<Option<AccountHolder>, ProviderError>`, where
`AccountHolder` holds one private `String` reachable only through `name()`
and has a **hand-written `Debug` that redacts it**.

Why the type rather than the string: "we return a name and nothing else" is
then a fact a reader of the trait sees, and there is somewhere for the
redaction to live. `{:?}` of a holder — or of any `Result` or `Option`
containing one — prints `AccountHolder { name: "[redacted]" }`, which closes
the accidental-logging path structurally instead of by care at every call
site. `ProviderConfig` already does exactly this for credentials.

### D2 — the projection is in the adapter's wire type, not downstream

`wire::BasicUserInfo` deserialises `given_name` and `family_name` and has no
field for the other four. serde drops the rest at the first point the bytes
become a Rust value, and a field MTN adds later is dropped by the same rule
with no edit. Modelling the whole body and projecting afterwards was the
alternative and is worse: every layer the full type reached would be one more
place a third party's date of birth could be logged.

### D3 — lower-case `msisdn` in the path segment

MTN's portal declares `accountHolderIdType`'s values **upper-case**
(`MSISDN | Email | Alias | ID`); issue #47 and every published example of the
endpoint spell the segment **lower-case**. Both cannot be right about a
case-sensitive backend.

vpay sends lower-case, from one constant
(`vpay_adapter_mtn_momo::ACCOUNT_HOLDER_ID_TYPE`), so changing the answer is
one edit. **This has never been checked against MTN's sandbox** and the
constant's doc comment, `flows/adapter-mtn-momo.md` and `docs/status.md` all
say so. The WireMock mapping pins the exact path vpay sends, so a change of
case is a `404` in CI rather than a silent difference against the real rail.

### D4 — `404` → `Ok(None)`, as a stated assumption

Given §1.3, mapping `404` is an assumption and not a transcription. It is
taken anyway, for three reasons:

1. A `404` is the only status a REST resource has for "no such thing", and if
   MTN answers one for an unregistered number, mapping it to a `502` would
   tell a merchant their integration is broken when it is working.
2. It is safe in the direction that matters. If MTN never sends a `404` the
   arm is dead code. If it sends one for some *other* reason, the caller's
   own rule (issue #47's name match refuses on `UNVERIFIABLE`) still refuses
   — `Ok(None)` and an error are both refusals; they differ in what a support
   ticket says, not in whether money moves.
3. Every other non-`200` is an `Err`, so the failure mode this could have had
   — a rail failure reported as "not registered" — is closed by the rest of
   the table rather than by this row being right.

**`ValidateAccountHolderStatus` (`/active`) was considered and not used.** It
answers the existence question without an assumption, but it returns a
boolean and no name, so it cannot serve `GET /v1/account_holders`. Using
*both* — `/active` to decide `Ok(None)`, `basicuserinfo` for the name —
doubles the rail calls per lookup and adds a second failure mode to reason
about, on the strength of a guess about the first one's `404`. Recorded here
as the fallback if MTN's `404` turns out not to exist. Issue #47's own
"narrower question" (`POST /v1/account_holders/verify` returning a boolean) is
a different shape again and is the maintainer's to pick; it is not
implemented.

### D5 — no `livemode` on the object

Issue #47's proposal names four keys and the implementation renders those
four. Every other object on `/v1` carries `livemode` read **off its row**, so
that an object cannot start describing itself differently when a deployment is
reconfigured. There is no row here (D6), so the only available value would be
configuration read at render time — a weaker guarantee wearing the same field
name. **Recorded as a decision, not settled**: it is one line in
`AccountHolderObject` plus a row in each SDK if the maintainer wants it.

### D6 — nothing is persisted, and the cost is stated

No repository call, no migration. The cost is that a merchant enumerating the
number space leaves no record in vpay — which is the same fact issue #47 §3's
audit-log request is about, pointing the other way. Both halves are in the
flow doc.

### D7 — `supports_account_holder_lookup` is not a `providers` column

The five capability flags beside it are columns on `providers` (migration
`0002`), seeded at boot from the adapter. This one is not, because nothing
reads a capability *out of* that table — `vpay_api` resolves an adapter
in-process and asks it — so a column would be a second copy of an answer the
linked code already owns, and a migration on the strength of it would claim a
durability the capability does not need.

### D8 — the three controls issue #47 §3 asks for that are **not built**

Rate limiting, audit logging, and a dedicated `identity:read` scope. All three
are reserved decisions, argued in the flow doc and in the module header, not
defaulted to here. The blunt version, which belongs in a note like this one:
**as shipped, a merchant credential can turn a list of phone numbers into a
list of names at whatever rate it can make HTTP requests.**

### D9 — the Node SDK's request field is `payment_method_type`, not `paymentMethodType`

The brief and the issue spell it camelCase. Every params type in
`sdks/nodejs` uses the wire's snake_case (`payment_method_types`,
`starting_after`, `success_url`), and the encoder walks those keys directly;
a camelCase field would be the only one in the package and would need a
translation step none of its neighbours have. The **accessor** is camelCase
(`client.accountHolders`), matching `client.paymentIntents`. Named in
`docs/sdks/parity.md` so it is not discovered by a merchant.

### D10 — the msisdn validator is server-side and duplicated on purpose

`frontends/apps/checkout/src/lib/msisdn.ts` had been the only implementation.
It is a form affordance that also formats for display; this one is a trust
boundary, and the page can be bypassed by any merchant calling `/v1`
directly. Sharing would mean the server trusting a client-side rule.

---

## 3. Mutations run, and what caught each

Every one was applied to the working tree, the named test was run, and the
change was reverted. "Caught by" names the test that failed.

| Mutation | Caught by |
|---|---|
| `wire::BasicUserInfo` gains `birthdate`/`locale`/`gender`/`status` and `name()` joins all six — i.e. the adapter returns the whole MTN body | `an_account_holder_lookup_returns_a_name_and_nothing_else::case_1_mtn_momo` (`left: "David Mbarga 1970-01-01 fr_CM MALE ACTIVE"`) **and** `an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing::case_1_mtn_momo` |
| the adapter's `debug!` carries `body = %text` — i.e. the rail's body, name included, reaches a log | `an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing::case_1_mtn_momo` |
| the route's `info!` carries `name = ?holder…name()` | `a_lookup_logs_a_masked_number_and_never_a_name` |
| `orange_money` declares `supports_account_holder_lookup: true` with no implementation | **the conformance suite**, all four `case_2_orange_money` parameterisations (`expected Malformed naming the body cap, got Unsupported`). **Not** `verify-status`: there is no `NotImplemented` token to be missing, which is exactly why the behavioural case has to exist |
| the adapter swallows a transport failure and answers `Ok(None)` | `a_lookup_that_cannot_reach_the_rail_is_never_reported_as_a_missing_holder::case_1_mtn_momo` (`a deadline that fires is a failure, and must not be Ok(None): None`) |
| the route drops `canonical_msisdn` and accepts any non-empty string | `a_missing_or_malformed_parameter_names_itself` |

**One mutation was placed wrongly first and is worth recording**, because a
mutation that lands where the bug cannot be is a green run that proves
nothing: mapping `Err(Transport) -> Ok(None)` *after* `read_body` left the
timeout case passing, because a fired deadline returns early from
`send_authorized` and never reaches that match. Moving it to the
`send_authorized` result — where the regression would actually be — failed the
case immediately.

---

## 4. What was not done

- **MTN's real sandbox was never called.** Not for this operation and not for
  any other; the repository has never held a credential for it.
- **`charges.payer_ref_masked` still has no producer.** The confirm path
  writes `NULL`. This route's `masked()` is the first implementation of the
  documented shape in the workspace, and the two are not wired together —
  writing that column is a change to the charge path.
- **The confirm path's `payer_ref` is still unvalidated.** `/v1/account_holders`
  validates E.164; `POST /v1/payment_intents/{id}/confirm` still accepts any
  non-empty `msisdn` and lets MTN refuse it. Out of scope here, named so it is
  not mistaken for done.
- **Neither SDK has run against a live vpay for this resource** — both are
  tested against in-process HTTP stubs (`docs/sdks/parity.md`'s gap ledger).
- **No rate limit, no audit log, no dedicated scope** (D8).
