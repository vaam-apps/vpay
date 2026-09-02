# merchant-demo

The local demo `just demo` runs. It authenticates as a registered merchant with
the shipping Rust SDK ([`sdks/rust`](../../sdks/rust/)) and walks the five
things vpay's `/v1` surface can currently do — the fifth of which is to reach a
rail and be told, honestly, that no rail adapter is written yet.

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
stack's database, thrown away by `just demo-down`. Step 5 confirms it and
expects **`501 not_implemented`**: the rail adapters are Step 3 of the
production-readiness plan, so `submit` is not written for either rail. **A
successful confirm on step 5 is a failure**, not a success: vpay cannot have
taken a payment, so a `200` there would have been fabricated
([`docs/status.md`](../../docs/status.md), [`CLAUDE.md`](../../CLAUDE.md)).

The private key it reads is a throwaway generated per checkout by `just
gen-demo-keys` into git-ignored `.e2e/`. Its public half is registered in
`.e2e/application-demo.yml`, which `compose.demo.yml` mounts into the server as
the `demo` profile overlay. Nothing here is reusable as a credential anywhere
else, and the access token itself is never printed.

See the module documentation in [`src/main.rs`](src/main.rs) for what each step
proves, and why step 2 performs its own token exchange instead of going through
`vpay_sdk::Client`.
