# merchant-demo

The local demo `just demo` runs. It authenticates as a registered merchant with
the shipping Rust SDK ([`sdks/rust`](../../sdks/rust/)) and walks the five
things vpay's `/v1` surface can currently do — the fifth of which is to
confirm a payment intent against a rail and watch it move to `processing`.

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

Exits `0` only when all five steps behave as expected, and non-zero naming the
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
because no adapter implemented `submit`. It now expects the opposite. What
has *not* changed is what the demo is careful not to claim: `processing` is
not `succeeded`, nothing polls the charge, and the rail is a stub — a
WireMock host reached over HTTP exactly as a real rail would be, never a
linked implementation ([ADR-0006](../../docs/adr/0006-no-mocks-in-main-processes.md)).
**MTN and Orange have never been called by this code.** A demo that ever
printed a *succeeded* intent would have fabricated it
([`docs/status.md`](../../docs/status.md), [`CLAUDE.md`](../../CLAUDE.md)).

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
