# merchant-demo

The local demo `just demo` runs. It authenticates as a registered merchant with
the shipping Rust SDK ([`sdks/rust`](../../sdks/rust/)) and walks a payment all
the way through vpay's `/v1` surface: discovery, a token, the `401` boundary, a
PaymentIntent, a confirm the MTN stub accepts, the worker driving that charge to
`succeeded`, and finally the signed webhook the settlement produced — read back
out of the receiver's own request journal and verified with the SDK.

```bash
just demo            # generate keys, boot the stack, run this
just demo-down       # containers and volumes
```

`cargo run -p merchant-demo` on its own works too, against an already-running
vpay:

| Variable | Default |
|---|---|
| `VPAY_BASE_URL` | `http://localhost:8080` |
| `VPAY_CLIENT_ID` | `demo-merchant` |
| `VPAY_PRIVATE_KEY_FILE` | `.e2e/demo-merchant/oauth-signing-key.pem` |
| `VPAY_RECEIVER_URL` | `http://localhost:8083` |
| `MERCHANT_WEBHOOK_SECRET` | `wiremock-stub-webhook-secret-32-bytes` |

Exits `0` only when all seven steps behave as expected, and non-zero naming the
step that did not.

Step 4 creates a real PaymentIntent and reads it back — a real row in the demo
stack's database, thrown away by `just demo-down`. **Step 5 confirms it and
expects `processing`**, a push rail's one success state: the confirm reaches
the stack's MTN WireMock rail over HTTP, the rail accepts, and the charge is
committed before the response is built. The demo then re-reads the intent and
requires the confirm's response and the retrieve to be the *same object*, so
a status that was rendered but never committed fails the run. Anything other
than `processing` — including a `next_action` on a push rail — is a failure.

**This is the assertion that inverted on 2026-09-03.** Until Step 3, step 5
expected `501 not_implemented` and treated a successful confirm as a defect,
because no adapter implemented `submit`. It now expects the opposite. The rail
is still a stub — a WireMock host reached over HTTP exactly as a real rail
would be, never a linked implementation
([ADR-0006](../../docs/adr/0006-no-mocks-in-main-processes.md)) — and **MTN's
and Orange's real endpoints have never been called by this code.**

**Step 6 waits for the worker to settle it.** It polls
`GET /v1/payment_intents/{id}` — the merchant's own fallback — until the intent
is no longer `processing`, and requires `succeeded`. Nothing here fakes an
approval: the `vpay-worker` container claims the `poll_charge` job the confirm
committed in the same transaction as the charge, asks the MTN stub over HTTP,
is told `PENDING`, comes back on the ladder's first rung and is told
`SUCCESSFUL`. The stub answers that way because the demo's MSISDN enters a
WireMock scenario keyed on it, which is the coupling
[`src/main.rs`](src/main.rs) states rather than buries.

**Step 7 reads the webhook that settlement produced.** It polls the
`wiremock-webhook` receiver's own request journal (`GET /__admin/requests` —
the same URL you can `curl`) for a POST that carries a `Vpay-Event-Id` *and*
whose body names this run's intent, then checks that `Stripe-Signature`
carries the same value as `Vpay-Signature` and verifies the recorded bytes
with `vpay_sdk::webhooks::verify` — the same call a merchant's handler makes.
It waits up to **30 seconds**. Both filters matter: the receiver answers `200`
to anything POSTed at it, and its journal outlives the run, so without the
intent id a *previous* run's delivery would satisfy this one. The body is
verified as the journal recorded it and never re-serialised, because the
signature covers bytes.

**This step used to report an absent webhook and pass.** That was correct while
nothing settled a charge and nothing drained the outbox; both now exist, so an
absence here is a defect and fails the run. **A delivery that arrives and does
not verify fails it louder**, because at that point vpay is signing something a
merchant cannot check, which is worse than sending nothing.

The destination is configuration, not code: `just gen-demo-keys` writes a
`webhooks:` block into `.e2e/application-demo.yml` pointing at
`http://wiremock-webhook:8080/webhooks`, and `compose.e2e.yml` runs that
receiver — a host in configuration, exactly as the rails are (ADR-0006).

The demo's intent is in **EUR**, because `config/application.yml` puts
`mtn_momo` on EUR (MTN's sandbox rejects XAF) and `/v1` refuses a confirm
whose intent currency is not the rail's settlement currency.

The private key it reads is a throwaway generated per checkout by `just
gen-demo-keys` into git-ignored `.e2e/`. Its public half is registered in
`.e2e/application-demo.yml`, which `compose.demo.yml` mounts into the server as
the `demo` profile overlay. Nothing here is reusable as a credential anywhere
else, and the access token itself is never printed.

See the module documentation in [`src/main.rs`](src/main.rs) for what each step
proves, and why step 2 performs its own token exchange instead of going through
`vpay_sdk::Client`.
