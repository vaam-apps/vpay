# `vpay-api` — the signed `/v1` transport (`envelope/`)

Why `vpay_api::envelope` is shaped the way it is (VP-3). The mechanism, the
sequence of one call and what a refusal looks like are the flow,
[signed-transport.md](../../flows/signed-transport.md); the decision is
[ADR-0035](../../adr/0035-v1-is-a-signed-message-not-a-token.md). This page is
the module by module reasoning.

## The five files

| Module       | What it is                                                                                                                                        |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mod.rs`     | `V1Envelope`: the assembled `EnvelopeLayer` (mode `Required`, `mount_prefix("/v1")`) and the placement argument below                             |
| `keys.rs`    | `EnvelopeKeys`: the two Ed25519 response seeds, read from Secret-mounted files, refused when malformed or identical                               |
| `nonces.rs`  | `PgNonceStore`: cratestack's `NonceStore` over `vpay_db::EnvelopeNonces`, one atomic statement; a database that cannot answer refuses the request |
| `binding.rs` | `V1Binding`: which `V1_ROUTES` row a request addresses, the `route`, `path_params` and contract key the signature binds                           |
| `scope.rs`   | `MerchantScope`: from a verified signer to a tenant and scopes, the whole of what `require_merchant_token` was for                                |

## Placement

The layer is applied to the `/v1` router **before** it is nested, with
`Router::layer` (so axum has set `MatchedPath` and the resolver can bind the
operation) and `.mount_prefix("/v1")`. Wrapped around the whole app it would see
no matched route, let plain traffic through unprotected and refuse every COSE
body with a `415`. The OP (`/v1/oauth/*`) and the payer surface (`/v1/browser/*`)
are nested beside it and are not behind it, which is the point of both.

`track_http_metrics` stays **outside** the layer: it reads the route pattern and
the response status, never a body or a header, and a sealed response keeps its
real HTTP status, so the counters still say `401`/`404`/`409` and not a wall of
`200`s. The body limit is **inside** it: the layer buffers at most the payload's
bound plus the header, signature and framing, and the limit then bounds what the
handler reads. Nothing reads `Authorization`, the body or `Content-Type` ahead
of the layer, and `a_bearer_token_buys_nothing_on_v1` pins that.

## Why the replay cache is a table

cratestack's own stores are per process (memory) or need Redis. vpay has several
replicas, two Deployments (ADR-0022) and no Redis, so the record is
`envelope_nonces` (migration `0055`): one row per accepted `(kid, cti)` with the
instant its window closes, `iat` plus twice the skew plus a second. The claim is
one `INSERT … ON CONFLICT (id) DO UPDATE … WHERE expires_at < now() RETURNING
id`, so two replicas opening the same captured request at the same instant are
serialised by the row lock and exactly one gets a row. A row whose window has
closed and which the sweep has not reached is **taken over**, not treated as a
replay (a counter-shaped `cti` reuses small values). The comparison reads the
database's clock, as every scheduling decision in this workspace does
(ADR-0026). The worker sweeps rows that closed more than five minutes ago (`ENVELOPE_NONCE_RETENTION`), on the `jti` sweep's schedule;
correctness does not depend on it, only the table's size.

**A failed claim is a refusal.** `record_if_unseen` returns `Err` for a database
that cannot answer and the layer turns that into a `500`: the request is not
opened and the handler does not run. A replay cache that failed open would make
a Postgres outage a replay window.

## Fail closed

No optional or compatibility mode exists. A missing flag, a missing file, a file
that is not a seed, the same seed twice, a merchant key registry the layer
refuses, or two merchants sharing a key are all exit `78` at boot, before the
listener binds. Only a process whose `deployment.surfaces` lacks `business`
loads no envelope keys, because it mounts no `/v1`.

## What is proven, and where

| Claim                                                                      | Proof                                                                                                                        |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| An unsigned request, any route, is the coarse `401`                        | `every_route_refuses_an_unsigned_request_with_the_coarse_401` (walks `V1_ROUTES`), `signed_transport.rs`                     |
| A replay is refused and the handler ran once                               | `a_replayed_envelope_is_refused_the_second_time_and_the_handler_ran_once`                                                    |
| A disabled client is refused on its next call, like an unknown key         | `a_disabled_client_is_refused_on_its_next_call_like_an_unknown_one`                                                          |
| A foreign tenant's id is the same sealed `404` as one that never existed   | `a_foreign_tenant_gets_the_uniform_sealed_404`                                                                               |
| An answer under a key the client does not pin is refused                   | `an_answer_is_refused_by_a_client_pinning_the_wrong_server_key`                                                              |
| The nonce claim is atomic, spares a live row and takes over an elapsed one | `concurrent_claims_of_one_envelope_nonce_yield_exactly_one_unseen` and its three siblings in `vpay-db/tests/repositories.rs` |
| The operation table agrees with the router                                 | `vpay-api`'s parity tests over `V1_OPERATIONS`/`V1_CONTRACTS`, and `sdks/rust/src/contracts.rs`'s against `vpay-wire`        |
