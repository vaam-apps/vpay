# The local demo: one command, both rails, six outcomes

**What this page is.** The procedure that brings vpay up from nothing on one
machine and drives six payments through it — both rails, every outcome each
rail documents — with the output of a real run pasted below rather than
narrated. It is [issue #11](https://github.com/vaam-store/vpay/issues/11)'s
"someone can bring it up from nothing and walk through a payment end to end".

**Status, stated before anything else.** Every command and every line of
output on this page was run on 2026-09-03/04 on the authoring machine. Two
things it does *not* claim:

- ~~**`just demo` end to end has not been observed green on that machine.**~~
  **Updated 2026-09-04.** When this page was first written, four of six
  walkthrough attempts died on a `confirm` with a `500` — **a defect in vpay and
  not in the demo**, written up in
  [§9](#9-the-known-flake-a-real-defect-the-demo-found) rather than retried
  until green, because a green obtained by retrying is not evidence. That defect
  was fixed later the same day. ~~`just demo` from nothing then ran green on a
  branch that carried the fix.~~ **Corrected 2026-09-04, and this is what the
  page claims about green runs:**
  **one green run from nothing exists** (lane A's rebased branch, 2026-09-04,
  **without** lane G; the race is timing-dependent and did not fire, so it is a
  green *pre-fix* run and not evidence for the fix) — six outcomes for six,
  exit 0, zero `write_matched_no_row`; **lane A's own earlier count was two
  greens in six attempts and zero for three from nothing**; **lane G did not
  re-run the demo**. **Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in):** `just demo` from nothing **six times, four green** (six outcomes for six each, exit 0; the first green is the paste in `docs/runbooks/demo.md` §4). The two failures were not the race: in both, the VM's Postgres answered single statements in 14–36 s while the host's I/O pressure was above 50 % (a second VM and two reviewer builds), and the worker's log shows the settlement and the webhook landing *after* the demo's 120 s / 30 s budgets — a `DELETE FROM jobs` at 18 s and a `COMMIT` at 14.6 s in one, `INSERT`s at 5 s each in the other. `write_matched_no_row` appeared in no run's server or worker log. The plan's bar of three from nothing is met in count, not consecutively, which is why the row stays 🟡 and this sentence says both.
- **No rail has ever been called.** Both rails are WireMock hosts in
  configuration, reached over HTTP exactly as a real rail would be
  ([ADR-0006](../adr/0006-no-mocks-in-main-processes.md)). A `succeeded` below
  means the worker asked a stub and the stub said `SUCCESSFUL`. **No money has
  moved and the "do not deploy" banner in [../status.md](../status.md)
  stands.**

---

## 1. Prerequisites

Docker with Compose v2.24+ (the demo overlay uses `!reset`), the toolchain
`rust-toolchain.toml` pins, and `just`, `jq`, `curl`, `openssl` on `PATH`.
Every recipe below checks for the tools it needs and names the missing one
rather than failing later with a timeout.

Nothing else has to exist first. `just demo` generates its own throwaway keys,
builds both images, and creates its own database.

## 2. The commands

```bash
just demo-down          # start from nothing; safe when nothing is running
just demo               # keys + up --wait + the walkthrough
just demo-down          # containers AND volumes
```

`just demo` is a composition of two recipes that also exist on their own, so
nothing on this page runs a path the one-liner does not:

| Recipe | What it does |
|---|---|
| `just demo-up` | `gen-demo-keys`, `docker compose up -d --build --wait`, then poll `/healthz` |
| `just demo-walk` | run `examples/merchant-demo` against a stack that is already up |
| `just demo-status` | what is running, under which project, on which host ports |
| `just demo-down` | stop the stack and delete its volumes |
| `just demo` | `demo-up` then `demo-walk` |

`demo-walk` is separately re-runnable, which is what you want while reading its
output: each run mints fresh idempotency keys and fresh intents.

**Three variables**, and they are the whole of the no-collision story
([§7](#7-two-demos-on-one-machine)):

| Variable | Default | What it moves |
|---|---|---|
| `demo_project` | `vpay-demo` | the Compose project name — containers, network, `pgdata` volume |
| `demo_port` | `8080` | the host port `vpay-server` is published on |
| `demo_receiver_port` | `8083` | the host port the merchant webhook receiver is published on |

**This page was run with `demo_port=18080 demo_receiver_port=18083`**, because
host port 8080 was occupied by an unrelated project on the authoring machine —
which is exactly the case the variables exist for. With 8080 free, plain `just
demo` is the same run.

**`demo_port` publishes the whole server, and since Step 8 that includes the
unauthenticated `POST /provider/{code}/callback`:** Compose binds
`${VPAY_DEMO_PORT:-8080}:8080` on `0.0.0.0`, so anyone on the same LAN as your
demo stack can post a rail notification at it without a credential — which
moves no state, because a callback is only ever a hint that pulls an
already-queued poll forward and the authenticated status query is the one thing
that settles a charge ([provider-port.md](../flows/provider-port.md)), but it
is a route that did not exist before and worth knowing about before running
this on a shared network.

`demo_port` is propagated to the three places that must agree, and
`gen-demo-keys` regenerates the profile overlay when it changes: the published
port, `deployment.public_base_url` in `.e2e/application-demo.yml` (which the OP
turns into the `issuer` on every token), and `VPAY_BASE_URL` for the demo
binary. When those disagree the OP answers `invalid_client` and **no message
mentions a port**, which is why the check exists.

## 3. Bringing it up

Readiness is `docker compose up --wait` on healthchecks, not a sleep. Postgres
and all three WireMock containers carry one; WireMock's `/__admin/health` means
"the admin API is up *and* the mappings under `/home/wiremock` have been
loaded", which a TCP probe cannot distinguish from a JVM that has merely bound
its port.

`vpay-server` and `vpay-worker` **cannot** carry one: their runtime image is
`FROM scratch` ([ADR-0004](../adr/0004-musl-mimalloc.md)), so there is no shell
to run a check in and the only executable in the image is the binary itself.
The honest fix is a `--healthcheck` self-check mode on the binaries; until that
lands, `demo-up` observes the server from outside, polling `/healthz` exactly
as `.github/workflows/ci.yml`'s e2e job does.

**That poll is load-bearing, and the paste below proves it**: Compose prints
`Container vpay-demo-vpay-server-1 Healthy` for a container that has no
healthcheck at all — for those it reports *running*, in a progress line that
says "Healthy" — and the very next line is a `curl` that got
`(52) Empty reply from server` because the server had not finished binding. A
demo that trusted `--wait` alone for those two services would fail in the
walkthrough and blame the wrong thing.

Real output, from `just demo_port=18080 demo_receiver_port=18083 demo`. The
image build is elided (it is several minutes of ordinary buildx output on a
cold cache and says nothing about vpay); everything after it is verbatim:

```console
$ just demo_port=18080 demo_receiver_port=18083 demo
gen-e2e-signing-key: .e2e/oauth-signing-key.pem already exists, keeping it
gen-demo-keys: .e2e/application-demo.yml was generated for a different demo_port than 18080 — regenerating the pair
gen-demo-keys: wrote .e2e/demo-merchant/oauth-signing-key.pem (3072-bit RSA, mode 0600, host-only)
gen-demo-keys: wrote .e2e/application-demo.yml — client_id=demo-merchant kid=aZbYeC696RJXBacNAF3GOCe2P6e4eOSX9g9gETeOoGs
demo-up: project vpay-demo, server :18080, receiver :18083
[... docker buildx output elided ...]
 Image vpay-demo-vpay-server Built 
 Image vpay-demo-vpay-worker Built 
 Volume vpay-demo_pgdata Creating 
 Volume vpay-demo_pgdata Creating 
 Network vpay-demo_default Creating 
 Network vpay-demo_default Creating 
 Volume vpay-demo_pgdata Created 
 Volume vpay-demo_pgdata Created 
 Network vpay-demo_default Created 
 Network vpay-demo_default Created 
 Container vpay-demo-wiremock-mtn-1 Creating 
 Container vpay-demo-wiremock-webhook-1 Creating 
 Container vpay-demo-wiremock-orange-1 Creating 
 Container vpay-demo-postgres-1 Creating 
 Container vpay-demo-postgres-1 Created 
 Container vpay-demo-wiremock-webhook-1 Created 
 Container vpay-demo-vpay-worker-1 Creating 
 Container vpay-demo-wiremock-mtn-1 Created 
 Container vpay-demo-wiremock-orange-1 Created 
 Container vpay-demo-vpay-server-1 Creating 
 Container vpay-demo-vpay-worker-1 Created 
 Container vpay-demo-vpay-server-1 Created 
 Container vpay-demo-wiremock-orange-1 Starting 
 Container vpay-demo-postgres-1 Starting 
 Container vpay-demo-wiremock-webhook-1 Starting 
 Container vpay-demo-wiremock-mtn-1 Starting 
 Container vpay-demo-wiremock-orange-1 Started 
 Container vpay-demo-postgres-1 Started 
 Container vpay-demo-wiremock-webhook-1 Started 
 Container vpay-demo-postgres-1 Waiting 
 Container vpay-demo-wiremock-mtn-1 Started 
 Container vpay-demo-postgres-1 Waiting 
 Container vpay-demo-postgres-1 Healthy 
 Container vpay-demo-vpay-server-1 Starting 
 Container vpay-demo-postgres-1 Healthy 
 Container vpay-demo-vpay-worker-1 Starting 
 Container vpay-demo-vpay-server-1 Started 
 Container vpay-demo-vpay-worker-1 Started 
 Container vpay-demo-vpay-server-1 Waiting 
 Container vpay-demo-vpay-worker-1 Waiting 
 Container vpay-demo-wiremock-mtn-1 Waiting 
 Container vpay-demo-wiremock-orange-1 Waiting 
 Container vpay-demo-postgres-1 Waiting 
 Container vpay-demo-wiremock-webhook-1 Waiting 
 Container vpay-demo-vpay-server-1 Healthy 
 Container vpay-demo-wiremock-webhook-1 Healthy 
 Container vpay-demo-vpay-worker-1 Healthy 
 Container vpay-demo-postgres-1 Healthy 
 Container vpay-demo-wiremock-mtn-1 Healthy 
 Container vpay-demo-wiremock-orange-1 Healthy 
demo-up: waiting for http://localhost:18080/healthz
```

Six services, named rather than left to Compose's "everything in the file
set": `postgres`, `wiremock-mtn`, `wiremock-orange`, `wiremock-webhook`,
`vpay-server`, `vpay-worker`. The seventh, `dashboard`, is deliberately absent
— see [§6](#6-the-dashboard-is-out-of-scope-and-why).

## 4. The walkthrough

Real output of `just demo` — `demo-up` from nothing, then `demo-walk` — on the
**merged** Step 8 branch (`claude/step8-production-gate`, code as of `4b5a9d7`,
with lanes G and H in), captured 2026-09-04 in the `vpay-ci` VM from the first
of the integrator's demo runs from nothing. Verbatim and complete from the
program's first line to its last; nothing below was written by hand. The
`demo-up` output above it (image builds, `docker compose up --wait`) is the
same as §3's and is not repeated here.

```console
vpay merchant demo
  base URL     http://localhost:8080   (VPAY_BASE_URL)
  client_id    demo-merchant   (VPAY_CLIENT_ID)
  private key  .e2e/demo-merchant/oauth-signing-key.pem   (VPAY_PRIVATE_KEY_FILE)
  receiver     http://localhost:8083   (VPAY_RECEIVER_URL)

[1/4] discovery + JWKS
  ✔ GET /v1/oauth/.well-known/openid-configuration
      issuer          http://localhost:8080/v1/oauth
      token_endpoint  http://localhost:8080/v1/oauth/token
      jwks_uri        http://localhost:8080/v1/oauth/jwks.json
  ✔ GET /v1/oauth/jwks.json — 1 key(s)
      kid=1KSUUZ9j7i7x8XDcFrbu4LzLC2aZEsJYZRHXHzLjbKI  alg=RS256  kty=RSA

[2/4] access token (client_credentials + private_key_jwt)
  ✔ POST http://localhost:8080/v1/oauth/token — HTTP 200, token_type=Bearer, expires_in=900
      decoded (UNVERIFIED) claims — the token itself is never printed:
        iss  http://localhost:8080/v1/oauth
        aud  vpay:v1
        sub  demo-merchant
        exp  1788496595

[3/4] the same path with no bearer token
  ✔ GET /v1/payment_intents/pi_demo without a token — HTTP 401
      error.type     authentication_error
      error.code     missing_bearer_token
      error.message  No Authorization header was provided. Send an OAuth2 access token as 'Authorization: Bearer <token>'.

[4/4] 6 payments, on both rails, to every outcome each rail documents
      each one: create → retrieve → confirm → the worker settles it → the signed webhook it produced

  ── 1/6  mtn_momo · the payer approves on their handset ─────────────────────────
     selected by: MSISDN 237600000ce0 enters the `mtn-e2e-poll` scenario (requesttopay-scenario.json): PENDING on the first status query, SUCCESSFUL on the second
     ✔ POST /v1/payment_intents
       id             pi_x70mabwa394396hp5j40hs0z
       status         requires_payment_method
       amount         5000 EUR   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
     ✔ GET /v1/payment_intents/pi_x70mabwa394396hp5j40hs0z — identical object
     ✔ POST /v1/payment_intents/pi_x70mabwa394396hp5j40hs0z/confirm — HTTP 200, the rail accepted the charge
       id             pi_x70mabwa394396hp5j40hs0z
       status         processing
       amount         5000 EUR   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       next_action    null   (a push rail prompts the handset; there is nothing for a browser to do)
     ✔ GET /v1/payment_intents/pi_x70mabwa394396hp5j40hs0z — identical object, so the `processing` a merchant was told is the `processing` vpay stored
     … polling until it leaves `processing` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 7 polls — the rail was asked, and answered
       id             pi_x70mabwa394396hp5j40hs0z
       status         succeeded
       amount         5000 EUR   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       amount_received  not on the wire — the settlement transaction writes payment_intents.amount_received (= amount, 5000), but the payment_intent object does not carry it yet
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_v3sfqma4js0dh14mnkx31qhg
       event.type     payment_intent.succeeded
       livemode       false
       data.object.id pi_x70mabwa394396hp5j40hs0z
       data.object.status succeeded

  ── 2/6  mtn_momo · the payer has no balance ─────────────────────────
     selected by: MSISDN 237600000f01 arms the `mtn-demo-decline` scenario (demo-outcomes.json), which answers the next status query FAILED/NOT_ENOUGH_FUNDS
     ✔ POST /v1/payment_intents
       id             pi_q114t7cjm57hx9pgxth7q73p
       status         requires_payment_method
       amount         5000 EUR   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
     ✔ GET /v1/payment_intents/pi_q114t7cjm57hx9pgxth7q73p — identical object
     ✔ POST /v1/payment_intents/pi_q114t7cjm57hx9pgxth7q73p/confirm — HTTP 200, the rail accepted the charge
       id             pi_q114t7cjm57hx9pgxth7q73p
       status         processing
       amount         5000 EUR   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       next_action    null   (a push rail prompts the handset; there is nothing for a browser to do)
     ✔ GET /v1/payment_intents/pi_q114t7cjm57hx9pgxth7q73p — identical object, so the `processing` a merchant was told is the `processing` vpay stored
     … polling until it leaves `processing` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_q114t7cjm57hx9pgxth7q73p
       status         requires_payment_method
       amount         5000 EUR   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       failure_code   insufficient_funds   (charges.failure_code, the closed vocabulary of docs/flows/failures.md)
       message        The payment was declined (insufficient_funds).
       the rail's own raw words are in charges.failure_raw and in the worker's log; only the taxonomy code and this generic message are public
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_7kvxw4c6hd0ndfr0p217hsr6
       event.type     payment_intent.payment_failed
       livemode       false
       data.object.id pi_q114t7cjm57hx9pgxth7q73p
       data.object.status requires_payment_method

  ── 3/6  mtn_momo · the prompt expires unanswered ─────────────────────────
     selected by: MSISDN 237600000f02 arms the `mtn-demo-expiry` scenario (demo-outcomes.json), which answers FAILED with the OBJECT-shaped reason COULD_NOT_PERFORM_TRANSACTION — MTN's ~5-minute PIN window
     ✔ POST /v1/payment_intents
       id             pi_wb6kzf1nk54kx2ag81vpmt3p
       status         requires_payment_method
       amount         5000 EUR   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
     ✔ GET /v1/payment_intents/pi_wb6kzf1nk54kx2ag81vpmt3p — identical object
     ✔ POST /v1/payment_intents/pi_wb6kzf1nk54kx2ag81vpmt3p/confirm — HTTP 200, the rail accepted the charge
       id             pi_wb6kzf1nk54kx2ag81vpmt3p
       status         processing
       amount         5000 EUR   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       next_action    null   (a push rail prompts the handset; there is nothing for a browser to do)
     ✔ GET /v1/payment_intents/pi_wb6kzf1nk54kx2ag81vpmt3p — identical object, so the `processing` a merchant was told is the `processing` vpay stored
     … polling until it leaves `processing` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_wb6kzf1nk54kx2ag81vpmt3p
       status         requires_payment_method
       amount         5000 EUR   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       failure_code   payer_timeout   (charges.failure_code, the closed vocabulary of docs/flows/failures.md)
       message        The payment was declined (payer_timeout).
       the rail's own raw words are in charges.failure_raw and in the worker's log; only the taxonomy code and this generic message are public
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_bkz10pdk0x7fx80qev6zeewe
       event.type     payment_intent.payment_failed
       livemode       false
       data.object.id pi_wb6kzf1nk54kx2ag81vpmt3p
       data.object.status requires_payment_method

  ── 4/6  orange_money · the payer completes the hosted page ─────────────────────────
     selected by: 5000 XAF is claimed by no amount-keyed mapping, so the status query falls through to transactionstatus.json's catch-all SUCCESS
     ✔ POST /v1/payment_intents
       id             pi_41w8es8my13jb3223e30tan9
       status         requires_payment_method
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
     ✔ GET /v1/payment_intents/pi_41w8es8my13jb3223e30tan9 — identical object
     ✔ POST /v1/payment_intents/pi_41w8es8my13jb3223e30tan9/confirm — HTTP 200, the rail accepted the charge
       id             pi_41w8es8my13jb3223e30tan9
       status         requires_action
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       next_action    redirect_to_url — send the payer here:
         url          http://localhost:8082/stub-hosted-page/pay-9d54a259-c1c9-48b1-a866-b21d72270ad4
         return_url   https://shop.example/orders/demo-1234/return
       (this demo does NOT open that URL. The rail stub answers the status query as though the payer had completed the page — the browser return trip is a named gap, docs/runbooks/demo.md)
     ✔ GET /v1/payment_intents/pi_41w8es8my13jb3223e30tan9 — identical object, so the `requires_action` a merchant was told is the `requires_action` vpay stored
     … polling until it leaves `requires_action` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_41w8es8my13jb3223e30tan9
       status         succeeded
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       amount_received  not on the wire — the settlement transaction writes payment_intents.amount_received (= amount, 5000), but the payment_intent object does not carry it yet
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_3sjya99zgs4ws8wd74478tv2
       event.type     payment_intent.succeeded
       livemode       false
       data.object.id pi_41w8es8my13jb3223e30tan9
       data.object.status succeeded

  ── 5/6  orange_money · the hosted page expires before the payer finishes ─────────────────────────
     selected by: 5001 XAF selects demo-outcomes.json's EXPIRED mapping — the amount travels on Orange's status body, so no scenario is needed
     ✔ POST /v1/payment_intents
       id             pi_8wxevnfggh499dkcnz23d11a
       status         requires_payment_method
       amount         5001 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
     ✔ GET /v1/payment_intents/pi_8wxevnfggh499dkcnz23d11a — identical object
     ✔ POST /v1/payment_intents/pi_8wxevnfggh499dkcnz23d11a/confirm — HTTP 200, the rail accepted the charge
       id             pi_8wxevnfggh499dkcnz23d11a
       status         requires_action
       amount         5001 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       next_action    redirect_to_url — send the payer here:
         url          http://localhost:8082/stub-hosted-page/pay-04fea4cd-ffcb-449e-9996-82cc354d9319
         return_url   https://shop.example/orders/demo-1234/return
       (this demo does NOT open that URL. The rail stub answers the status query as though the payer had completed the page — the browser return trip is a named gap, docs/runbooks/demo.md)
     ✔ GET /v1/payment_intents/pi_8wxevnfggh499dkcnz23d11a — identical object, so the `requires_action` a merchant was told is the `requires_action` vpay stored
     … polling until it leaves `requires_action` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_8wxevnfggh499dkcnz23d11a
       status         requires_payment_method
       amount         5001 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       failure_code   payer_timeout   (charges.failure_code, the closed vocabulary of docs/flows/failures.md)
       message        The payment was declined (payer_timeout).
       the rail's own raw words are in charges.failure_raw and in the worker's log; only the taxonomy code and this generic message are public
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_ddswvx3m6n2h18a8391w0nnd
       event.type     payment_intent.payment_failed
       livemode       false
       data.object.id pi_8wxevnfggh499dkcnz23d11a
       data.object.status requires_payment_method

  ── 6/6  orange_money · the rail refuses, and documents no reason for it ─────────────────────────
     selected by: 5002 XAF selects demo-outcomes.json's FAILED mapping. Orange documents no sub-reason vocabulary for FAILED, so the adapter refuses to guess: `provider_error` carrying the raw text
     ✔ POST /v1/payment_intents
       id             pi_c6gyz365d12b504jyk361fn7
       status         requires_payment_method
       amount         5002 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
     ✔ GET /v1/payment_intents/pi_c6gyz365d12b504jyk361fn7 — identical object
     ✔ POST /v1/payment_intents/pi_c6gyz365d12b504jyk361fn7/confirm — HTTP 200, the rail accepted the charge
       id             pi_c6gyz365d12b504jyk361fn7
       status         requires_action
       amount         5002 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       next_action    redirect_to_url — send the payer here:
         url          http://localhost:8082/stub-hosted-page/pay-dfc2c926-dfd7-49b8-ac13-afb0bba0cc8d
         return_url   https://shop.example/orders/demo-1234/return
       (this demo does NOT open that URL. The rail stub answers the status query as though the payer had completed the page — the browser return trip is a named gap, docs/runbooks/demo.md)
     ✔ GET /v1/payment_intents/pi_c6gyz365d12b504jyk361fn7 — identical object, so the `requires_action` a merchant was told is the `requires_action` vpay stored
     … polling until it leaves `requires_action` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_c6gyz365d12b504jyk361fn7
       status         requires_payment_method
       amount         5002 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       failure_code   provider_error   (charges.failure_code, the closed vocabulary of docs/flows/failures.md)
       message        The payment was declined (provider_error).
       the rail's own raw words are in charges.failure_raw and in the worker's log; only the taxonomy code and this generic message are public
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_2n6x4aq9e14db13yr92bhcgg
       event.type     payment_intent.payment_failed
       livemode       false
       data.object.id pi_c6gyz365d12b504jyk361fn7
       data.object.status requires_payment_method

      what just happened, in one table:
        #   rail         intent                      status                   failure_code
        1   mtn_momo     pi_x70mabwa394396hp5j40hs0z succeeded                —
        2   mtn_momo     pi_q114t7cjm57hx9pgxth7q73p requires_payment_method  insufficient_funds
        3   mtn_momo     pi_wb6kzf1nk54kx2ag81vpmt3p requires_payment_method  payer_timeout
        4   orange_money pi_41w8es8my13jb3223e30tan9 succeeded                —
        5   orange_money pi_8wxevnfggh499dkcnz23d11a requires_payment_method  payer_timeout
        6   orange_money pi_c6gyz365d12b504jyk361fn7 requires_payment_method  provider_error

      the callback route exists — `POST /provider/{code}/callback` — but this demo's rail stubs never call it, so every settlement above came from the worker's own authenticated query_status; a callback would only have been a hint that pulled that same poll forward (docs/flows/provider-port.md)

✔ all four steps behaved as expected — 6 payments on 2 rails, every one settled by the worker asking the rail and evidenced by a signed webhook.

  server      http://localhost:8080
  discovery   http://localhost:8080/v1/oauth/.well-known/openid-configuration
  receiver    http://localhost:8083/__admin/requests
  rail journal  docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml exec wiremock-mtn curl -s localhost:8080/__admin/requests
  (no dashboard: it has no data source to show — docs/runbooks/demo.md)

  tear down with: just demo_project=vpay-demo demo-down
```

## 5. What this proves, and what it does not

**Proves**, because a run that did not do it fails and exits non-zero:

- **The `private_key_jwt` handshake works end to end** against a merchant
  whose public JWK the server holds — discovery, JWKS, an access token whose
  `iss`/`aud`/`sub` are the OP's own, and a `401` with vpay's error envelope
  for the same path without one. ([ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md))
- **Both rails, six outcomes.** Push and redirect; succeeded, declined and
  expired on MTN; succeeded, expired and refused on Orange. Every intent's
  public fields are printed from the object the API actually returned.
- **The response and the stored row agree.** Every create and every confirm is
  followed by a retrieve, and the two must be the *same object* (bar
  `client_secret`, which `confirm` omits by design). A status rendered but not
  committed fails the run.
- **The failure taxonomy, not merely "it failed".** Each failing outcome
  asserts its exact `last_payment_error.code` — `insufficient_funds`,
  `payer_timeout`, `provider_error` — which is the difference between showing
  a decline and showing the adapter's mapping table working.
  ([../flows/failures.md](../flows/failures.md))
- **Settlement is the worker asking the rail.** Nothing in the demo fakes an
  approval. The `vpay-worker` container claims the `poll_charge` job the
  confirm committed *in the same transaction as the charge*, asks the stub over
  HTTP, and commits the charge, the intent and one event together.
- **The webhook a merchant actually receives.** Read out of the receiver's own
  request journal (`GET /__admin/requests` — the merchant-side view, not
  vpay's belief about what it sent), matched on `Vpay-Event-Id` *and* the
  intent id in the body, `Stripe-Signature` asserted byte-identical to
  `Vpay-Signature`, and the recorded bytes verified with
  `vpay_sdk::webhooks::verify` — the same call a merchant's handler makes. The
  verified event's `type` is asserted, so a run in which every payment was
  delivered as `payment_intent.succeeded` could not pass.
- **The redirect rail hands over a real URL.** Outcome 4 prints
  `next_action.redirect_to_url`, committed before the response was built.

**Does not prove**, each named so it is a decision and not an omission:

- **That MTN or Orange work.** No real rail endpoint has ever been called by
  this code. Every outcome is chosen at a WireMock stub, by a field of the
  request a merchant genuinely controls — the MSISDN on MTN (a `GET` status
  query steers no other way), the amount on Orange (whose status query is a
  `POST` carrying it). **Nothing rewrites stored state to force an outcome.**
- **That a payer can complete Orange's hosted page.** The demo prints the URL
  and does not open it; the stub then answers the status query as though the
  payer had finished. The browser return trip is a named gap — a product
  contract for the bounce URL is needed first.
- **That a rail can call us.** There is no `POST /provider/{code}/callback`.
  Every settlement above came from vpay asking, never from being told. The
  walkthrough prints that sentence itself.
- **`amount_received`.** The settlement transaction writes the column; the
  `payment_intent` object does not carry it, so neither a merchant's client
  nor this demo can see it. The demo says so rather than reading the database
  behind the API it exists to demonstrate.
- **Anything about a deployment.** No cluster has ever run vpay.
- **That the demo is a build gate.** It is an assertion harness a human reads.
  Nothing in CI fails if it regresses; the closest thing is
  `just stripe-compat`, which drives the official `stripe` package against the
  same stack.

## 6. The dashboard is out of scope, and why

Issue #11 asks that the dashboard either show the intents the walkthrough
created *or* that the issue state why it is out of scope. It is out of scope,
and the reason is not that the screen is unfinished:

**There is no data source to show.** Per [../status.md](../status.md) the
dashboard renders a static scaffold notice and makes no call to `vpay-server`;
`/dash/v1` does not exist (Phase 2b, not started). A demo that booted it would
be inviting a reader to look at a screen that *cannot* show the six payments
just made, which is worse than not booting it. `just demo-up` therefore starts
six services and not seven, and building `frontends/Dockerfile` for it would
cost minutes the demo does not buy anything with.

`docker compose -f compose.yml -f compose.e2e.yml up` still starts it if you
want to look at the scaffold.

## 7. Two demos on one machine

**What is isolated, measured on 2026-09-04** by bringing a second stack up
beside the first (`just demo_project=vpay-demo-b demo_port=18088
demo_receiver_port=18089 demo-up`) and running both walkthroughs concurrently:

```console
$ docker ps --filter name=vpay-demo --format '{{.Names}}\t{{.Ports}}' | sort
vpay-demo-b-postgres-1          5432/tcp
vpay-demo-b-vpay-server-1       0.0.0.0:18088->8080/tcp
vpay-demo-b-vpay-worker-1
vpay-demo-b-wiremock-mtn-1      8080/tcp, 8443/tcp
vpay-demo-b-wiremock-orange-1   8080/tcp, 8443/tcp
vpay-demo-b-wiremock-webhook-1  0.0.0.0:18089->8080/tcp
vpay-demo-postgres-1            5432/tcp
vpay-demo-vpay-server-1         0.0.0.0:18080->8080/tcp
vpay-demo-vpay-worker-1
vpay-demo-wiremock-mtn-1        8080/tcp, 8443/tcp
vpay-demo-wiremock-orange-1     8080/tcp, 8443/tcp
vpay-demo-wiremock-webhook-1    0.0.0.0:18083->8080/tcp

$ docker network ls --filter name=vpay-demo --format '{{.Name}}'
vpay-demo-b_default
vpay-demo_default

$ docker volume ls --filter name=vpay-demo --format '{{.Name}}'
vpay-demo-b_pgdata
vpay-demo_pgdata
```

Two projects, two networks, two volumes, two databases with different rows in
them (4 payment intents in one, 6 in the other), and **only the two intended
host ports published per stack** — Postgres and both rail stubs publish nothing
(`ports: !reset []` in `compose.demo.yml`), which is what makes a second stack
possible at all: 5432, 8081 and 8082 are fixed literals in `compose.yml` and
two stacks would collide on them however the three variables were set.

`vpay-demo-b`'s walkthrough ran all six outcomes green while `vpay-demo` was
up, so the stacks genuinely do not interfere at the Compose layer.

**What is NOT isolated, and it is a real limitation rather than a caveat.**
The three variables isolate everything Compose owns. They do not isolate
`.e2e/`, which holds **one** merchant key pair and **one** profile overlay for
the whole checkout:

```console
$ just demo_project=vpay-demo-b demo_port=18088 demo_receiver_port=18089 demo-up
gen-demo-keys: .e2e/application-demo.yml was generated for a different demo_port than 18088 — regenerating the pair
gen-demo-keys: wrote .e2e/demo-merchant/oauth-signing-key.pem (3072-bit RSA, mode 0600, host-only)
gen-demo-keys: wrote .e2e/application-demo.yml — client_id=demo-merchant kid=e-xZOcqEipVJG5wrXY7DE6WHUn8S-lkfDuShnxmv1Ss
```

Because the two stacks want different `demo_port`s, bringing the second one up
**regenerates the shared merchant key pair**. The first stack's server still
holds the *old* public JWK in memory, so its walkthrough then fails at step 2:

```console
✘ step 2 (access token): the token endpoint refused this merchant with HTTP 401: {"error":"invalid_client","error_description":"Client authentication failed"}
```

So: **two demos brought up in sequence coexist and both serve; the older one's
`demo-walk` stops working from the moment the newer one's `demo-up` runs.**
Bring the second stack up *before* you start walking the first, or accept that
only the most recently generated key pair authenticates.

The fix is to key the `.e2e/` artefacts on `demo_project` the way the Compose
project is keyed. It was **not** done in this step: `.e2e/demo-merchant/oauth-signing-key.pem`
is a literal in `.github/workflows/ci.yml` (twice), in `just stripe-compat`, in
`examples/merchant-stripe-node/index.mjs`, in `sdks/stripe-compat`, and as the
default of `examples/merchant-demo`'s `VPAY_PRIVATE_KEY_FILE`, and a mistake
there fails *silently* as `invalid_client`. See `docs/plans/step8-notes/lane-a.md`.

## 8. The two hazards

Neither is closed by this demo, and issue #11 asks that they be fixed or made
explicit here. They are made explicit.

### 8.1 The rustls `CryptoProvider` panic — **closed**

`docs/status.md`, row *"rustls `CryptoProvider` process default, for
`authkestra_resource::jwt::Jwks::fetch`"*: **✅, closed 2026-09-02.** Both
binaries call `rustls::crypto::ring::default_provider().install_default()` as
the second thing in `run()`, before tracing init, so no client construction can
precede it. The workspace pins reqwest with `rustls-no-provider`, under which
`ClientBuilder::build()` *panics* if no process default was installed — an
application may install one, a library may not.

A unit test per binary asserts `CryptoProvider::get_default()` is `Some`
afterwards and that a second call does not panic; emptying the function's body
fails both. `examples/merchant-demo` does the same thing in its own `main()`
for the same reason, and the run pasted above is a process that did it.

What the row still says is weaker than "proven in production": no
containerised `/v1` request had been made when it was written. **The run on
this page is one** — six confirms and thirty-odd authenticated calls through a
`FROM scratch` `vpay-server` container.

### 8.2 RUSTSEC-2023-0071 — **open, accepted, and `ignore`d**

`deny.toml`'s `[advisories] ignore` list, with its reasoning in full at
`deny.toml:14-49`. The "Marvin Attack": a timing side-channel in the `rsa`
crate's PKCS#1 v1.5 *decryption*.

- **There is no patched release.** The advisory has carried no fixed version
  since 2023, and `rsa` is an unconditional, non-optional dependency of
  `authkestra-engine`, which vpay uses to run its own OpenID Provider. It
  cannot be feature-gated away, and `authkestra-op` signs RS256 only.
- **The exposure is on-topic, not incidental**: this is the crate that signs
  the tokens the walkthrough above obtained. What limits it is that the attack
  needs a *decryption* oracle, and vpay's use is JWT signing and verification.
- **Accepted deliberately by the maintainer on 2026-08-09.** The entry genuinely
  fires — `cargo deny -L info check advisories` reports
  `note[advisory-ignored]` against `rsa v0.9.10`. Revisit if a fixed `rsa`
  appears, if authkestra gains non-RSA signing, or if the Keycloak/ZITADEL
  comparison [ADR-0009](../adr/0009-dashboard-oidc-provider.md) leaves open is
  carried out.

**Every token in the run above was signed by that crate.** That is the honest
statement of the blast radius on this page.

### 8.3 The authkestra pin, checked

Issue #11's last checklist item says `docs/status.md` cites `=0.3.4` while
`Cargo.toml` says `=0.5.4`. **Checked against the tree on 2026-09-04: the
discrepancy is gone, and neither number is current.** `Cargo.toml` pins all
four crates at `=0.7.1` (`Cargo.toml:257`, `:258`, `:262`, `:264`), and
`docs/status.md` says so — its only `=0.3.4` mention is a historical statement
about where migration `0006`'s DDL was transcribed from, not a claim about the
current pin. Nothing to reconcile; the item is answered by "already done, by
the SDK/authkestra pass".

## 9. The known flake: a real defect the demo found

**`just demo` from nothing did not go green on the authoring machine.** Six
walkthrough attempts on 2026-09-03/04: two green (six outcomes for six, exit
`0` — one is pasted in §4), four failed, always the same way:

```console
✘ orange_money · the payer completes the hosted page — confirm: confirming the payment intent: vpay API error (500): api_error — An internal error occurred. Contact support with the request id.
```

```json
{"level":"ERROR","fields":{"message":"api error","alert":true,"category":"Internal","code":"write_matched_no_row","error":"no row in charges matched ch_pk69syzy2x16s9f0wmpvx8gg, or it was no longer in the required state"}}
```

**This is not the demo's bug.** It is a race between `vpay-api`'s confirm and
`vpay-worker`'s poll job, and the demo is what exposed it:

1. `insert_charge` commits the charge in `submitting` **and** its `poll_charge`
   job in one transaction, with `run_at = OffsetDateTime::now_utc()` —
   immediately runnable (`backends/crates/vpay-api/src/v1/payment_intents.rs:1368`).
2. The confirm then calls the rail and finally CASes the charge
   `submitting` → `submitted` (`charges::mark_submitted`, `vpay-db/src/charges.rs:463`,
   `WHERE id = $1 AND state = 'submitting'`).
3. The worker is entitled to claim that job at once — `IDLE_SLEEP` is **1 s**
   (`vpay-worker/src/run_loop.rs:69`), and zero if it is already busy. It finds
   a charge in `submitting` and applies the crash-recovery table, whose
   precondition is "the process died". Nothing distinguishes *that* from a
   confirm still in flight.
4. Whichever branch it takes moves the charge, so the confirm's CAS matches no
   row and the merchant gets a `500` — with `alert: true`, so it pages.

The window is the confirm's rail call plus two commits. Normally tens of
milliseconds; **measured at 3.7 s** on a loaded machine, which is where four of
six runs lost it.

Two distinct bad outcomes were observed in the database, and the second is the
serious one:

| Rail | Branch | What the merchant got | What the database holds |
|---|---|---|---|
| MTN (push) | `RecoveryAction::Advance` — "the rail answered and the state update was lost" | `500` | intent `succeeded`, and a `payment_intent.succeeded` webhook **was delivered** |
| Orange (redirect) | `RecoveryAction::FailDeadOrder` (`vpay-worker/src/recovery.rs:179`, taken **unconditionally** for `ProviderFlow::Redirect`, with no age check) | `500` | charge `failed`, `failure_code = provider_unavailable`, `failure_raw` = *"the rail's submit response was lost before its token could be committed; the payer was never handed a redirect URL…"* — **while the confirm was in flight and holding exactly that token** |

So on a push rail a merchant is told the confirm failed and is then sent a
`succeeded` webhook; on a redirect rail a **live order is killed** and
mis-labelled `provider_unavailable`, having never been unreachable.

The `Never` branch of that same recovery table already guards against exactly
this class of mistake, with a 60-second `not_found_window` whose comment says a
count alone "would look identical to [a rail] that never got it". The
`Answered` and `Redirect` branches have no equivalent minimum age.

~~**Not fixed in this step.**~~ **Fixed 2026-09-04, later the same day (Step 8,
lane G).** The three candidates were a minimum charge age before the recovery
table applies, a first-rung delay on the poll job, and a `submitting` lease the
confirm holds; the first was taken. `recovery_step` now answers
`RecoveryAction::Wait` — reschedule on the ladder's first rung, write nothing,
ask nothing — for any `submitting` charge younger than
`RecoveryPolicy::not_found_window` (60 s), measured from `charges.created_at`.
Deleting that one predicate reproduces the failure above, including the
merchant's own error text. ~~**After the fix, `just demo` from nothing ran
green: six outcomes for six, zero `write_matched_no_row`** — measured on lane
A's rebased branch, which carried the fix.~~ **Corrected 2026-09-04: no demo
run after the fix is recorded.** One green run from nothing exists (lane A's
rebased branch, 2026-09-04, **without** lane G — it was rebased onto `068d8b7`,
master plus lanes B and D, and lane G merged later as `53f7a7e`; the race is
timing-dependent and did not fire), lane A's own earlier count was two greens
in six attempts and zero for three from nothing, lane G did not re-run the
demo. **Run on the merged branch, 2026-09-04, in the `vpay-ci` VM (code as of `4b5a9d7`, lanes G and H in):** `just demo` from nothing **six times, four green** (six outcomes for six each, exit 0; the first green is the paste in `docs/runbooks/demo.md` §4). The two failures were not the race: in both, the VM's Postgres answered single statements in 14–36 s while the host's I/O pressure was above 50 % (a second VM and two reviewer builds), and the worker's log shows the settlement and the webhook landing *after* the demo's 120 s / 30 s budgets — a `DELETE FROM jobs` at 18 s and a `COMMIT` at 14.6 s in one, `INSERT`s at 5 s each in the other. `write_matched_no_row` appeared in no run's server or worker log. The plan's bar of three from nothing is met in count, not consecutively, which is why the row stays 🟡 and this sentence says both. **What proves the fix is lane G's
tests, not the demo** (`docs/status.md`'s confirm/worker race row).

**Two things this section must still be read as saying.** The line references in
the account above (`payment_intents.rs:1368`, `charges.rs:463`,
`run_loop.rs:69`, `recovery.rs:179`) are the ones the defect was found at and
have since moved; the current ones are in `docs/status.md`'s confirm/worker race
row. And **`just demo` has not been run on the merged Step 8 gate branch** — the
one green run from nothing is lane A's, and it predates lane G's fix — so if you
see a `500` with `write_matched_no_row` here, report it: it would be the first
observation of the defect on a tree that carries the fix.

## 10. Tearing down

`just demo-down` removes the containers **and** the volumes, so the next `just
demo` starts on a freshly migrated database rather than one carrying a previous
run's rows.

It takes no port — Compose matches by project name and label — but it **does**
need the project name, which is the one thing it cannot guess:

```console
$ just demo_project=vpay-demo-b demo-down
[...]
demo-down: project vpay-demo-b is gone (containers and volumes)
$ just demo_project=vpay-demo demo-down
[...]
demo-down: project vpay-demo is gone (containers and volumes)
$ docker ps -a --filter name=vpay-demo --format '{{.Names}}'
$ docker volume ls --filter name=vpay-demo --format '{{.Name}}'
$ docker network ls --filter name=vpay-demo --format '{{.Name}}'
```

All three empty — verbatim, on 2026-09-04. `just demo-status` prints every
vpay-ish project on the machine, which is the command for "is anything of mine
still up, and whose is that other stack".

## 11. When something goes wrong

| Symptom | Cause |
|---|---|
| `500 api_error` on a confirm | Was [§9](#9-the-known-flake-a-real-defect-the-demo-found)'s confirm/worker race until it was fixed on 2026-09-04. If you see one now — check the log line's `code`: `write_matched_no_row` means that race is back and is worth reporting, anything else is a different fault. |
| `invalid_client` at step 2 | Either another `demo-up` regenerated the shared key pair ([§7](#7-two-demos-on-one-machine)), or `deployment.public_base_url` disagrees with `VPAY_BASE_URL`. Step 1 prints a note when it can see the second one coming. |
| `/healthz` never answers in 120 s | `demo-up` prints `docker compose ps` and the last 80 server log lines. Exit 78 there means a config or CLI prerequisite is missing. |
| `port is already allocated` | Something holds 8080 or 8083. Pass `demo_port=` / `demo_receiver_port=`. |
| The walkthrough hangs on settlement | `docker compose … logs vpay-worker`. A worker that is not running fails the step in under two minutes with a message saying so. |

What the demo actually sent a rail is one command:

```bash
docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml \
    exec wiremock-mtn curl -s localhost:8080/__admin/requests
```

## 12. See also

- [`examples/merchant-demo/README.md`](../../examples/merchant-demo/README.md) — the outcome table and how each outcome is steered
- [`docs/flows/payment-lifecycle.md`](../flows/payment-lifecycle.md) — why a failure is `requires_payment_method` and not `failed`
- [`docs/flows/failures.md`](../flows/failures.md) — the closed `failure_code` vocabulary the demo asserts
- [`docs/flows/crash-safety.md`](../flows/crash-safety.md) — the three kill points, and the recovery table §9 is about
- [`docs/status.md`](../status.md) — what is actually built
