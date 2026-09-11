# `vpay-api` — the merchant OP (`op/`)

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## The merchant OP (`op/`)

The merchant-facing OAuth2 provider behind `/v1/oauth`
([ADR-0010](../../adr/0010-merchant-auth-private-key-jwt.md),
[merchant-auth.md](../../flows/merchant-auth.md)). Four pieces, each its own module
so it can be tested on its own:

- `clients` — the `ClientStore` the OP looks merchants up in: the statically
  registered `merchant_clients` from YAML, minus anything in the
  `disabled_clients` kill switch.
- `keys` — the RS256 signing key: loaded from a file at boot, never persisted;
  its `kid` and public JWK are what `oauth_signing_keys` records and
  `/jwks.json` publishes.
- `jwks` — the vpay-owned `/jwks.json`, publishing every key in its rotation
  window from the database rather than the one key this process holds.
- `token` — the two HTTP handlers this crate writes itself: the RFC 6749 token
  endpoint and the discovery document.

`MerchantOp` is the assembly: it holds the one `OpConfig`, the one `OpStore` and
the one `TokenManager` that `token::token_handler` needs, built once at boot
from a validated `Config`, a `keys::LoadedSigningKey` and the repositories.

Nothing in this module serves the dashboard surface: `/dash/v1` login is a
separate, later step, and this OP is deliberately pinned to the one grant `/v1`
uses.

### Why vpay writes its own handlers

`authkestra-axum` ships `axum_token_handler`/`axum_discovery_handler` and vpay
does not use them, for three reasons that are all about _not_ serving surface
this deployment does not implement:

1. Its router helpers mount the authorization-code, device and userinfo
   endpoints alongside the token endpoint. vpay serves none of those (see
   `OP_GRANT_TYPES`), and a route that exists only to answer an error is a route
   an integrator can find and misread.
2. `axum_authorize_handler` needs `tower_cookies::Cookies` in the request
   extensions, so mounting that crate's OP routes drags a cookie layer into a
   router whose entire `/v1` surface is cookie-free bearer auth.
3. Its handlers reach their state through `FromRef<AppState>` for
   `Result<Arc<dyn OpStore>, AxumError>` and render their own errors, which
   would put a second error-rendering path next to `ApiError`
   ([ADR-0011](../../adr/0011-error-modelling.md) wants one).

What vpay does _not_ re-implement is the protocol itself: `token::token_handler`
calls `authkestra_op`'s own `handle_token` directly, and the status mapping it
applies is copied from `authkestra-axum-0.7.1/src/op.rs::axum_token_handler`.

Nothing on this surface reads or sets a cookie. `/v1` is bearer-token only
(ADR-0010), and the cookie-bearing half of authkestra's OP
(`axum_authorize_handler`, which requires `tower_cookies::Cookies` in the
request extensions) is not mounted, which is why `tower-cookies` is not a
dependency of this crate at all.

### The token endpoint speaks RFC 6749, not the Stripe envelope

Every other failure in this crate is rendered by `ApiError` as
`{"error":{"type","code","message"}}` (ADR-0011: one renderer). The token
endpoint is the deliberate exception, and it has to be: it is an OAuth2
authorization server endpoint, and RFC 6749 §5.2 fixes the body as
`{"error":"invalid_client","error_description":"…"}`. Every OAuth client in
existence parses that shape, including vpay's own — `sdks/rust`'s
`Client::fetch_token` tries `TokenErrorResponse` (`error` +
`error_description`) and falls through to `Error::UnexpectedResponse`
otherwise, and `sdks/nodejs` does the same. Rendering the Stripe envelope here
would make every SDK report "unexpected response" instead of "invalid_client",
which is precisely the diagnostic a merchant needs.

So `op::token` renders `authkestra_op`'s own `TokenErrorResponse` verbatim and
does not route through `ApiError`. The status mapping — `invalid_client` → 401,
everything else → 400 — is copied from `axum_token_handler`, so a merchant
integrating against vpay sees the same statuses as against any other authkestra
deployment.

### The reference copy is not a dependency

**`authkestra-axum` is deliberately not in this workspace's dependency graph**
(the three reasons above), so it is in neither `Cargo.lock` nor the local
registry cache: there is nothing on disk to diff a bump against. On an
`authkestra-op` version bump, fetch the matching reference copy yourself —

```text
https://static.crates.io/crates/authkestra-axum/authkestra-axum-<version>.crate
```

— and compare `src/op.rs::axum_token_handler` against `op::token`, whose tail as
of `0.7.1` (lines 239-247) is inlined here so the comparison target is local and
a drift is visible without a download:

```text
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(err) => {
            let status = match err.error.as_str() {
                "invalid_client" => StatusCode::UNAUTHORIZED,
                _ => StatusCode::BAD_REQUEST,
            };
            (status, Json(err)).into_response()
        }
```

`token_handler` and its `token_error_status` are together that, with `resp`/`err`
renamed. Note what is _not_ copied: the same file's
`axum_device_authorization_handler` maps `"invalid_client" |
"unauthorized_client"` to 401, and matching _that_ arm here would be a bug —
vpay serves no device-authorization endpoint, and RFC 6749 §5.2 makes
`unauthorized_client` a 400 on the token endpoint.

### Deliberate deviations from `axum_token_handler`

`token_handler` is a port of that function (read it: it is ~50 lines). Three
things are left out, each on purpose:

- **No DPoP header handling.** `axum_token_handler` reads the `DPoP` header and
  passes it to `handle_token_with_client_cert`. vpay wires no `DpopReplayStore`
  (`MerchantOp::new`), and `authkestra_op`'s `NoDpopReplayStore` fails closed —
  so a `DPoP` header would be answered `invalid_dpop_proof` rather than
  honoured. Calling `handle_token` (which passes `None`) means a `DPoP` header
  is _ignored_ instead, and the client gets a plain Bearer token it can actually
  use. Neither behaviour is DPoP support; ignoring it is the one that does not
  fail a request over an unsupported extension.
- **No mTLS client certificate.** vpay does not terminate TLS in this process
  ([ADR-0004](../../adr/0004-musl-mimalloc.md): the image is `FROM scratch` behind
  an ingress), so there is no certificate to bind a token to and RFC 8705
  `cnf.x5t#S256` is not offered.
- **No device, authorize or userinfo route.** See "Why vpay writes its own
  handlers" above.

---
