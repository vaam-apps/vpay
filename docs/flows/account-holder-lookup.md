# Account-holder lookup

`GET /v1/account_holders` — "whose mobile-money account is this number?"

Built for [issue #47](https://github.com/vaam-apps/vpay/issues/47): an
integrator whose refund flow lets a buyer nominate a _different_ number for
their money must match the nominated account's registered name against the
buyer's verified one, and refuse on a mismatch. Without a lookup, every
nomination is `UNVERIFIABLE` and every one is refused — safe, and dead.

**This is the only route on `/v1` that returns information about a person who
is not the caller.** Everything below follows from that.

## Two callers since 2026-09-16, and only one of them is this route

`POST /v1/refunds` (RFC-0003 § 2) asks the same question about a nominated
payee before it instructs a transfer, on the capability and never on a rail
code. It is the in-repo half of the caller described above, and it is
deliberately **narrower**: it refuses a number the rail has no record of, and
it never compares a name, because vpay holds no verified buyer name to
compare one against. Matching the holder's name to the buyer's is still the
integrator's job and is still what this route is for — see
[merchant-auth/resource-contract.md](merchant-auth/resource-contract.md).

Both callers go through one function,
`vpay_api::v1::account_holders::ask_rail`, so the counter, the masked log
line, audit events, rate limits, and the three-way answer below are the same in both. That was **not**
true as the refund routes were first written: the refund path called the
adapter directly and so incremented nothing and logged nothing, which is
recorded here because both halves of it mattered — the metric stopped being
every lookup vpay makes (the `not_found` alarm under Status reads a
denominator, so a missing caller makes it quieter rather than louder), and a
refund refused for an unregistered payee was refused before any row, any
provider-request attempt and any rail instruction, leaving no trace in vpay
at all. `a_refund_path_lookup_is_counted_and_logged_like_the_routes`
(`vpay-api`) is the case that fails if a future author points that call back
at the adapter.

**Controls apply across both paths.** A
credential with `payments:write` can put any number in `destination` on a
refund of its own intent and read "not a registered account" off the `400`,
without ever calling `GET /v1/account_holders`. So the lookup controls —
rate limit, dedicated scope, audit log — govern **lookups** across both
`GET /v1/account_holders` and `POST /v1/refunds` (ADR-0020, issue #56).

## What happens, in order

```text
merchant server
  -> GET /v1/account_holders?msisdn=…&payment_method_type=mtn_momo
       Authorization: Bearer <merchant token with account_holders:read> or signed COSE envelope

vpay_api::v1::account_holders::retrieve
  1. Scope policy            checks account_holders:read on CallerCredential (provisional pending RFC-0002 D5) -> else 403
  2. Rate limiting           checks and counts attempt on CallerCredential in rate_limit_windows
                             under privacy.lookup_rate_limit (provisional pending RFC-0002 D6) -> else 429 Retry-After
  3. Initiated audit         writes initiated record with HMAC-SHA256(lookup_audit_salt, tenant:msisdn) to audit_events
                             (provisional pending RFC-0002 D3/D7). Fails closed -> else 500
  4. payment_method_type     present? names a rail this deployment offers and
                             has enabled?                       -> else 400
  5. capabilities()          supports_account_holder_lookup?    -> else 400
  6. msisdn                  Cameroon E.164, three input shapes -> else 400
  7. ProviderAdapter::account_holder_name(msisdn, config)
  8. Terminal audit          writes terminal record (found / not_found / rail_error) to audit_events
  9. count one outcome, log one masked line, render four keys
```

```text
vpay_adapter_mtn_momo
  -> GET {base}/collection/v1_0/accountholder/msisdn/{msisdn}/basicuserinfo
       Authorization: Bearer <collections token>
       Ocp-Apim-Subscription-Key: <collections key>
       X-Target-Environment: sandbox | mtncameroon

  200 -> project to a name        -> Ok(Some(AccountHolder))
  404 -> the rail has no record   -> Ok(None)
  everything else                 -> Err(..)
```

## The three-way answer, and why nothing may collapse it

| The port says      | `/v1` answers                                                                                                                       | What it means                                          |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| `Ok(Some(holder))` | `200`, `name: "…"`, `verified: true`                                                                                                | the rail named a holder                                |
| `Ok(None)`         | `200`, `name: null`, `verified: false`                                                                                              | **the rail answered and has no record of this number** |
| `Err(..)`          | the classified status — `502` for a rail that could not be reached, `500` for a misconfiguration, `400` for a rail with no such API | **nobody asked, or the rail could not answer**         |

The middle row is a fact about the **number**. The bottom row is a fact about
the **lookup**. Issue #47's caller refuses a nominated destination on both —
but only one of them is the buyer's to fix, and only one of them is worth
paging an operator about. A rail that could not be reached, reported as
`Ok(None)`, tells an integrator that a real person's real account does not
exist.

That is why `Ok(None)` is documented on the port method as "the rail has no
record, never we could not ask", why the MTN adapter maps exactly one status
to it, and why the route matches on the result rather than `?`-ing it.

## Privacy

Issue #47 §3 is the specification. Five rules, and what enforces each.

### 1. A name, and nothing else

MTN's `basicuserinfo` body is OIDC-shaped and carries `given_name`,
`family_name`, `birthdate`, `locale`, `gender` and `status`
([adapter-mtn-momo.md](adapter-mtn-momo.md)). vpay reads two of the six.

The projection is in `vpay_adapter_mtn_momo::wire::BasicUserInfo`, which has
no field for the rest — so serde drops them at the first point the bytes
become a Rust value, and a field MTN adds tomorrow is dropped by the same
rule with no edit. The port's own type,
`vpay_provider::AccountHolder`, carries one private `String` and a
hand-written `Debug` that redacts even that, so a `{:?}` of a holder — or of
any `Result` or `Option` containing one — cannot print a name.

_Proven by_ `an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing`
in the conformance suite, whose stub sends eleven fields including five MTN
does not document (`sub`, `email`, `address`, `national_id`,
`phone_number`), and asserts each one absent from the returned value.

### 2. Privacy-minimal audit journal (ADR-0020 §3, ISO 27001 A.12.4, issue #56)

An append-only journal in `audit_events` (migration `0059`) records two events per lookup: `initiated` before the rail call, and a terminal event (`found`, `not_found`, `invalid`, `unsupported`, `rate_limited`, `rail_error`) after it, sharing a single `operation_id` (provisional pending RFC-0002 D7).

The stored `target_repr` holds a keyed HMAC-SHA256 digest of `tenant_id` + normalized MSISDN under `privacy.lookup_audit_salt` (provisional pending RFC-0002 D3); the raw MSISDN and returned name are never stored in the database.

If writing the `initiated` event to the database fails, the lookup is immediately refused with `500 Internal Server Error` (fail-closed). Retention is operational (provisional pending RFC-0002 D9).

_Proven by_ `audit_table_holds_hmac_target_representation_and_no_raw_pii` (integration).

### 3. Logs carry a masked number and never a name

One line per request, at `info` (or `warn` on a rail failure), carrying the
rail's code, `+2376••••200` and whether a holder was found. The mask is the
shape `charges.payer_ref_masked` is documented to hold: country code, the
leading `6`, four bullets, the last three digits. The bullet count is
**fixed**, not one per hidden digit — a mask whose length revealed the
input's length would be a small oracle for free.

_Proven by_ `a_lookup_logs_a_masked_number_and_never_a_name`
(`vpay-api`, against captured `tracing` output) and, for the adapter's own
debug line, by the conformance case named in rule 1 — which asserts the
holder's name, the unmasked number and all eight other personal fields are
absent from everything written to the subscriber during the call.

**A gap, named:** nothing writes `charges.payer_ref_masked` yet. The confirm
path stores `NULL` there, and this route's mask is the first producer of the
shape in the workspace. The two are not wired together, because writing that
column is a change to the charge path and not to this route.

### 4. Rate limiting — per-credential fixed window (ADR-0020, issue #56, provisional pending RFC-0002 D6)

Each lookup is counted against the caller's credential before the rail call. Lookups share a single fixed-window budget per credential across both `GET /v1/account_holders` and nominated-payee refunds on `POST /v1/refunds`.

- For signed COSE envelopes: keyed by the signing-key thumbprint (`lookup:merchant_key:SHA-256:<thumbprint>`).
- For machine tokens: keyed by the client ID (`lookup:machine_token:<client_id>`).

The window duration and attempt limit are configured via `privacy.lookup_rate_limit` (provisional pending RFC-0002 D6).

When the limit is reached, vpay returns HTTP 429 Too Many Requests with a `Retry-After: <seconds>` header rounding up the remaining seconds in the window, and error body:

```json
{
  "error": {
    "type": "rate_limit_error",
    "code": "rate_limit",
    "message": "account-holder lookup rate limit exceeded"
  }
}
```

If counting the attempt fails in the database, the lookup fails closed with a `500`.

_Proven by_ `rate_limit_blocks_and_retry_after_is_provided_and_second_credential_unaffected` and `a_nominated_payee_refund_spends_the_lookup_rate_limit_budget` (integration).

### 5. Dedicated scope (`account_holders:read`, issue #56, provisional pending RFC-0002 D5)

`GET /v1/account_holders` requires the `account_holders:read` scope.
Nominated-payee refunds via `POST /v1/refunds` also require `account_holders:read` in addition to `payments:write`.
Any credential lacking this scope is refused with HTTP 403 Forbidden (`insufficient_scope`).
Since PR #396 an implicit `HEAD` to the route is held to the same `GET` policy
— Axum serves `HEAD` from the `GET` handler, so a payments-only credential
cannot trigger the rail lookup via `HEAD` without `account_holders:read`, and
an account-holders-only credential is not refused for it.

_Proven by_ `credential_without_account_holders_read_scope_is_refused_403` and `a_nominated_payee_refund_without_account_holders_read_is_refused_403` (integration), and `a_head_to_account_holders_is_held_to_the_get_scope_policy` (integration).

## The wire

```http
GET /v1/account_holders?msisdn=237600000200&payment_method_type=mtn_momo
Authorization: Bearer <token>
```

```json
{ "object": "account_holder", "payment_method_type": "mtn_momo",
  "name": "David Mbarga", "verified": true }
```

Four keys, always all four. `name` is **present and null** when the rail has
no record — never omitted, because both SDKs model it as a required nullable
field and a dropped key is a decode failure in a merchant's own client.
`verified` is `true` exactly when `name` is present; it is redundant on
purpose, because it is what an SDK branches on, and it is **not** a claim
that anything was cryptographically verified.

`msisdn` takes the three shapes a Cameroon number is written in —
`+2376XXXXXXXX`, `2376XXXXXXXX`, or the national `6XXXXXXXX` — and the same
separators `frontends/apps/checkout/src/lib/msisdn.ts` accepts. Anything else
is a `400` naming `msisdn`.

**There is no `livemode`, and that is a departure from every other object on
this surface.** `PaymentIntentObject::livemode` is read off the _row_, so an
object cannot start describing itself differently when a deployment is
reconfigured. There is no row here and there never will be (rule 2), so the
only available value would be the deployment's current configuration read at
render time — the same field name carrying a weaker guarantee. Issue #47's
proposal names four keys and vpay renders those four. **Recorded as a
decision, not settled:** if the maintainer wants the field, it is one line in
`vpay_api::model::AccountHolderObject` plus a row in each SDK.

## Capability, not provider code

`Capabilities::supports_account_holder_lookup` is what the core branches on
(ADR-0002). `mtn_momo` declares `true` and implements it; `orange_money`
declares `false` and inherits the port's `ProviderError::Unsupported` —
Orange's equivalent route is unconfirmed from this repository and is item 8
on [adapter-orange-money.md](adapter-orange-money.md)'s "to confirm" list.

`Unsupported` and **not** a `NotImplemented` token, because nothing about
Orange is unbuilt work someone owes: the flag is a permanent answer. A rail
that _does_ expose a lookup and has not written one must declare `true` and
override the method with its own token, so `verify-status` sees the gap.

The flag is deliberately **not persisted**: unlike the four capability flags
beside it, it has no column in `providers` (migration `0002`) and no field on
`vpay_db::ProviderSeed`. Nothing reads a capability out of that table —
`vpay_api` resolves an adapter in-process and asks it — so a column would be
a second copy of an answer the linked code already owns.

`/v1` refuses an unknown rail, a _disabled_ rail and an incapable rail with a
byte-identical `400`. Telling them apart would let a merchant enumerate which
rails a deployment has configured but switched off, and the fix is the same
either way.

## Metrics

`vpay_account_holder_lookups_total{outcome}`, one increment per request,
`outcome` one of `found` / `not_found` / `unsupported` / `error`
(`vpay_core::metrics::account_holder_outcome`).

`unsupported` covers every refusal decided on the `payment_method_type` —
missing, unknown, disabled, or a rail whose capability is `false` — for the
same reason `/v1` answers all four with a byte-identical envelope: splitting
them would leak on the operator's side what the response deliberately does
not on the merchant's. `error` covers a **malformed `msisdn`** as well as
rail trouble, which is a merchant's mistake sitting in a label an operator
reads as an outage; a fifth `invalid_request` value is a reserved decision,
recorded on the constant rather than taken here.

**No label carries the number looked up, the name returned, the merchant, or
even the rail.** A Prometheus label is retained, queryable and shipped
wherever the scrape goes; a number in one would be exactly the stored record
rule 2 exists not to keep. The four outcomes are not derivable from the HTTP
status — `found` and `not_found` are both `200`, which is the whole reason
the series exists rather than being read off `vpay_http_requests_total`.

The rail call itself is on `vpay_provider_requests_total{operation="account_holder_name"}`
like every other port call, through `vpay_provider::Measured`. That series
cannot tell `found` from `not_found`, deliberately: it answers a question
about the _rail_, and this one answers a question about the _route_.

## Status

**Built, and proven against a WireMock stub only.**

> **Not safe to expose to untrusted merchants without ingress rate
> limiting.** As shipped, any credential that can call `/v1` can turn a list
> of phone numbers into a list of names at whatever rate it can make HTTP
> requests, and vpay keeps no record that it happened (rules 2 and 4 below —
> and they are the same fact, pointing in two directions). That is the
> condition on turning this route on, not a caveat about it. Since PR #396 the
> per-credential limit is **required at boot** (`privacy.lookup_rate_limit` must
> accompany `privacy.lookup_audit_salt`), so an operator can no longer
> configure the salt and forget the limit; the ingress-scale question remains
> an operator concern, not a code one.

- `GET /v1/account_holders` — served (`vpay_api::v1::V1_ROUTES`). Merchant
  token required (`payments:read` or `payments:write`), tenant-scoped
  extractor bound, no persistence, four-key response.
- `POST /v1/refunds` — **a second caller since 2026-09-16** (RFC-0003 § 2).
  Refuses a nominated payee the rail has no record of with a `400` naming
  `destination`; never compares a name; skips the lookup entirely on a rail
  whose capability is `false`, which is `orange_money`. Counted and logged
  through the same function as the route
  (`a_refund_path_lookup_is_counted_and_logged_like_the_routes`).
- `ProviderAdapter::account_holder_name` — on the port, defaulted to
  `ProviderError::Unsupported`.
- `mtn_momo` — `supports_account_holder_lookup: true`, implemented against
  `GET /collection/v1_0/accountholder/msisdn/{msisdn}/basicuserinfo`. The
  number is percent-encoded into its segment and, since 2026-10-20, refused
  as `ProviderError::Malformed` before any request (the token fetch
  included) if it is empty or only dots
  (`a_dot_segment_reference_is_refused_before_any_request`). `/v1`'s digits-only
  validation means no route reaches that refusal; it guards the port.
- `orange_money` — `supports_account_holder_lookup: false`, no
  implementation, no `NotImplemented` token.
- Both merchant SDKs — `account_holders.retrieve` / `accountHolders.retrieve`
  ([../sdks/parity.md](../sdks/parity.md)).

**What has never happened:**

- **This lookup has never been called against MTN's real sandbox.** Every
  assertion here is against a WireMock container reached over HTTP
  (ADR-0006). _(This read "MTN's real sandbox has never been called — for
  this or for any other operation" until 2026-09-23, and the second half had
  been wrong since 2026-09-15, when the Collections token, `requesttopay` and
  the status queries ran against MTN's real sandbox, settling over seven EUR
  payments (count confirmed on 2026-10-20 by Valantine Suh (@Valsuh45), who performed the tests)
  ([../status/verification/2026-09-15.md](../status/verification/2026-09-15.md)).
  Those tests never called `basicuserinfo`, so the first half still stands.)_ Two specifics are unverified as a direct consequence:
  the case of the `accountHolderIdType` path segment, and whether MTN answers
  `404` for an unknown holder at all (it documents `200`, `401` and `500`
  only for this operation). See [adapter-mtn-momo.md](adapter-mtn-momo.md).

  **The two compound, and that is the failure mode to watch for on the first
  real call.** If the path segment's case is wrong, MTN's gateway answers
  `404` to _every_ lookup; the `404 -> Ok(None)` mapping then renders every
  one of them as `{ "name": null, "verified": false }` with HTTP `200`. A
  total misconfiguration would look exactly like "nobody in Cameroon is
  registered" — no error, no page, nothing in
  `vpay_account_holder_lookups_total` but a `not_found` rate of 1.0. Neither
  assumption is unsafe about _money_ (the caller refuses on `Ok(None)` just as
  it refuses on an error), and the mapping is still the right default for the
  reason the adapter's own doc gives; what is worth knowing is that the two
  wrong together are **silent**, where either alone is not. The first
  sandbox call should therefore check a number known to be registered before
  it trusts a `not_found`, and an operator should alert on a sustained
  `not_found` rate near 1.0. Reversing either decision costs one constant
  (`vpay_adapter_mtn_momo::ACCOUNT_HOLDER_ID_TYPE`) or one match arm.

- **Orange's route is unconfirmed**, so `false` there is "we do not know of
  one", not "Orange has none".
- **Rate limiting, audit logging, dedicated scope** — built (issue #56, ADR-0020),
  governing lookups across both `GET /v1/account_holders` and `POST /v1/refunds`
  nominated payee paths.
- **Neither SDK has run against a live vpay** for this resource; both are
  tested against in-process HTTP stubs
  ([../sdks/parity.md](../sdks/parity.md)'s gap ledger, 2026-09-05).

**Where the evidence is:**

| Claim                                                                           | Test                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| the projection keeps a name and drops the rest                                  | `an_account_holder_lookup_returns_a_name_and_nothing_else` (conformance, both rails), `a_basic_user_info_body_keeps_only_the_two_name_fields`, `nothing_but_the_name_survives_the_projection`                                                          |
| a number with no holder is `Ok(None)`, not an error                             | `a_number_the_rail_has_no_record_of_is_not_an_error` (conformance, both rails)                                                                                                                                                                         |
| a rail that cannot be reached is never `Ok(None)`, and keeps its source chain   | `a_lookup_that_cannot_reach_the_rail_is_never_reported_as_a_missing_holder` (conformance, both rails)                                                                                                                                                  |
| an oversized body is refused at the cap                                         | `an_oversized_account_holder_body_is_refused_at_the_cap` (conformance, both rails)                                                                                                                                                                     |
| no personal data reaches a log line                                             | `an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing` (conformance), `a_lookup_logs_a_masked_number_and_never_a_name` (`vpay-api`)                                                                                                 |
| Orange answers `Unsupported` rather than a token                                | all five conformance cases above, on their `orange_money` parameterisation                                                                                                                                                                             |
| the whole `account_holder_outcome` table                                        | `the_account_holder_table_maps_every_documented_status` (`vpay-adapter-mtn-momo`)                                                                                                                                                                      |
| the route's validation and refusals                                             | `a_missing_or_malformed_parameter_names_itself`, `a_rail_that_has_no_such_api_is_a_400_naming_the_parameter`, `a_disabled_or_unknown_rail_is_the_same_refusal_as_an_incapable_one`                                                                     |
| every lookup vpay makes is counted and masked, from either caller               | `every_outcome_is_counted_and_no_label_carries_the_number_or_the_name`, `a_lookup_logs_a_masked_number_and_never_a_name` (the route), `a_refund_path_lookup_is_counted_and_logged_like_the_routes` (`POST /v1/refunds`)                                |
| the refund path's three-way answer: unregistered is a `400`, unreachable is not | `an_unregistered_payee_is_a_400_and_an_unreachable_rail_is_not`, `a_rail_without_a_lookup_accepts_the_destination_unverified` (`vpay-api`), `a_payee_the_rail_does_not_know_is_refused_before_the_transfer` (integration, against a real WireMock MTN) |
| every outcome counted, no label carrying the number or the name                 | `every_outcome_is_counted_and_no_label_carries_the_number_or_the_name`                                                                                                                                                                                 |
| rate limit: exceeding budget is refused 429 with Retry-After                    | `rate_limit_blocks_and_retry_after_is_provided_and_second_credential_unaffected` (integration)                                                                                                                                                         |
| rate limit: nominated payee refund spends lookup budget                         | `a_nominated_payee_refund_spends_the_lookup_rate_limit_budget` (integration)                                                                                                                                                                           |
| rate limit: a short scope never resets a longer one's counter                  | `a_short_scope_count_never_sweeps_a_longer_scope_row_inside_its_own_window` (`vpay-db`), the per-row `window_expires_at` sweep (migration 0060)                                                                                                          |
| rate limit: a salt without a configured limit refuses at boot                  | `an_audit_salt_without_a_lookup_rate_limit_is_refused_at_boot` (`vpay-config`) — lookups can no longer boot into an unlimited per-credential budget                                                                                                      |
| audit log: initiated and terminal events with HMAC target, no raw PII           | `audit_table_holds_hmac_target_representation_and_no_raw_pii` (integration)                                                                                                                                                                            |
| scope: lacking `account_holders:read` refused 403 Forbidden                     | `credential_without_account_holders_read_scope_is_refused_403`, `a_nominated_payee_refund_without_account_holders_read_is_refused_403` (integration)                                                                                                   |
| the route, end to end, over a socket, against a real WireMock MTN               | `backends/tests/integration/tests/account_holders.rs` — nine cases                                                                                                                                                                                     |
| both SDKs speak the same query string and read the same object                  | `docs/sdks/parity.md`, five `account_holders` rows                                                                                                                                                                                                     |

See [../status.md](../status.md).
