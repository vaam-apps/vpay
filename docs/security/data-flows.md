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

### 3. Pairing a console key (ADR-0036)

```text
Console (browser)        Admin phone (vaam)        vaam (service)              vpay-server /svc/v1            Postgres
      │                         │                        │                              │                        │
      │ 1. Generate a non-extractable P-256 key; sign "vpay.console-pair.v1\n" + key + "\n" + issuedAt          │
      │ 2. Show the pairing request                       │                              │                        │
      ├────────────────────────▶│ 3. The admin approves   │                              │                        │
      │                         ├────────────────────────▶│ 4. POST /svc/v1/console_keys │                        │
      │                         │                         │    COSE_Sign1 under vaam's    │                        │
      │                         │                         │    Ed25519 key                ├───────────────────────▶│
      │                         │                         │                              │ 5. service key holds    │
      │                         │                         │                              │    console_pairing? shape,│
      │                         │                         │                              │    freshness, proof      │
      │                         │                         │                              │ 6. find or provision a  │
      │                         │                         │                              │    plain member; refuse │
      │                         │                         │                              │    an administrator     │
      │                         │                         │                              │ 7. INSERT console_keys  │
      │                         │                         │                              │    (expires = +720 h)   │
      │                         │                         │◀─────────────────────────────┤ 8. 201 {thumbprint, …}  │
```

After pairing, every `/dash/v1` request is a COSE_Sign1 message under the
console key. The signed transport's nonce store is `envelope_nonces`; the key
is looked up in `console_keys` on every request, so a revocation or expiry takes
effect on the next one. There is no session, cookie, token or authorization
code, and no identity provider.

### 4. Staff sign-in, only where `human_login` is served (ADR-0037)

```text
Staff browser     Dashboard app (server)         vpay-server /dash/v1                Postgres
      │                  │                                │                             │
      │ 1. email + password                              │                             │
      ├─────────────────▶│ 2. POST /staff/login           │                             │
      │                  ├───────────────────────────────▶│ 3. rate-limit windows,      │
      │                  │                                │    argon2id + pepper        │
      │                  │                                ├────────────────────────────▶│ 4. staff_sessions
      │                  │◀───────────────────────────────┤    (pending_totp, SHA-256   │    row
      │ 5. TOTP code     │ 6. POST /staff/totp            │     of the session token)   │
      ├─────────────────▶├───────────────────────────────▶│ 7. RFC 6238, replay guard   │
      │                  │                                ├────────────────────────────▶│    (authenticated)
      │                  │ 8. GET /oauth/authorize (PKCE) │                             │
      │                  ├───────────────────────────────▶│ 9. one 60 s single-use code │
      │                  │◀─────── 302 ?code= ────────────┤                             │
      │                  │ 10. POST /oauth/token          │                             │
      │                  ├───────────────────────────────▶│ 11. consume the code (CAS), │
      │                  │◀──── access token (Bearer) ────┤     PKCE S256               │
      │                  │ 12. GET /payment_intents       │                             │
      │                  ├───────────────────────────────▶│ 13. validate the token,     │
      │                  │                                │     re-read the member      │
```

The app follows the `302` itself, so a browser never sees a code, a verifier or
a token. The same `/dash/v1` also takes a signed console key, one credential per
request; a request that presents both is refused. Where the method is off the
sign-in routes answer `404` and `/dash/v1` takes a signature alone.
[dashboard-auth/human-login.md](../flows/dashboard-auth/human-login.md) has the
state machine and the refusals.

### 5. Staff single sign-on, only where `human_login` is served and `staff_auth.sso` is written (ADR-0030)

```text
Staff browser     Dashboard app (server)         vpay-server /dash/v1            OIDC provider (Keycloak)
      │                  │                                │                                │
      │ 1. "Sign in SSO" │ 2. POST /staff/sso/start       │                                │
      ├─────────────────▶├───────────────────────────────▶│ 3. rate limit; state, nonce,   │
      │                  │                                │    PKCE verifier; staff_sso_   │
      │                  │                                │    attempts row (hashes, the   │
      │                  │◀── pending token + provider URL┤    verifier sealed)            │
      │ 4. redirect      │                                │                                │
      │◀─────────────────┤                                │                                │
      │ 5. authenticate at the provider (password + second factor)                         │
      ├────────────────────────────────────────────────────────────────────────────────────▶│
      │ 6. redirect back to the dashboard with code and state                              │
      │◀────────────────────────────────────────────────────────────────────────────────────┤
      ├─────────────────▶│ 7. POST /staff/sso/callback    │ 8. take the flow ONCE (DELETE) │
      │                  ├───────────────────────────────▶│    compare state               │
      │                  │                                ├───────────────────────────────▶│ 9. code + verifier
      │                  │                                │◀───────────────────────────────┤    -> id_token (TLS)
      │                  │                                │ 10. signature, iss, aud, azp,  │
      │                  │                                │     exp, iat, nonce, group,    │
      │                  │                                │     second factor (acr or amr) │
      │                  │◀── new session token ──────────┤ 11. link by (iss, sub) or      │
      │                  │    then 8-11 of flow 4         │     provision a non-admin      │
```

The provider's id_token is verified, read and discarded; its access token is
never read, and vpay stays the OP for the token `/dash/v1` accepts. Every
refusal is one `401` and the reason goes to vpay's log. There is no `jti` check
in this flow: replay defence is the single-use pending flow (`state`, nonce and
PKCE verifier are bound to a row that is deleted on the first callback, and
expires after ten minutes) and the single-use authorization code that follows.
[dashboard-auth/sso.md](../flows/dashboard-auth/sso.md) has the state machine
and the refusals.

## Security Controls Embedded in Flows

1. **Replay defence:** Every verified `/v1` envelope claims its nonce in
   `envelope_nonces`, once, for the freshness window
   ([ADR-0035](../adr/0035-v1-is-a-signed-message-not-a-token.md)).
2. **Crash safety:** A rail-issued reference is persisted before the prompt
   ([ADR-0032](../adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md)).
3. **Data redaction:** Payer MSISDN and other personal data are protected in
   logs and error structures by `vpay-core::privacy` and the privacy inventory
   ([ADR-0020](../adr/0020-privacy-controls-and-evidence.md)).
4. **Staff sign-in is a deployment's choice, and a bounded one:** one answer for
   every refusal, per-email and per-source-address limits spent in
   `rate_limit_windows`, a TOTP code usable once per step, and the member
   re-read on every `/dash/v1` request
   ([ADR-0017](../adr/0017-staff-authentication.md)).
5. **Pairing is a signed service call:** only a `service_clients` key holding
   `console_pairing` can pair or revoke a console key, and a pairing can never
   create or upgrade an administrator
   ([ADR-0036](../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)).
6. **Egress control:** Merchant webhook delivery refuses redirects, private
   targets by default, and vets the resolved address before connecting.
