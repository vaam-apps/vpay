# merchant-demo

The local demo `just demo` runs. It authenticates as a registered merchant with
the shipping Rust SDK ([`sdks/rust`](../../sdks/rust/)) and walks the four
things vpay's `/v1` surface can currently do — the fourth of which is to say
that it cannot do anything else yet.

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

Exits `0` only when all four steps behave as expected, and non-zero naming the
step that did not. **A `200` on step 4 is a failure**, not a success: no `/v1`
resource route exists, so a payment intent coming back would have been
fabricated.

The private key it reads is a throwaway generated per checkout by `just
gen-demo-keys` into git-ignored `.e2e/`. Its public half is registered in
`.e2e/application-demo.yml`, which `compose.demo.yml` mounts into the server as
the `demo` profile overlay. Nothing here is reusable as a credential anywhere
else, and the access token itself is never printed.

See the module documentation in [`src/main.rs`](src/main.rs) for what each step
proves, and why step 2 performs its own token exchange instead of going through
`vpay_sdk::Client`.
