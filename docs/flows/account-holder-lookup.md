# Account-holder lookup

`GET /v1/account_holders` — "whose mobile-money account is this number?"

Built for [issue #47](https://github.com/vaam-apps/vpay/issues/47): an
integrator whose refund flow lets a buyer nominate a *different* number for
their money must match the nominated account's registered name against the
buyer's verified one, and refuse on a mismatch. Without a lookup, every
nomination is `UNVERIFIABLE` and every one is refused — safe, and dead.

**This is the only route on `/v1` that returns information about a person who
is not the caller.** Everything below follows from that.

## What happens, in order

```text
merchant server
  -> GET /v1/account_holders?msisdn=…&payment_method_type=mtn_momo
       Authorization: Bearer <merchant token, payments:read or :write>

vpay_api::v1::account_holders::retrieve
  1. MerchantScope           the auth middleware resolved a tenant, or this
                             fails closed with a 500 (nothing is scoped by
                             it — see "the tenant is bound and unused")
  2. payment_method_type     present? names a rail this deployment offers and
                             has enabled?                       -> else 400
  3. capabilities()          supports_account_holder_lookup?    -> else 400
  4. msisdn                  Cameroon E.164, three input shapes -> else 400
  5. ProviderAdapter::account_holder_name(msisdn, config)
  6. count one outcome, log one line, render four keys
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

| The port says | `/v1` answers | What it means |
|---|---|---|
| `Ok(Some(holder))` | `200`, `name: "…"`, `verified: true` | the rail named a holder |
| `Ok(None)` | `200`, `name: null`, `verified: false` | **the rail answered and has no record of this number** |
| `Err(..)` | the classified status — `502` for a rail that could not be reached, `500` for a misconfiguration, `400` for a rail with no such API | **nobody asked, or the rail could not answer** |

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

*Proven by* `an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing`
in the conformance suite, whose stub sends eleven fields including five MTN
does not document (`sub`, `email`, `address`, `national_id`,
`phone_number`), and asserts each one absent from the returned value.

### 2. Nothing is persisted

Not the name, not the number, not the fact that the question was asked.
There is no repository call in `vpay_api::v1::account_holders` and no
migration behind it.

**This has a cost and it is not hidden:** it is also why rule 4 below is a
reserved decision rather than a feature. A merchant enumerating the number
space leaves no record in vpay.

### 3. Logs carry a masked number and never a name

One line per request, at `info` (or `warn` on a rail failure), carrying the
rail's code, `+2376••••200` and whether a holder was found. The mask is the
shape `charges.payer_ref_masked` is documented to hold: country code, the
leading `6`, four bullets, the last three digits. The bullet count is
**fixed**, not one per hidden digit — a mask whose length revealed the
input's length would be a small oracle for free.

*Proven by* `a_lookup_logs_a_masked_number_and_never_a_name`
(`vpay-api`, against captured `tracing` output) and, for the adapter's own
debug line, by the conformance case named in rule 1 — which asserts the
holder's name, the unmasked number and all eight other personal fields are
absent from everything written to the subscriber during the call.

**A gap, named:** nothing writes `charges.payer_ref_masked` yet. The confirm
path stores `NULL` there, and this route's mask is the first producer of the
shape in the workspace. The two are not wired together, because writing that
column is a change to the charge path and not to this route.

### 4. Rate limiting — **a reserved decision, not built**

Issue #47 §3 asks for a per-merchant rate limit, and says why: "unlimited
lookup of arbitrary MSISDNs is a name-harvesting oracle", and the abuse here
is *exfiltration* rather than load.

**It is not built, and no default was chosen.** Rate limiting in this
deployment is an ingress concern — [provider-port.md](provider-port.md)
records the same "**no rate limit**, per charge or per source" for the
callback route — and inventing a per-merchant token bucket inside one
handler would be a control nothing else on this surface has, with a limit
nobody chose. It is left to the maintainer, who has to decide the shape (per
merchant? per merchant per MSISDN? a daily ceiling?), the enforcement point
(ingress, or `/v1`) and what a merchant who exceeds it is told.

Until then: **this route lets any merchant with a valid credential turn a
list of phone numbers into a list of names, at whatever rate they can make
HTTP requests.** That sentence is the honest description of what shipping
this costs.

### 5. Audit logging and a scope of its own — **also reserved**

§3 asks for both. Neither is built.

*The audit log* contradicts rule 2 above: a per-merchant, per-MSISDN trail
**is** a stored record of who asked about whom, and it is precisely the
record rule 2 declines to keep. Which of the two wins is a policy choice, not
an implementation detail, and taking it quietly in either direction would be
wrong. (`MerchantScope` is already bound on the handler, so the value such a
log would be keyed on is in hand the day it is decided.)

*A scope of its own* (`identity:read` rather than `payments:read`) is a
three-place change that fails **silently** when the places disagree — see
`SCOPE_PAYMENTS_WRITE`'s own doc comment: the string an operator writes in a
registration, the string the OP mints, and the string the middleware checks.
It would also refuse every existing merchant credential on the day it landed.
The route is served under `payments:read` today, like every other `GET`.

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
this surface.** `PaymentIntentObject::livemode` is read off the *row*, so an
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
that *does* expose a lookup and has not written one must declare `true` and
override the method with its own token, so `verify-status` sees the gap.

The flag is deliberately **not persisted**: unlike the four capability flags
beside it, it has no column in `providers` (migration `0002`) and no field on
`vpay_db::ProviderSeed`. Nothing reads a capability out of that table —
`vpay_api` resolves an adapter in-process and asks it — so a column would be
a second copy of an answer the linked code already owns.

`/v1` refuses an unknown rail, a *disabled* rail and an incapable rail with a
byte-identical `400`. Telling them apart would let a merchant enumerate which
rails a deployment has configured but switched off, and the fix is the same
either way.

## Metrics

`vpay_account_holder_lookups_total{outcome}`, one increment per request,
`outcome` one of `found` / `not_found` / `unsupported` / `error`
(`vpay_core::metrics::account_holder_outcome`).

**No label carries the number looked up, the name returned, the merchant, or
even the rail.** A Prometheus label is retained, queryable and shipped
wherever the scrape goes; a number in one would be exactly the stored record
rule 2 exists not to keep. The four outcomes are not derivable from the HTTP
status — `found` and `not_found` are both `200`, which is the whole reason
the series exists rather than being read off `vpay_http_requests_total`.

The rail call itself is on `vpay_provider_requests_total{operation="account_holder_name"}`
like every other port call, through `vpay_provider::Measured`. That series
cannot tell `found` from `not_found`, deliberately: it answers a question
about the *rail*, and this one answers a question about the *route*.

## Status

**Built, and proven against a WireMock stub only.**

- `GET /v1/account_holders` — served (`vpay_api::v1::V1_ROUTES`). Merchant
  token required (`payments:read` or `payments:write`), tenant-scoped
  extractor bound, no persistence, four-key response.
- `ProviderAdapter::account_holder_name` — on the port, defaulted to
  `ProviderError::Unsupported`.
- `mtn_momo` — `supports_account_holder_lookup: true`, implemented against
  `GET /collection/v1_0/accountholder/msisdn/{msisdn}/basicuserinfo`.
- `orange_money` — `supports_account_holder_lookup: false`, no
  implementation, no `NotImplemented` token.
- Both merchant SDKs — `account_holders.retrieve` / `accountHolders.retrieve`
  ([../sdks/parity.md](../sdks/parity.md)).

**What has never happened:**

- **MTN's real sandbox has never been called** — for this or for any other
  operation. Every assertion here is against a WireMock container reached
  over HTTP (ADR-0006). Two specifics are unverified as a direct consequence:
  the case of the `accountHolderIdType` path segment, and whether MTN answers
  `404` for an unknown holder at all (it documents `200`, `401` and `500`
  only for this operation). See [adapter-mtn-momo.md](adapter-mtn-momo.md).
- **Orange's route is unconfirmed**, so `false` there is "we do not know of
  one", not "Orange has none".
- **No rate limit, no audit log, no dedicated scope** — the three reserved
  decisions above.
- **Neither SDK has run against a live vpay** for this resource; both are
  tested against in-process HTTP stubs
  ([../sdks/parity.md](../sdks/parity.md)'s gap ledger, 2026-09-05).

**Where the evidence is:**

| Claim | Test |
|---|---|
| the projection keeps a name and drops the rest | `an_account_holder_lookup_returns_a_name_and_nothing_else` (conformance, both rails), `a_basic_user_info_body_keeps_only_the_two_name_fields`, `nothing_but_the_name_survives_the_projection` |
| a number with no holder is `Ok(None)`, not an error | `a_number_the_rail_has_no_record_of_is_not_an_error` (conformance, both rails) |
| a rail that cannot be reached is never `Ok(None)`, and keeps its source chain | `a_lookup_that_cannot_reach_the_rail_is_never_reported_as_a_missing_holder` (conformance, both rails) |
| an oversized body is refused at the cap | `an_oversized_account_holder_body_is_refused_at_the_cap` (conformance, both rails) |
| no personal data reaches a log line | `an_account_holder_body_of_personal_data_yields_a_name_and_leaks_nothing` (conformance), `a_lookup_logs_a_masked_number_and_never_a_name` (`vpay-api`) |
| Orange answers `Unsupported` rather than a token | all five conformance cases above, on their `orange_money` parameterisation |
| the whole `account_holder_outcome` table | `the_account_holder_table_maps_every_documented_status` (`vpay-adapter-mtn-momo`) |
| the route's validation and refusals | `a_missing_or_malformed_parameter_names_itself`, `a_rail_that_has_no_such_api_is_a_400_naming_the_parameter`, `a_disabled_or_unknown_rail_is_the_same_refusal_as_an_incapable_one` |
| every outcome counted, no label carrying the number or the name | `every_outcome_is_counted_and_no_label_carries_the_number_or_the_name` |
| the route, end to end, over a socket, against a real WireMock MTN | `backends/tests/integration/tests/account_holders.rs` — six cases |
| both SDKs speak the same query string and read the same object | `docs/sdks/parity.md`, five `account_holders` rows |

See [../status.md](../status.md).
