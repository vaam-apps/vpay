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

Personal data has its own inventory, which a gate checks:
[reference/personal-data-inventory.md](../reference/personal-data-inventory.md).

Security assets, trust boundaries and ISO 27001 control evidence are documented in:
- [trust-boundaries.md](trust-boundaries.md) — Trust tiers and ingress/egress boundaries
- [data-flows.md](data-flows.md) — Core payment, webhook, and SSO data flow maps
- [reference/security-asset-inventory.md](../reference/security-asset-inventory.md) — Human-facing asset and control inventory
- [../../schemas/security-inventory.yaml](../../schemas/security-inventory.yaml) — Machine-readable inventory checked by `cargo xtask verify-security-inventory`

