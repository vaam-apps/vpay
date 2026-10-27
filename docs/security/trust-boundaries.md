# Security: Trust Boundaries

_Issue [#150](https://github.com/vaam-apps/vpay/issues/150), [EPIC #155](https://github.com/vaam-apps/vpay/issues/155). The machine-readable inventory is [`schemas/security-inventory.yaml`](../../schemas/security-inventory.yaml); the architecture decision is [ADR-0034](../adr/0034-security-asset-and-control-evidence-inventory.md)._

## Overview

vpay identifies five trust tiers, Tier 0 through Tier 4. Every deployable
service in the inventory names exactly one. A transition across a boundary
requires authentication, payload validation and least-privilege authorization
where the surface provides them; where it does not, the document says so.

```text
 ┌─────────────────────────────────────────────────────────────────────────────┐
 │ TIER 0: Untrusted external networks                                         │
 │                                                                             │
 │   • Merchant systems (API clients, webhook listeners)                       │
 │   • Payers (mobile handsets, browsers)                                      │
 │   • Mobile money telcos (MTN, Orange) and aggregators (Notch Pay)           │
 │   • Identity providers (Keycloak, enterprise IdP)                           │
 └──────────────────────┬───────────────────────────────▲──────────────────────┘
                        │ Inbound HTTPS                 │ Outbound HTTPS
                        ▼                               │
 ┌──────────────────────────────────────────────────────┴──────────────────────┐
 │ TIER 1: Edge and ingress                                                    │
 │                                                                             │
 │   • Kubernetes Ingress or reverse proxy; TLS terminates here                │
 │   • The chart's NetworkPolicy is opt-in, default off, never applied         │
 └──────────────────────┬──────────────────────────────────────────────────────┘
                        │ Plain TCP from the ingress to the server
       ┌────────────────┴───────────────────────────────┐
       ▼                                                ▼
 ┌─────────────────────────────┐                  ┌────────────────────────────┐
 │ TIER 2: Application tier    │                  │ TIER 3: Frontend tier      │
 │                             │                  │                            │
 │ • vpay-server               │                  │ • checkout Next.js app     │
 │   - /v1 (signed, COSE)      │                  │   - /v1/browser only       │
 │   - /v1/oauth               │                  │ • dashboard Next.js app    │
 │   - /v1/browser             │                  │   - /dash/v1, opaque token │
 │   - /dash/v1 (staff token)  │                  │                            │
 │   - /provider (callbacks)   │                  │                            │
 │ • vpay-management (opt-in)  │                  │                            │
 │   - /dash/v1 only           │                  │                            │
 │ • vpay-worker (same image)  │                  │                            │
 └──────────────┬──────────────┘                  └────────────────────────────┘
                │ SQL over the operator's network
                ▼
 ┌─────────────────────────────────────────────────────────────────────────────┐
 │ TIER 4: Persistent state                                                    │
 │                                                                             │
 │   • PostgreSQL (the only durable store)                                     │
 │     - Ledger, transactions, events, outbox jobs, idempotency, JTI cache     │
 │     - Transport TLS and volume encryption are operator-owned assumptions    │
 │     - Tenant separation enforced in the repository layer                    │
 └─────────────────────────────────────────────────────────────────────────────┘
```

## Boundary Invariants

### 1. External caller -> `/v1` (Merchant API)

- **Protocol:** HTTP behind an operator-terminated TLS ingress. The server
  itself binds plain TCP; rustls is used for outbound calls
  ([ADR-0005](../adr/0005-rustls-only.md)).
- **Authentication:** every request is a COSE_Sign1 envelope from a merchant
  Ed25519 key registered in `merchant_clients[].cose_keys`, and every answer is
  sealed by vpay's own key
  ([ADR-0035](../adr/0035-v1-is-a-signed-message-not-a-token.md)). An envelope
  needs a valid signature from an enabled client's key, an `iat` inside the
  freshness window, and a nonce not recorded in `envelope_nonces`. Bearer
  tokens are refused on `/v1`; the OAuth OP at `/v1/oauth` still mints them
  until VP-4 deletes it, and nothing accepts them.
- **Authorization:** Scopes are checked per handler, and every read and write
  is scoped to the authenticated `merchant_id`.

### 2. Payer browser -> `/c/{cs_id}` and `/v1/browser`

- **Authentication:** The payer possesses only the ephemeral `client_secret`
  passed in the URL fragment.
- **Surface isolation:** Hosted checkout cannot invoke merchant management
  endpoints; it reads and confirms through `/v1/browser` only
  ([ADR-0022](../adr/0022-surface-isolation-and-independent-scaling.md)).

### 3. Staff operator -> `/dash/v1`

- **Authentication:** Password with Argon2id and a server-side pepper plus
  mandatory TOTP, or federated OpenID Connect SSO
  ([ADR-0030](../adr/0030-staff-sign-in-federates-to-an-oidc-provider.md)).
- **Session:** vpay sets no cookie. vpay mints an opaque bearer token and stores
  it in `staff_sessions`; the dashboard holds it in an httpOnly, Secure,
  SameSite=Lax cookie on its own origin and presents it to `/dash/v1`.
- **Authorization:** Cross-tenant access is restricted to read-only views
  ([ADR-0018](../adr/0018-cross-tenant-admin-reads.md)).

### 4. External payment rails -> `/provider/{code}/callback`

- **Notch Pay is verified.** `x-notch-signature` is checked as the hex
  HMAC-SHA256 of the raw body under the endpoint's Hash Key
  ([RFC-0014](../rfc/0014-verifying-rail-callbacks.md), PR #357), and boot
  refuses a configured Notch Pay rail without the key.
- **MTN and Orange are not.** MTN does not sign callbacks; Orange's
  `BodyToken` check is not built. Their routes are unauthenticated hints.
- **Every callback is still a hint.** The route parses identifiers only and
  pulls a poll job's `run_at` forward. The worker's authenticated status read
  is the only thing that settles a charge
  ([docs/security/threat-models/notchpay.md](threat-models/notchpay.md),
  [docs/flows/crash-safety.md](../flows/crash-safety.md)).

### 5. Outbound deliveries -> merchant webhooks

- **Authentication:** `vpay-signature: t=<timestamp>,v1=<hmac_sha256>` over the
  timestamp and body.
- **Replay protection:** The receiver rejects timestamps outside its tolerance.
- **Egress controls:** Redirects are refused, private targets are refused by
  default, and the resolved address passes the SSRF guard
  (`backends/crates/vpay-worker/src/ssrf.rs`).

### 6. Application -> PostgreSQL

- **Transport:** Operator-owned. Nothing in this repository requires TLS to
  Postgres or enforces volume encryption.
- **Backups:** Not implemented. ADR-0013 is Proposed and no restore drill has
  run (issue [#153](https://github.com/vaam-apps/vpay/issues/153)).
