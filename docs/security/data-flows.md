# Security: Payment and Identity Data Flows

_Issue [#150](https://github.com/vaam-apps/vpay/issues/150), [EPIC #155](https://github.com/vaam-apps/vpay/issues/155). The machine-readable inventory is [`schemas/security-inventory.yaml`](../../schemas/security-inventory.yaml); the architecture decision is [ADR-0034](../adr/0034-security-asset-and-control-evidence-inventory.md)._

## Core Data Flows

### 1. Payment Intent Confirmation (Push USSD Flow)

Used for mobile money rails (MTN MoMo, Notch Pay USSD). The **confirm request
submits to the rail synchronously**. The worker polls and, when the submit
outcome is unknown, resubmits; it does not initiate the charge.

```text
Merchant              vpay-server             Postgres               Payment Rail          Payer Handset
   │                       │                      │                       │                      │
   │ POST /v1/payment_intents/{id}/confirm         │                       │                      │
   ├──────────────────────▶│                      │                       │                      │
   │                       │ Commit charge,       │                       │                      │
   │                       │ intent, request row  │                       │                      │
   │                       ├─────────────────────▶│                       │                      │
   │                       │ Commit tx            │                       │                      │
   │                       │◀─────────────────────┤                       │                      │
   │                       │ Submit the charge (TLS)                      │                      │
   │                       ├─────────────────────────────────────────────▶│                      │
   │                       │                                              │ Send USSD prompt     │
   │                       │                                              ├─────────────────────▶│
   │                       │                                              │                      │ Payer enters
   │                       │                                              │                      │ PIN on phone
   │                       │                                              │◀─────────────────────┤
   │  200 OK               │                                              │                      │
   │◀──────────────────────┤                                              │                      │
   │                       │                      │                       │                      │
   │                  vpay-worker                 │                       │                      │
   │                       │ Claim poll_charge job│                       │                      │
   │                       │ (SKIP LOCKED)        │                       │                      │
   │                       ├─────────────────────▶│                       │                      │
   │                       │ Authenticated status read (TLS)              │                      │
   │                       ├─────────────────────────────────────────────▶│                      │
   │                       │◀─────────────────────────────────────────────┤                      │
   │                       │ Write terminal status,                       │                      │
   │                       │ insert webhook job, ledger entry              │                      │
   │                       ├─────────────────────▶│                       │                      │
```

A rail callback does not replace the status read. `/provider/{code}/callback`
parses identifiers and moves a poll job's `run_at` forward; the authenticated
status read is what settles the charge.

### 2. Merchant Webhook Delivery Flow

Guarantees at-least-once signed delivery of lifecycle events such as
`payment_intent.succeeded`:

```text
    Postgres (jobs + webhook_deliveries)   vpay-worker                Merchant Webhook Endpoint
              │                              │                                  │
              │  Claim deliver_webhook job   │                                  │
              │◀─────────────────────────────┤                                  │
              │                              │                                  │
              │  SSRF guard on the resolved address; redirects refused           │
              │  Format JSON payload and HMAC-SHA256 signature                   │
              │  POST payload (vpay-signature header)                            │
              │                              ├─────────────────────────────────▶│
              │                              │                                  │ 2xx
              │                              │◀─────────────────────────────────┤
              │  Mark webhook_deliveries row completed                              │
              │◀─────────────────────────────┤                                  │
```

A failed attempt is recorded on the `webhook_deliveries` row and rescheduled
with exponential backoff. `provider_requests` is the rail-call journal, not the
webhook-delivery journal.

### 3. Staff Single Sign-On (OIDC Federation)

```text
Staff Browser           Next.js Dashboard               vpay-server                      OIDC IdP (Keycloak)
      │                         │                            │                                    │
      │ 1. Click "Sign in SSO"  │                            │                                    │
      ├────────────────────────▶│                            │                                    │
      │                         │ 2. POST /dash/v1/staff/sso/start                                 │
      │                         ├───────────────────────────▶│                                    │
      │                         │                            │ 3. Generate state, nonce and PKCE  │
      │                         │                            │    Store staff_sso_attempts row    │
      │ 4. Redirect to IdP      │                            │                                    │
      │◀────────────────────────┤                            │                                    │
      │                                                                                           │
      │ 5. Authenticate at IdP (username + password + org MFA)                                   │
      ├──────────────────────────────────────────────────────────────────────────────────────────▶│
      │                                                                                           │
      │ 6. Redirect to the dashboard callback with code and state                               │
      │◀──────────────────────────────────────────────────────────────────────────────────────────┤
      │                                                                                           │
      │ 7. Exchange code via BFF                                                                 │
      ├────────────────────────▶│ 8. POST /dash/v1/staff/sso/callback                            │
      │                         ├───────────────────────────▶│                                    │
      │                         │                            │ 9. Exchange code for ID token (TLS)│
      │                         │                            ├───────────────────────────────────▶│
      │                         │                            │◀───────────────────────────────────┤
      │                         │                            │ 10. Verify signature, iss, aud,    │
      │                         │                            │     azp, exp, iat, nonce, group,   │
      │                         │                            │     and second factor              │
      │                         │ 11. Mint an opaque token    │                                    │
      │                         │     in an httpOnly cookie   │                                    │
      │◀────────────────────────┤◀───────────────────────────┤                                    │
      │ 12. Authenticated session                                                            │
      │◀────────────────────────┤                                                                 │
```

There is no `jti` check in the ID-token flow. Replay defence in the dashboard
surface is the single-use authorization code and the server-side session row;
the `jti` replay cache belongs to merchant client assertions and lives in
`oauth_client_assertion_jtis`.

## Security Controls Embedded in Flows

1. **Replay defence:** Merchant client assertions record their `jti` in
   `oauth_client_assertion_jtis` past the validator leeway.
2. **Crash safety:** A rail-issued reference is persisted before the prompt
   ([ADR-0032](../adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md)).
3. **Data redaction:** Payer MSISDN and other personal data are protected in
   logs and error structures by `vpay-core::privacy` and the privacy inventory
   ([ADR-0020](../adr/0020-privacy-controls-and-evidence.md)).
4. **Rate limiting:** Staff sign-in and the current-password check spend from a
   shared `rate_limit_windows` budget.
5. **Egress control:** Merchant webhook delivery refuses redirects, private
   targets by default, and vets the resolved address before connecting.
