# Security: Trust Boundaries

_Issue [#150](https://github.com/vaam-apps/vpay/issues/150), [EPIC #155](https://github.com/vaam-apps/vpay/issues/155). The machine-readable inventory is [`schemas/security-inventory.yaml`](../../schemas/security-inventory.yaml); the architecture decision is [ADR-0033](../adr/0033-security-asset-and-control-evidence-inventory.md)._

## Overview

vpay identifies four distinct trust tiers across its deployed architecture. Transitions across these boundaries require explicit authentication, cryptographic verification, payload validation, and least-privilege authorization.

```text
 ┌─────────────────────────────────────────────────────────────────────────────┐
 │ TIER 0: Untrusted External Networks (Public Internet)                       │
 │                                                                             │
 │   • Merchant Systems (API Clients, Webhook Listeners)                       │
 │   • Payers (Mobile Handsets, Web Browsers)                                  │
 │   • Mobile Money Telcos (MTN, Orange) & Aggregators (Notch Pay)             │
 │   • Identity Providers (Keycloak, Enterprise IdP)                           │
 └──────────────────────┬───────────────────────────────▲──────────────────────┘
                        │ Inbound HTTPS (TLS 1.2+)      │ Outbound HTTPS (TLS)
                        ▼                               │
 ┌──────────────────────────────────────────────────────┴──────────────────────┐
 │ TIER 1: Edge & Ingress Boundary                                             │
 │                                                                             │
 │   • Kubernetes Ingress / Reverse Proxy (TLS termination, IP rate limiting)  │
 │   • Network Policy: Restricts /metrics and /healthz to cluster-internal CIDR │
 └──────────────────────┬──────────────────────────────────────────────────────┘
                        │
       ┌────────────────┴───────────────────────────────┐
       ▼                                                ▼
 ┌─────────────────────────────┐                  ┌────────────────────────────┐
 │ TIER 2: Application Tier    │                  │ TIER 3: Payer / Dash Tier  │
 │                             │                  │                            │
 │ • vpay-server               │                  │ • checkout Next.js app     │
 │   - /v1 (private_key_jwt)   │                  │   - client_secret scope    │
 │   - /dash/v1 (Staff Session)│                  │ • dashboard Next.js app    │
 │   - /provider/* (Callbacks) │                  │   - staff SSO / BFF        │
 │ • vpay-worker (Outbox Poll) │                  │                            │
 └──────────────┬──────────────┘                  └────────────────────────────┘
                │
                │ Authenticated SQL over TLS / Unix Socket
                ▼
 ┌─────────────────────────────────────────────────────────────────────────────┐
 │ TIER 4: Persistent State Boundary                                           │
 │                                                                             │
 │   • PostgreSQL (Primary Relational Store)                                   │
 │     - Authoritative ledger, transactions, events, outbox jobs               │
 │     - Encrypted storage volume at rest                                      │
 │     - Strict tenant separation enforced at repository layer                 │
 └─────────────────────────────────────────────────────────────────────────────┘
```

## Boundary Invariants

### 1. External Caller -> `/v1` (Merchant API)
- **Protocol:** HTTP/1.1 or HTTP/2 over TLS with rustls-only ([ADR-0005](../adr/0005-rustls-only.md)).
- **Authentication:** OAuth2 `private_key_jwt` ([ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md)) or bearer access tokens. Client assertions must present a valid signature from an active merchant credential, an unexpired timestamp, and a `jti` that has not been recorded in `spent_jtis` ([ADR-0028](../adr/0028-a-spent-jti-outlives-the-validators-leeway.md)).
- **Authorization:** Scopes are checked on each handler. All data reads and writes are strictly scoped to the authenticated `merchant_id`.

### 2. Payer Browser -> `/c/{cs_id}` (Hosted Checkout)
- **Authentication:** The payer possesses only the ephemeral `client_secret` passed via URL fragment (`#`).
- **Surface Isolation:** Hosted checkout cannot invoke merchant management endpoints; it communicates only with `/v1/checkout_sessions/{id}` to read payment methods and submit payer confirmations ([ADR-0022](../adr/0022-surface-isolation-and-independent-scaling.md)).

### 3. Staff Operator -> `/dash/v1` (Staff Dashboard)
- **Authentication:** Password authentication protected by Argon2id and server-side pepper, mandatory TOTP 2FA, or federated OpenID Connect SSO ([ADR-0030](../adr/0030-staff-sign-in-federates-to-an-oidc-provider.md)).
- **Session:** State is held in an encrypted, HTTP-only, SameSite session cookie with an idle timeout. Cross-tenant access is restricted to read-only views ([ADR-0018](../adr/0018-cross-tenant-admin-reads.md)).

### 4. External Payment Rails -> `/provider/*` (Callbacks)
- **Callback Verification:** Inbound callbacks from rails must verify HMAC signatures or mutual authentication where provided by the rail ([RFC-0014](../rfc/0014-verifying-rail-callbacks.md)).
- **Defensive Ingestion:** Where rails lack cryptographic signatures (e.g. unauthenticated webhooks), payloads are treated as untrusted hints: the worker must independently poll the authoritative rail status endpoint before committing settlement ([docs/security/threat-models/notchpay.md](threat-models/notchpay.md)).

### 5. Outbound Deliveries -> Merchant Webhooks
- **Authentication:** Outbound webhooks carry a cryptographic `vpay-signature` header containing timestamp `t` and HMAC-SHA256 signature `v1`.
- **Replay Protection:** Merchants verify signatures using their configured signing key and reject timestamps older than tolerance.
