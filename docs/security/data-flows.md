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

## Security Controls Embedded in Flows

1. **Replay defence:** Every verified `/v1` envelope claims its nonce in
   `envelope_nonces`, once, for the freshness window
   ([ADR-0035](../adr/0035-v1-is-a-signed-message-not-a-token.md)).
2. **Crash safety:** A rail-issued reference is persisted before the prompt
   ([ADR-0032](../adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md)).
3. **Data redaction:** Payer MSISDN and other personal data are protected in
   logs and error structures by `vpay-core::privacy` and the privacy inventory
   ([ADR-0020](../adr/0020-privacy-controls-and-evidence.md)).
4. **Pairing is a signed service call:** only a `service_clients` key holding
   `console_pairing` can pair or revoke a console key, and a pairing can never
   create or upgrade an administrator
   ([ADR-0036](../adr/0036-the-console-is-a-signed-message-under-a-paired-key.md)).
5. **Egress control:** Merchant webhook delivery refuses redirects, private
   targets by default, and vets the resolved address before connecting.
