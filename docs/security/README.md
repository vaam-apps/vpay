# Security

Threat models for the trust boundaries vpay opens. A new external interface,
a new store of secrets or of personal data, or a new inbound route gets one
**before** the code that opens it, which is the SSDLC requirement the epics
carry (for example vaam-apps/vpay#291 § 10).

| Boundary                       | Threat model                                           | Written    |
| ------------------------------ | ------------------------------------------------------ | ---------- |
| The `notchpay` rail, both ways | [threat-models/notchpay.md](threat-models/notchpay.md) | 2026-10-20 |

This directory is new on 2026-10-20. Until then the repository kept no threat
model anywhere. Security analysis lived in ADRs (for example
[ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
D1's hard rules), in RFCs, and in module documentation such as
`backends/crates/vpay-api/src/provider_callback.rs`'s account of what an
anonymous caller can get out of the callback route. Those are not threat
models and are not listed here. The rails that predate this directory,
`mtn_momo` and `orange_money`, have none.

**Notch Pay callbacks are signature-verified; MTN and Orange are not.** Notch
Pay's `x-notch-signature` is checked as the HMAC-SHA256 of the raw body under
the endpoint's Hash Key ([RFC-0014](../rfc/0014-verifying-rail-callbacks.md),
PR #357). MTN does not sign callbacks, and Orange's `BodyToken` check is not
built. A callback is still a hint that advances a poll; only the authenticated
status read moves money.

Personal data has its own inventory, which a gate checks:
[reference/personal-data-inventory.md](../reference/personal-data-inventory.md).

Security assets, trust boundaries and ISO 27001 control evidence are documented
in:

- [trust-boundaries.md](trust-boundaries.md) — five trust tiers and their
  crossing controls
- [data-flows.md](data-flows.md) — payment, webhook and SSO data-flow maps
- [reference/security-asset-inventory.md](../reference/security-asset-inventory.md)
  — human-facing asset and control inventory
- [../../schemas/security-inventory.yaml](../../schemas/security-inventory.yaml)
  — machine-readable inventory checked by
  `cargo xtask verify-security-inventory`
