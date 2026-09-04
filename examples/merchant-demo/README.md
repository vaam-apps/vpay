# merchant-demo

The local demo `just demo` runs. It authenticates as a registered merchant with
the shipping Rust SDK ([`sdks/rust`](../../sdks/rust/)) and then makes **six
payments, on both rails, to every outcome each rail documents** — each one
walked from `create` all the way to the signed webhook the settlement produced,
read back out of the receiver's own request journal and verified with the SDK.

```bash
just demo            # generate keys, boot the stack, run this
just demo-walk       # just this, against a stack that is already up
just demo-down       # containers and volumes
```

[`docs/runbooks/demo.md`](../../docs/runbooks/demo.md) is the procedure, with
the real output of a real run and a "what this proves / what it does not".

`cargo run -p merchant-demo` on its own works too, against an already-running
vpay:

| Variable | Default |
|---|---|
| `VPAY_BASE_URL` | `http://localhost:8080` |
| `VPAY_CLIENT_ID` | `demo-merchant` |
| `VPAY_PRIVATE_KEY_FILE` | `.e2e/demo-merchant/oauth-signing-key.pem` |
| `VPAY_RECEIVER_URL` | `http://localhost:8083` |
| `MERCHANT_WEBHOOK_SECRET` | `wiremock-stub-webhook-secret-32-bytes` |

Exits `0` only when all four steps behave as expected — and step 4 is six
payments, every one of which must reach the exact status, `failure_code` and
event type the table expects. It exits non-zero naming the outcome that did
not.

## The four steps

1. **discovery + JWKS** — the OP's issuer, token endpoint and the `kid` it
   signs `/v1` access tokens with. If the issuer disagrees with the
   `VPAY_BASE_URL` the SDK derives its endpoints from, this step says so,
   because step 2 would otherwise fail with a bare `invalid_client`.
2. **an access token** — `client_credentials` + `private_key_jwt`, shown as its
   decoded `iss`/`aud`/`sub`/`exp`. Never verified here (the OP just signed it)
   and never printed.
3. **the `401`** — the same `/v1` path with no bearer token, carrying vpay's
   error envelope, so the authentication boundary is visibly real.
4. **the outcome table** — below.

## The outcome table

| # | Rail | Outcome | Steered by | After confirm | Settles to | `failure_code` | Event |
|---|---|---|---|---|---|---|---|
| 1 | `mtn_momo` | the payer approves on their handset | MSISDN `237600000ce0` | `processing` | `succeeded` | — | `payment_intent.succeeded` |
| 2 | `mtn_momo` | the payer has no balance | MSISDN `237600000f01` | `processing` | `requires_payment_method` | `insufficient_funds` | `payment_intent.payment_failed` |
| 3 | `mtn_momo` | the prompt expires unanswered | MSISDN `237600000f02` | `processing` | `requires_payment_method` | `payer_timeout` | `payment_intent.payment_failed` |
| 4 | `orange_money` | the payer completes the hosted page | 5000 XAF | `requires_action` | `succeeded` | — | `payment_intent.succeeded` |
| 5 | `orange_money` | the hosted page expires | 5001 XAF | `requires_action` | `requires_payment_method` | `payer_timeout` | `payment_intent.payment_failed` |
| 6 | `orange_money` | the rail refuses, reason undocumented | 5002 XAF | `requires_action` | `requires_payment_method` | `provider_error` | `payment_intent.payment_failed` |

There is no `failed` status: a rail-reported failure returns the intent to
`requires_payment_method` carrying `last_payment_error`
([`docs/flows/payment-lifecycle.md`](../../docs/flows/payment-lifecycle.md)).

**Nothing in this program tells vpay what should happen.** Every outcome is
selected at the rail stub, by a field of the request a merchant genuinely
controls, and the stub is a WireMock host reached over HTTP exactly as a real
rail would be ([ADR-0006](../../docs/adr/0006-no-mocks-in-main-processes.md)).
The two rails give different handles, which is why the table has a "steered by"
column:

- **MTN — the MSISDN.** `confirm` mints the rail reference itself
  (`Uuid::new_v4()`) and MTN's status query is a `GET` carrying no body, so the
  number on the submit is the only steerable field of the whole exchange. The
  stub carries the choice forward to the status query with a WireMock scenario
  (`backends/tests/conformance/wiremock/mtn/mappings/demo-outcomes.json`).
  Those are documentation numbers in the `2376000000xx` block.
- **Orange — the amount.** Orange's status call is a `POST` whose body carries
  `amount` beside `order_id`, so the stub selects on the status request itself,
  with no scenario and no state
  (`backends/tests/conformance/wiremock/orange/mappings/demo-outcomes.json`).

**The MTN half is order-sensitive**, and the demo is what keeps it honest: its
scenarios are armed by a submit and answer the *next* status query whatever
reference it carries, so the table is driven strictly sequentially — each
charge reaches a terminal state and has its webhook verified before the next
confirm is sent. Do not parallelise it without re-reading those mapping files.

