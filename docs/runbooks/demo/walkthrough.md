# The demo — §4. The walkthrough

_Moved out of [docs/runbooks/demo.md](../demo.md) on 2026-09-11 by exp57, which split a 1 426-line runbook into the procedure and its steps. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a `§` cross-reference pointed at a section that is now on another page, the page it moved to._

## 4. The walkthrough

Real output of `just demo` — `demo-up` from nothing, then `demo-walk` — on the
**merged** Step 9 branch (`claude/step9-hosted-checkout` at `e22b591`, with
this commit's one-line correction to `examples/merchant-demo/src/main.rs`
applied and nothing else), **re-captured 2026-09-04 in the `vpay-ci` VM** so
that the sentence that changed is the sentence below rather than a hand-edited
paste. **One green run from nothing, not three** — the three consecutive runs
that stood here before were of the code without that correction. Five steps:
the fifth mints one hosted and one embedded Checkout Session and prints the
hosted `url` in full and the embedded secret redacted. Every amount is XAF on
both rails (the demo overlay; the real MTN sandbox rejects XAF, see §"One
currency"). Verbatim and complete from the program's first line to its last;
nothing below was written by hand. **It is a 2026-09-04 capture and it is
left as one**: issue #58 changed the `selected by:` line outcome 4/6 prints and
added a ten-second rung to its settlement, so a run today prints different
bytes there. Hand-editing a transcript labelled verbatim would be worse than
the staleness; the current behaviour is §"The demo's test numbers" below. The `demo-up` output above it (image
builds, `docker compose up --wait`) is the same as §3's and is not repeated.

```console
vpay merchant demo
  base URL     http://localhost:8080   (VPAY_BASE_URL)
  client_id    demo-merchant   (VPAY_CLIENT_ID)
  private key  .e2e/demo-merchant/oauth-signing-key.pem   (VPAY_PRIVATE_KEY_FILE)
  receiver     http://localhost:8083   (VPAY_RECEIVER_URL)

[1/5] discovery + JWKS
  ✔ GET /v1/oauth/.well-known/openid-configuration
      issuer          http://localhost:8080/v1/oauth
      token_endpoint  http://localhost:8080/v1/oauth/token
      jwks_uri        http://localhost:8080/v1/oauth/jwks.json
  ✔ GET /v1/oauth/jwks.json — 1 key(s)
      kid=YnD_53uX6sBnWOHAbz_TFjEMeYOLGEGyUcctkh5BIxw  alg=RS256  kty=RSA

[2/5] access token (client_credentials + private_key_jwt)
  ✔ POST http://localhost:8080/v1/oauth/token — HTTP 200, token_type=Bearer, expires_in=900
      decoded (UNVERIFIED) claims — the token itself is never printed:
        iss  http://localhost:8080/v1/oauth
        aud  vpay:v1
        sub  demo-merchant
        exp  1788536226

[3/5] the same path with no bearer token
  ✔ GET /v1/payment_intents/pi_demo without a token — HTTP 401
      error.type     authentication_error
      error.code     missing_bearer_token
      error.message  No Authorization header was provided. Send an OAuth2 access token as 'Authorization: Bearer <token>'.

[4/5] 6 payments, on both rails, to every outcome each rail documents
      each one: create → retrieve → confirm → the worker settles it → the signed webhook it produced

  ── 1/6  mtn_momo · the payer approves on their handset ─────────────────────────
     selected by: MSISDN 237600000ce0 enters the `mtn-e2e-poll` scenario (requesttopay-scenario.json): PENDING on the first status query, SUCCESSFUL on the second
     ✔ POST /v1/payment_intents
       id             pi_sjt5sp38m962z3xgs66vrxq7
       status         requires_payment_method
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
     ✔ GET /v1/payment_intents/pi_sjt5sp38m962z3xgs66vrxq7 — identical object
     ✔ POST /v1/payment_intents/pi_sjt5sp38m962z3xgs66vrxq7/confirm — HTTP 200, the rail accepted the charge
       id             pi_sjt5sp38m962z3xgs66vrxq7
       status         processing
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       next_action    null   (a push rail prompts the handset; there is nothing for a browser to do)
     ✔ GET /v1/payment_intents/pi_sjt5sp38m962z3xgs66vrxq7 — identical object, so the `processing` a merchant was told is the `processing` vpay stored
     … polling until it leaves `processing` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 7 polls — the rail was asked, and answered
       id             pi_sjt5sp38m962z3xgs66vrxq7
       status         succeeded
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       amount_received  not on the wire — the settlement transaction writes payment_intents.amount_received (= amount, 5000), but the payment_intent object does not carry it yet
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_97n0tfssp97y1ak2mmqxhqj8
       event.type     payment_intent.succeeded
       livemode       false
       data.object.id pi_sjt5sp38m962z3xgs66vrxq7
       data.object.status succeeded

  ── 2/6  mtn_momo · the payer has no balance ─────────────────────────
     selected by: MSISDN 237600000f01 arms the `mtn-demo-decline` scenario (demo-outcomes.json), which answers the next status query FAILED/NOT_ENOUGH_FUNDS
     ✔ POST /v1/payment_intents
       id             pi_jfk35g1bkd6y1be1w1wwftcp
       status         requires_payment_method
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
     ✔ GET /v1/payment_intents/pi_jfk35g1bkd6y1be1w1wwftcp — identical object
     ✔ POST /v1/payment_intents/pi_jfk35g1bkd6y1be1w1wwftcp/confirm — HTTP 200, the rail accepted the charge
       id             pi_jfk35g1bkd6y1be1w1wwftcp
       status         processing
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       next_action    null   (a push rail prompts the handset; there is nothing for a browser to do)
     ✔ GET /v1/payment_intents/pi_jfk35g1bkd6y1be1w1wwftcp — identical object, so the `processing` a merchant was told is the `processing` vpay stored
     … polling until it leaves `processing` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_jfk35g1bkd6y1be1w1wwftcp
       status         requires_payment_method
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       failure_code   insufficient_funds   (charges.failure_code, the closed vocabulary of docs/flows/failures.md)
       message        The payment was declined (insufficient_funds).
       the rail's own raw words are in charges.failure_raw and in the worker's log; only the taxonomy code and this generic message are public
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_z0j8gh4nrd6pxf4wc7wf7dz1
       event.type     payment_intent.payment_failed
       livemode       false
       data.object.id pi_jfk35g1bkd6y1be1w1wwftcp
       data.object.status requires_payment_method

  ── 3/6  mtn_momo · the prompt expires unanswered ─────────────────────────
     selected by: MSISDN 237600000f02 arms the `mtn-demo-expiry` scenario (demo-outcomes.json), which answers FAILED with the OBJECT-shaped reason COULD_NOT_PERFORM_TRANSACTION — MTN's ~5-minute PIN window
     ✔ POST /v1/payment_intents
       id             pi_fqd7rpchax5v5ddfgz92wg6f
       status         requires_payment_method
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
     ✔ GET /v1/payment_intents/pi_fqd7rpchax5v5ddfgz92wg6f — identical object
     ✔ POST /v1/payment_intents/pi_fqd7rpchax5v5ddfgz92wg6f/confirm — HTTP 200, the rail accepted the charge
       id             pi_fqd7rpchax5v5ddfgz92wg6f
       status         processing
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       next_action    null   (a push rail prompts the handset; there is nothing for a browser to do)
     ✔ GET /v1/payment_intents/pi_fqd7rpchax5v5ddfgz92wg6f — identical object, so the `processing` a merchant was told is the `processing` vpay stored
     … polling until it leaves `processing` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_fqd7rpchax5v5ddfgz92wg6f
       status         requires_payment_method
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          mtn_momo
       livemode       false
       failure_code   payer_timeout   (charges.failure_code, the closed vocabulary of docs/flows/failures.md)
       message        The payment was declined (payer_timeout).
       the rail's own raw words are in charges.failure_raw and in the worker's log; only the taxonomy code and this generic message are public
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_ysjzzy08sd74h43906tf2cgv
       event.type     payment_intent.payment_failed
       livemode       false
       data.object.id pi_fqd7rpchax5v5ddfgz92wg6f
       data.object.status requires_payment_method

  ── 4/6  orange_money · the payer completes the hosted page ─────────────────────────
     selected by: 5000 XAF is claimed by no amount-keyed mapping, so the status query falls through to transactionstatus.json's catch-all SUCCESS
     ✔ POST /v1/payment_intents
       id             pi_qkj9fhvcgd4g59jhhznsz9av
       status         requires_payment_method
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
     ✔ GET /v1/payment_intents/pi_qkj9fhvcgd4g59jhhznsz9av — identical object
     ✔ POST /v1/payment_intents/pi_qkj9fhvcgd4g59jhhznsz9av/confirm — HTTP 200, the rail accepted the charge
       id             pi_qkj9fhvcgd4g59jhhznsz9av
       status         requires_action
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       next_action    redirect_to_url — send the payer here:
         url          http://localhost:8082/stub-hosted-page/pay-3ac4e1c0-eca2-450c-83c7-28a3aa68c13c?return=https%3A%2F%2Fshop.example%2Forders%2Fdemo-1234%2Freturn&cancel=https%3A%2F%2Fshop.example%2Forders%2Fdemo-1234%2Freturn
         return_url   https://shop.example/orders/demo-1234/return
       (this demo does NOT open that URL. The rail stub answers the status query as though the payer had completed the page — the browser return trip is a named gap, docs/runbooks/demo.md)
     ✔ GET /v1/payment_intents/pi_qkj9fhvcgd4g59jhhznsz9av — identical object, so the `requires_action` a merchant was told is the `requires_action` vpay stored
     … polling until it leaves `requires_action` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_qkj9fhvcgd4g59jhhznsz9av
       status         succeeded
       amount         5000 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       amount_received  not on the wire — the settlement transaction writes payment_intents.amount_received (= amount, 5000), but the payment_intent object does not carry it yet
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_bkt0kyerdn0g31k25j209xzv
       event.type     payment_intent.succeeded
       livemode       false
       data.object.id pi_qkj9fhvcgd4g59jhhznsz9av
       data.object.status succeeded

  ── 5/6  orange_money · the hosted page expires before the payer finishes ─────────────────────────
     selected by: 5001 XAF selects demo-outcomes.json's EXPIRED mapping — the amount travels on Orange's status body, so no scenario is needed
     ✔ POST /v1/payment_intents
       id             pi_nska2pkdtx4dqcr2cn5hrwg2
       status         requires_payment_method
       amount         5001 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
     ✔ GET /v1/payment_intents/pi_nska2pkdtx4dqcr2cn5hrwg2 — identical object
     ✔ POST /v1/payment_intents/pi_nska2pkdtx4dqcr2cn5hrwg2/confirm — HTTP 200, the rail accepted the charge
       id             pi_nska2pkdtx4dqcr2cn5hrwg2
       status         requires_action
       amount         5001 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       next_action    redirect_to_url — send the payer here:
         url          http://localhost:8082/stub-hosted-page/pay-b8374817-257a-4ef3-8c8b-f547719c2e32?return=https%3A%2F%2Fshop.example%2Forders%2Fdemo-1234%2Freturn&cancel=https%3A%2F%2Fshop.example%2Forders%2Fdemo-1234%2Freturn
         return_url   https://shop.example/orders/demo-1234/return
       (this demo does NOT open that URL. The rail stub answers the status query as though the payer had completed the page — the browser return trip is a named gap, docs/runbooks/demo.md)
     ✔ GET /v1/payment_intents/pi_nska2pkdtx4dqcr2cn5hrwg2 — identical object, so the `requires_action` a merchant was told is the `requires_action` vpay stored
     … polling until it leaves `requires_action` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_nska2pkdtx4dqcr2cn5hrwg2
       status         requires_payment_method
       amount         5001 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       failure_code   payer_timeout   (charges.failure_code, the closed vocabulary of docs/flows/failures.md)
       message        The payment was declined (payer_timeout).
       the rail's own raw words are in charges.failure_raw and in the worker's log; only the taxonomy code and this generic message are public
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_aa1qbqw6zx5z3f9hvjp3hw9d
       event.type     payment_intent.payment_failed
       livemode       false
       data.object.id pi_nska2pkdtx4dqcr2cn5hrwg2
       data.object.status requires_payment_method

  ── 6/6  orange_money · the rail refuses, and documents no reason for it ─────────────────────────
     selected by: 5002 XAF selects demo-outcomes.json's FAILED mapping. Orange documents no sub-reason vocabulary for FAILED, so the adapter refuses to guess: `provider_error` carrying the raw text
     ✔ POST /v1/payment_intents
       id             pi_cjx973q83n5hk11wdrm4jypk
       status         requires_payment_method
       amount         5002 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
     ✔ GET /v1/payment_intents/pi_cjx973q83n5hk11wdrm4jypk — identical object
     ✔ POST /v1/payment_intents/pi_cjx973q83n5hk11wdrm4jypk/confirm — HTTP 200, the rail accepted the charge
       id             pi_cjx973q83n5hk11wdrm4jypk
       status         requires_action
       amount         5002 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       next_action    redirect_to_url — send the payer here:
         url          http://localhost:8082/stub-hosted-page/pay-9d1213c4-a5d3-428d-8f53-53e179d810b4?return=https%3A%2F%2Fshop.example%2Forders%2Fdemo-1234%2Freturn&cancel=https%3A%2F%2Fshop.example%2Forders%2Fdemo-1234%2Freturn
         return_url   https://shop.example/orders/demo-1234/return
       (this demo does NOT open that URL. The rail stub answers the status query as though the payer had completed the page — the browser return trip is a named gap, docs/runbooks/demo.md)
     ✔ GET /v1/payment_intents/pi_cjx973q83n5hk11wdrm4jypk — identical object, so the `requires_action` a merchant was told is the `requires_action` vpay stored
     … polling until it leaves `requires_action` (the worker is asking the rail; the ladder's first rung is 10s)
     ✔ settled after 2 polls — the rail was asked, and answered
       id             pi_cjx973q83n5hk11wdrm4jypk
       status         requires_payment_method
       amount         5002 XAF   (integer minor units — docs/flows/money.md)
       rails          orange_money
       livemode       false
       failure_code   provider_error   (charges.failure_code, the closed vocabulary of docs/flows/failures.md)
       message        The payment was declined (provider_error).
       the rail's own raw words are in charges.failure_raw and in the worker's log; only the taxonomy code and this generic message are public
     ✔ the receiver recorded a POST, and its Vpay-Signature verifies with vpay-sdk (Stripe-Signature is byte-identical)
       event.id       evt_sx8kj81n5h7x95yr81stey29
       event.type     payment_intent.payment_failed
       livemode       false
       data.object.id pi_cjx973q83n5hk11wdrm4jypk
       data.object.status requires_payment_method

      what just happened, in one table:
        #   rail         intent                      status                   failure_code
        1   mtn_momo     pi_sjt5sp38m962z3xgs66vrxq7 succeeded                —
        2   mtn_momo     pi_jfk35g1bkd6y1be1w1wwftcp requires_payment_method  insufficient_funds
        3   mtn_momo     pi_fqd7rpchax5v5ddfgz92wg6f requires_payment_method  payer_timeout
        4   orange_money pi_qkj9fhvcgd4g59jhhznsz9av succeeded                —
        5   orange_money pi_nska2pkdtx4dqcr2cn5hrwg2 requires_payment_method  payer_timeout
        6   orange_money pi_cjx973q83n5hk11wdrm4jypk requires_payment_method  provider_error

      the callback route exists — `POST /provider/{code}/callback` — but this demo's rail stubs never call it, so every settlement above came from the worker's own authenticated query_status; a callback would only have been a hint that pulled that same poll forward (docs/flows/provider-port.md)

[5/5] one hosted and one embedded Checkout Session (Step 9, D1/D6)
      each on its own fresh PaymentIntent: a session requires one in requires_payment_method with no charge, and every intent above has a charge

      ✔ POST /v1/payment_intents   (hosted)
        id             pi_sc0q0ysf6d1f335987try64c
        status         requires_payment_method
        amount         5000 XAF   (integer minor units — docs/flows/money.md)
        rails          mtn_momo, orange_money
        livemode       false
      ✔ POST /v1/checkout/sessions (hosted)
      ✔ GET  /v1/checkout/sessions/cs_938t20sg8x07x5c08nh7nk7f  (identical)

      HOSTED — open this in a browser:

        http://localhost:3080/c/cs_938t20sg8x07x5c08nh7nk7f?key=pk_test_demomerchantsandbox01#cs_938t20sg8x07x5c08nh7nk7f_secret_1ex13vm04s2bhbk6rbxyvahpkh79zfkd

      That URL's #fragment IS the session's client_secret (D6). It is printed here in full and NOWHERE else — it is not logged, and the SDK's own Debug for CheckoutSession redacts it (`http://localhost:3080/c/cs_938t20sg8x07x5c08nh7nk7f?key=pk_test_demomerchantsandbox01#[67 chars redacted]`).
      A fragment never leaves the browser: it is not sent to a server, not written to an access log, and not carried across the rail's redirect — which is why the return page gets its own weaker `return_token` in a query string instead.

      ✔ POST /v1/payment_intents   (embedded)
        id             pi_rk9n6731210218yzwkzd93yp
        status         requires_payment_method
        amount         5000 XAF   (integer minor units — docs/flows/money.md)
        rails          mtn_momo, orange_money
        livemode       false
      ✔ POST /v1/checkout/sessions (embedded)
      ✔ GET  /v1/checkout/sessions/cs_n9m7r87m1n4vsbrmkf8bfnhj  (identical)

      EMBEDDED — what a merchant's own page does with it:

        import { initEmbeddedCheckout } from '@vaam-apps/vpay-stripe-js';
        const checkout = await initEmbeddedCheckout({
          publishableKey: 'pk_test_demomerchantsandbox01',
          fetchClientSecret: async () => '[67 chars redacted]',
        });
        checkout.mount('#vpay-checkout');

      The secret above is REDACTED on purpose — the same treatment step 2 gives the access token. It is a live payer credential, this output ends up in CI logs and in pasted terminal transcripts, and a demo that printed it would be teaching the habit.
      Read the real one with:  vpay.checkout().sessions().retrieve("cs_n9m7r87m1n4vsbrmkf8bfnhj")
      It only mounts from an origin in this merchant's `checkout_origins` (D4): vpay serves `Content-Security-Policy: frame-ancestors <that list>` on the embedded page, and the page independently compares its own framer against the same list. The second of those is the one a browser has been observed performing — see docs/runbooks/checkout.md §5.

      what just happened, in one table:
        ui_mode    session                     payment_intent              status   payment_status url
        hosted     cs_938t20sg8x07x5c08nh7nk7f pi_sc0q0ysf6d1f335987try64c open     unpaid         printed above
        embedded   cs_n9m7r87m1n4vsbrmkf8bfnhj pi_rk9n6731210218yzwkzd93yp open     unpaid         — (embedded sessions have none)

      NEITHER SESSION HAS BEEN PAID, and this program cannot pay one: both intents are still requires_payment_method, no rail has been called for either, and no browser has rendered either page. Open the hosted URL to change that.

✔ all five steps behaved as expected — 6 payments on 2 rails, every one settled by the worker asking the rail and evidenced by a signed webhook, plus one hosted and one embedded Checkout Session a browser can open.

  server      http://localhost:8080
  discovery   http://localhost:8080/v1/oauth/.well-known/openid-configuration
  receiver    http://localhost:8083/__admin/requests
  checkout    http://localhost:3080/healthz  (vpay's own payment page)
  shop        http://localhost:3001                (the demo merchant's storefront)
  orange stub http://localhost:8082/__admin/requests
  (an orange_money confirm answers next_action.redirect_to_url.url on that host:
   open it for the RAIL's stub hosted page, with a Pay link and a Cancel link)
  rail journal  docker compose -f compose.yml -f compose.e2e.yml -f compose.demo.yml exec wiremock-mtn curl -s localhost:8080/__admin/requests
  (no dashboard: it has no data source to show — docs/runbooks/demo.md)

  tear down with: just demo_project=vpay-demo demo-down
```
