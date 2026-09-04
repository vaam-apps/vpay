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

`submit`, `query_status` and `parse_callback` are implemented and proven
against a real `wiremock/wiremock` container by the shared conformance suite
(`backends/tests/conformance/tests/adapter_conformance.rs`, mappings in
`backends/tests/conformance/wiremock/mtn/mappings/`). The failure table above
is transcribed into `mapping::FAILURE_REASONS` and every row is asserted, in
both directions, by a unit test.

**`refund` is not implemented** and returns
`ProviderError::NotImplemented("mtn_momo::refund")` — see
[../status.md](../status.md). MTN refunds are the *Disbursements* product: a
different subscription key, a separately-scoped token and a `transfer` call.
No deployment holds those credentials, so there is nothing to build against.
`supports_refunds` stays `true` because the *rail* refunds; it is we who have
not built it, and answering `Unsupported` would be a lie about MTN.

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
minted from the *old* one, until the cached token aged out (up to an hour)
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
otherwise from `externalId`. `externalId` works because it is what *we* set on
submit: `ChargeRef` carries no charge id — `reference_id` is the only
identifier the port gives an adapter — so the "charge id" in the request body
above is that reference, rendered. A body with neither field is refused as
`Malformed` rather than guessed at.

### What the transport refuses, and why

Every call this adapter makes goes through `vpay_provider::http`, so it
inherits three refusals that are not MTN-specific:

* **Redirects are returned, never followed.** A 3xx from a rail host is an
  answer to look at, not a hop to take — following one would let a
  compromised or misconfigured DNS entry move an authenticated payment
  request to another host (`redirects_are_refused_and_never_followed`,
  conformance, both rails).
* **`HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` are ignored.** A payment
  gateway's own egress is not a merchant's corporate network. (The merchant
  SDK's copy of this client deliberately keeps proxy support.)
* **Response bodies are capped at 256 KiB** (`bounded_body`,
  `MAX_RAIL_BODY_BYTES`) rather than read to end of stream, so a load
  balancer's HTML error page or a captive portal cannot choose how much
  memory a worker task allocates. Proven live by
  `an_oversized_rail_body_is_refused_at_the_cap`; the truncation of a body
  that *does* fit but is long is proven by
  `a_rails_error_body_is_bounded_before_it_reaches_a_message`.
* Each request carries `ProviderConfig::request_timeout` explicitly, because
  one `reqwest::Client` is shared across rails and a client-level deadline
  could only ever be one rail's.

### Not proven

* **Nothing here has ever called MTN.** Every wire assertion in this document
  is against a `wiremock/wiremock` container. Both **confirm** rows in the
  environment table above are still unconfirmed, and a mapping faithful to
  this document but not to MTN would pass.
* **The 401 → re-mint → retry path is not covered by a test.** The logic is
  there and is bounded at one retry, but no mapping in the conformance suite
  returns 401 from `requesttopay` after a good token, and the adapter's own
  crate may not stand up an in-process HTTP double (ADR-0006). What *is*
  proven is the 401 on the token endpoint itself
  (`bad_credentials_are_not_reported_as_a_payer_problem`).
* The submit mappings for a 400 (`…0400`), a 500 with a code (`…0500`) and a
  500 with an HTML body (`…05ff`) exist and are correct per this document, but
  no conformance case drives them yet; their outcomes are proven instead by
  `submit_outcome`'s unit tests, which take the same status and body.
* ~~**No callback route exists.**~~ **Corrected 2026-09-04 (Step 8, lane C):
  the callback route exists, and nothing has ever called it but this
  repository's own tests.** `POST /provider/mtn_momo/callback`
  (`vpay_api::provider_callback`) parses this document's notification body into
  identifiers and pulls the charge's poll job forward; MTN signs nothing, so it
  is still a hint and the route writes no charge state.
  `backends/tests/integration/tests/provider_callback.rs` POSTs the body
  transcribed above to the URL MTN was handed on the submit, so **a body
  faithful to this document but not to MTN would pass**.
* The crate runs **48 tests, 48 passed, 0 skipped**
  (`cargo nextest run -p vpay-adapter-mtn-momo`, measured 2026-09-03).
