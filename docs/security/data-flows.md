# Security: Payment & Identity Data Flows

_Issue [#150](https://github.com/vaam-apps/vpay/issues/150), [EPIC #155](https://github.com/vaam-apps/vpay/issues/155). The machine-readable inventory is [`schemas/security-inventory.yaml`](../../schemas/security-inventory.yaml); the architecture decision is [ADR-0033](../adr/0033-security-asset-and-control-evidence-inventory.md)._

## Core Data Flows

### 1. Payment Intent Confirmation (Push USSD Flow)

Used for mobile money rails (MTN MoMo, Notch Pay USSD):

```text
Merchant              vpay-server             Postgres               Payment Rail          Payer Handset
   │                       │                      │                       │                      │
   │  POST /v1/intents/confirm (OAuth JWT)        │                       │                      │
   ├──────────────────────▶│                      │                       │                      │
   │                       │ Insert charge,       │                       │                      │
   │                       │ intent, job (outbox) │                       │                      │
   │                       ├─────────────────────▶│                       │                      │
   │                       │ Commit tx            │                       │                      │
   │                       │◀─────────────────────┤                       │                      │
   │  200 OK (processing)  │                      │                       │                      │
   │◀──────────────────────┤                      │                       │                      │
   │                       │                      │                       │                      │
   │                  vpay-worker                 │                       │                      │
   │                       │                      │                       │                      │
   │                       │ Claim pending charge │                       │                      │
   │                       │ (SKIP LOCKED)        │                       │                      │
   │                       ├─────────────────────▶│                       │                      │
   │                       │                      │                       │                      │
   │                       │ Initiate push payment request (TLS)          │                      │
   │                       ├─────────────────────────────────────────────▶│                      │
   │                       │                                              │ Send USSD prompt     │
   │                       │                                              ├─────────────────────▶│
   │                       │                                              │                      │ Payer enters
   │                       │                                              │                      │ PIN on phone
   │                       │                                              │◀─────────────────────┤
   │                       │ Poll status / receive callback (TLS)         │                      │
   │                       │◀─────────────────────────────────────────────┤                      │
   │                       │                      │                       │                      │
   │                       │ Write payment.succeeded,                     │                      │
   │                       │ insert webhook job, ledger entry             │                      │
   │                       ├─────────────────────▶│                       │                      │
```

### 2. Merchant Webhook Delivery Flow

Guarantees at-least-once signed delivery of lifecycle events:

```text
    Postgres (jobs table)         vpay-worker                      Merchant Webhook Endpoint
              │                        │                                       │
              │  Claim webhook job     │                                       │
              │◀───────────────────────┤                                       │
              │                        │                                       │
              │                        │ Format JSON payload                   │
              │                        │ Calculate HMAC-SHA256 signature       │
              │                        │ POST payload (vpay-signature header)  │
              │                        ├──────────────────────────────────────▶│
              │                        │                                       │ 200 OK
              │                        │◀──────────────────────────────────────┤
              │  Mark job completed    │                                       │
              │◀───────────────────────┤                                       │
```

If the merchant endpoint is unreachable or returns a non-2xx status, the worker records the failure attempt in `provider_requests` / `jobs` and reschedules with exponential backoff up to 24 hours.

### 3. Staff Single Sign-On (OIDC Federation)

```text
Staff Browser           Next.js Dashboard               vpay-server                      OIDC IdP (Keycloak)
      │                         │                            │                                    │
      │ 1. Click "Sign in SSO"  │                            │                                    │
      ├────────────────────────▶│                            │                                    │
      │                         │ 2. Generate PKCE & state   │                                    │
      │                         │    Store staff_sso_attempt │                                    │
      │                         ├───────────────────────────▶│                                    │
      │ 3. Redirect to IdP      │                            │                                    │
      │◀────────────────────────┤                            │                                    │
      │                                                                                           │
      │ 4. Authenticate at IdP (Username + Password + Org MFA)                                   │
      ├──────────────────────────────────────────────────────────────────────────────────────────▶│
      │                                                                                           │
      │ 5. Redirect to Dashboard with code & state                                                │
      │◀──────────────────────────────────────────────────────────────────────────────────────────┤
      │                                                                                           │
      │ 6. Exchange code via BFF                                                                 │
      ├────────────────────────▶│                                                                 │
      │                         │ 7. POST /dash/v1/sso/callback                                   │
      │                         ├───────────────────────────▶│                                    │
      │                         │                            │ 8. Exchange code for ID token (TLS)│
      │                         │                            ├───────────────────────────────────▶│
      │                         │                            │◀───────────────────────────────────┤
      │                         │                            │ 9. Verify JWT claims, aud, jti     │
      │                         │ 10. Issue session cookie   │                                    │
      │                         │◀───────────────────────────┤                                    │
      │ 11. Authenticated session│                           │                                    │
      │◀────────────────────────┤                            │                                    │
```

## Security Controls Embedded in Flows

1. **Replay Defense:** Client assertions use `jti` replay tracking in PostgreSQL (`spent_jtis`).
2. **Crash Safety:** In-flight operations persist external reference IDs prior to prompt dispatch ([ADR-0032](../adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md)).
3. **Data Redaction:** PII (payer MSISDN) is protected across all operational logs and error structures using the `vpay-core::privacy` positive projection wrappers ([ADR-0020](../adr/0020-privacy-controls-and-evidence.md)).
