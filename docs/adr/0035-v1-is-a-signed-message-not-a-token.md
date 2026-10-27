# ADR-0035: `/v1` is a signed message, not a token

- **Status:** Accepted; superseded in part by
  [ADR-0037](0037-authentication-methods-are-deployment-configuration.md)
  (decision 1's "no optional or compatibility mode, no token fallback" and
  decision 6's flag day now hold only for a deployment that permits `keypair`
  alone). Supersedes [ADR-0010](0010-merchant-auth-private-key-jwt.md) for
  `/v1`; the OP it describes stays for `/v1/oauth/*` until VP-4 removes it.
- **Date:** 2026-10-26
- **Deciders:** vpay maintainers; the key-only ruling is the owner's.

## Context

- vpay's merchant API authenticated with OAuth2
  `client_credentials` + `private_key_jwt`: an RS256 assertion exchanged for a
  900-second bearer token, presented on every call. The owner's ruling for the
  whole product family (vaam, vpay, vsms) is _keypairs everywhere, never
  tokens_. cratestack 0.16 ships the mechanism (COSE_Sign1 messages with the
  route, parameters, query, payload type and idempotency key bound, a replay
  cache, an axum layer, a client that seals and opens, and a callback signer
  for a key that never leaves Node).

## Decision

1. Every `/v1` request is a COSE_Sign1 message under an Ed25519 key
   registered in `merchant_clients[].cose_keys`; every answer is one under
   one of two Ed25519 response keys the client pins. `EnvelopeMode::Required`:
   there is no optional or compatibility mode, no token fallback and no
   per-merchant opt-out.
2. The replay cache is Postgres (`envelope_nonces`), not Redis or process
   memory: it must be shared by every replica and by both Deployments of
   [ADR-0022](0022-surface-isolation-and-independent-scaling.md), and vpay runs no
   Redis.
3. A refusal is the layer's one coarse `401`, unsigned and identical for
   every cause. It is no longer the Stripe-shaped error envelope; errors
   raised after the layer still are.
4. Operations are bound by a table in `vpay-wire` (37 rows, mounted `/v1`),
   and each SDK vendors a copy that a workspace test holds to it. Both merchant SDKs seal through
   **one** implementation: the Node SDK calls `sdks/rust/wasm` (the Rust
   SDK's sealing path over cratestack's client, compiled to WebAssembly)
   rather than reimplementing the binding in TypeScript.
5. The Stripe SDK integration moves from `config.authenticator` (a header
   hook) to `config.httpClient` (the seam that can carry a body), because the
   signature _is_ the request body.
6. Merchant-side rollout is a flag day: `/v1` accepts no token from the
   commit that mounts the layer, and the SDKs, the demo shop, the Cypress
   tasks, the examples and the compose and Helm key wiring move in the same
   change.

## Consequences

- A merchant's integration is a seed file and two public keys, not a client
  secret, a token cache and a re-authentication retry. A revoked merchant is
  a `disabled_clients` row, effective on the next request.
- `Authorization` is not read anywhere on `/v1`, which removes a class of
  middleware that read it ahead of the layer.
- Diagnosis is harder by design: one `401` for every cause. The checklist is
  in [the flow](../flows/signed-transport.md).
- The Node SDK needs Rust and `wasm-bindgen-cli` at build time (not for the
  merchant, who installs the published package, which ships the `.wasm`).
- A sealed `GET` carries a body, which `fetch` forbids; the Node SDK sends on
  `node:http(s)`. A browser cannot host this SDK, which was never a goal.
- `curl` can no longer call `/v1`.

## Alternatives rejected

- an optional envelope (`EnvelopeMode::Optional`)
  for a transition, because a request that may be unsigned is a request that
  is, to whoever is attacking it; a TypeScript port of the binding, because two
  implementations of one byte layout will eventually disagree about a byte;
  keeping short-lived tokens signed with Ed25519, because that is still a token.