**MTN's expiry is not spelled `EXPIRED`, deliberately.** `EXPIRED` is Orange's
status string. MTN's vocabulary is `PENDING`/`SUCCESSFUL`/`FAILED`, and the
reason on a prompt nobody answered is `COULD_NOT_PERFORM_TRANSACTION`
(`vpay_adapter_mtn_momo::mapping::FAILURE_REASONS`). Both land on the same core
code, `payer_timeout`, which is what a merchant integrates against — so the
outcome is demonstrated on both rails and neither stub is made to claim
something about a rail nobody has called.

## What each payment actually does

**Create and retrieve.** A real row in the demo stack's database, filed under
the demo merchant's tenant and thrown away by `just demo-down`. The retrieve is
not decoration: it is what proves the create *persisted* rather than merely
rendered an object.

**Confirm.** The request is real all the way down — vpay resolves the adapter,
commits the charge row with the reference it will submit under, calls the rail
over HTTP, and commits what came back before answering
([`docs/flows/crash-safety.md`](../../docs/flows/crash-safety.md)). A push rail
answers `processing`; a redirect rail answers `requires_action` and the demo
prints the `next_action.redirect_to_url` a merchant would send a browser to.
The demo then re-reads the intent and requires the confirm's response and the
retrieve to be the same object (bar `client_secret`, which `confirm` omits by
design), so a status that was rendered but never committed fails the run.

**Settlement.** The demo polls `GET /v1/payment_intents/{id}` — the merchant's
own fallback — until the intent leaves its post-confirm status. Nothing here
fakes an approval: the `vpay-worker` container claims the `poll_charge` job the
confirm committed *in the same transaction as the charge*, asks the stub over
HTTP, and either goes back on the ladder or commits the charge, the intent and
one event together. The demo asserts the exact `last_payment_error.code`, not
merely "it failed" — the taxonomy code
([`docs/flows/failures.md`](../../docs/flows/failures.md)) is what a merchant
integrates against, and asserting it is the difference between showing a
decline and showing the adapter's mapping table working.

**The webhook.** The demo polls the `wiremock-webhook` receiver's own request
journal (`GET /__admin/requests` — the same URL you can `curl`) for a POST that
carries a `Vpay-Event-Id` *and* whose body names this payment's intent, waits
up to 30 seconds, then checks `Stripe-Signature` carries the same value as
`Vpay-Signature` and verifies the recorded bytes with
`vpay_sdk::webhooks::verify` — the same call a merchant's handler makes. It
asserts the verified event's `type`, so a run in which every payment was
delivered as `payment_intent.succeeded` could not pass. Both journal filters
matter: the receiver answers `200` to anything POSTed at it, and its journal
outlives the run, so without the intent id one payment would happily pass on
another's webhook. The body is verified as the journal recorded it and never
re-serialised, because the signature covers bytes.

**A delivery that arrives and does not verify fails the demo louder than one
that never arrives**, because at that point vpay is signing something a
merchant cannot check.

The destination is configuration, not code: `just gen-demo-keys` writes a
`webhooks:` block into `.e2e/application-demo.yml` pointing at
`http://wiremock-webhook:8080/webhooks`, and `compose.e2e.yml` runs that
receiver — a host in configuration, exactly as the rails are (ADR-0006).

## Currencies

MTN's intents are **EUR** and Orange's are **XAF**, because
`config/application.yml` puts `mtn_momo` on EUR (MTN's sandbox rejects XAF) and
`orange_money` on XAF, and `/v1` refuses a confirm whose intent currency is not
the chosen rail's. That is a property of the profile, expressed as a config
value — never a code branch (ADR-0003).

## What this demo still cannot show

- **`amount_received`.** The settlement transaction writes that column, but the
  `payment_intent` object does not carry it, so a merchant's client cannot see
  it and neither can this demo. Printing it would mean reading the database
  behind the API this demo exists to demonstrate.
- **A payer actually visiting Orange's hosted page.** Outcome 4 prints the URL
  a merchant would send a browser to, and the stub then answers the status
  query as though the payer had completed it. Nothing here opens that URL.
- **A rail calling *us*.** There is no `POST /provider/{code}/callback`, so
  every settlement above came from vpay asking rather than from being told. The
  demo prints that sentence at the end of the table, and it is the one claim in
  the file that no assertion backs — precisely because it is a claim about
  something that does not exist.

The private key the demo reads is a throwaway generated per checkout by `just
gen-demo-keys` into git-ignored `.e2e/`. Its public half is registered in
`.e2e/application-demo.yml`, which `compose.demo.yml` mounts into the server as
the `demo` profile overlay. Nothing here is reusable as a credential anywhere
else, and neither the access token nor the webhook secret is ever printed.

See the module documentation in [`src/main.rs`](src/main.rs) for what each step
proves, and why step 2 performs its own token exchange instead of going through
`vpay_sdk::Client`.
