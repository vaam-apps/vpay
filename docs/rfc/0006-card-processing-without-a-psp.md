# RFC-0006: Card processing without a PSP — a separate vault and acquirer connectors

- **Status:** Draft
- **Author:** vpay maintainers
- **Date:** 2026-09-23
- **Related:** [RFC-0004](0004-billing-on-top-of-invoices.md) § 7 (cards
  through PSPs), [RFC-0008](0008-payment-method-routing.md) (routing),
  [ADR-0002](../adr/0002-provider-port.md) (the port),
  [ADR-0021](../adr/0021-flutter-checkout-plugin.md) (why no native card
  field), [RFC-0001](0001-settlement-and-payouts.md) (pass-through), and the
  [2026-09-23 provider evaluation](../plans/2026-09-23-card-provider-evaluation.md).

Nothing in this document is built. It proposes an **optional** component that
a deployment may run. It is off by default, and it is never linked into
`vpay-server`.

## Problem

Every card payment RFC-0004 § 7 proposes goes through a third-party PSP's
hosted page. That has two costs for an open-source project:

1. **Coverage is borrowed.** The 2026-09-23 evaluation found no PSP shown to
   accept Visa and Mastercard for a Cameroon merchant. A deployment that has an
   acquirer contract of its own, with a bank, locally or abroad, has no way to
   use it.
2. **The PSP sets the price.** A PSP's margin is on top of the acquirer's.
   An operator with a direct acquirer contract pays less, and RFC-0008's
   routing can only choose between the options that exist.

## What cannot be done, stated first

**Visa and Mastercard cannot be reached without an acquirer.** The card
networks publish no API and accept connections only from member acquirers, or
from processors certified through one. Every card payment passes through:

```text
payer ─▶ gateway ─▶ acquirer / processor ─▶ card network ─▶ issuing bank
         (vpay can                (a contract the operator
          be this)                 must hold; code cannot replace it)
```

So "raw Visa/Mastercard" in this RFC means **vpay acting as the gateway**:

- it holds the card data;
- it runs or brokers 3-D Secure;
- it speaks each acquirer's protocol directly.

It still needs an acquirer contract. What it removes is the PSP in the middle,
not the institution at the end.

**Installing this code does not make a deployment PCI DSS compliant.**
Compliance is assessed per deployment, by a Qualified Security Assessor or
through the applicable self-assessment. The repository can be _PCI-ready_:
segmented, hardened, documented, and designed so the audited surface is small.
It must say so in every place this component is described, because a payment
system that implies it is certified when it is not is this repository's
cardinal failure mode.

**"Without a local institution" is a legal question, not a technical one.** A
foreign acquirer can technically process a Cameroon merchant's cards. Whether a
Cameroon-registered merchant may lawfully be acquired offshore is not settled
here:

- BEAC's foreign-exchange rules and Règlement n° 04/18/CEMAC/UMAC/COBAC bear
  on it.
- CEMAC-issued cards clear locally through GIMAC.

The design works with any acquirer. Whether a given deployment may use a given
acquirer is open question 1.

## Proposal

### 1. A separate component: `vpay-vault`

The cardholder data environment (CDE) is **its own binary, its own
deployment, its own database and its own network segment**.

- **What it holds:** card numbers (PANs), encrypted, and nothing else a merchant
  or payer can reach through `/v1`. Security codes (CVV/CVC) are never stored,
  as the standard requires, only held in memory for the authorisation they
  accompany.
- **What vpay core holds:** a vault token, `brand`, `last4`, the leading
  digits (the BIN, which is what RFC-0008 prices by), and the expiry. The BIN
  and last four together are a _truncated_ PAN. How many leading digits may be
  kept alongside the last four depends on the card-number length and the PCI
  SSC's current guidance, and is fixed with the QSA, not here. Nothing in
  `vpay-server` ever holds a card number, so `vpay-server` stays **out of**
  PCI scope when the segmentation is validated.
- **A gate enforces the separation.** A `cargo xtask` check, in the manner of
  `verify-no-mocks`, fails if any `vpay-vault*` crate appears in
  `vpay-server`'s dependency tree, or any vpay core crate in the vault's
  beyond a small shared-types crate. A PAN reachable from the main binary is
  a regression no other gate would see.
- **Keys:**
  - Envelope encryption: each PAN is encrypted with a data key, and the data
    key with a key-encryption key held in a KMS or HSM, never on disk.
  - Keys rotate without downtime.
  - A detokenise operation is reachable only from the acquirer connectors
    inside the segment.
- **Build or adopt.** Juspay publishes an open-source card vault in Rust,
  `hyperswitch-card-vault`, under Apache-2.0, which `deny.toml` allows. Adopting
  it, or learning from it, is weighed against writing one in open question 2.
  Nothing here assumes either.

### 2. How the payer enters a card

- **v1: a vault-hosted page.** The checkout sends the payer to a page served
  from the vault's own origin, exactly as a PSP's hosted page is reached today.
  For vpay core this is the existing `Redirect` flow, the vault is **one more
  adapter behind the port** (`vpay_vault`), and the checkout needs no card
  screen.
- **Later: hosted fields.** An iframe from the vault's origin inside vpay's
  checkout, so the page looks like one surface. That needs a new
  `ProviderFlow::Embedded` and a checkout change. It is not v1.
- **The mobile SDKs** open the vault's page the way they open any redirect.
  ADR-0021's refusal of a native card field stands: a card number typed into
  an app's own widget would move every integrator to SAQ-D.

### 3. 3-D Secure

There are three ways, in order of preference:

1. **The acquirer's own 3DS.** Many acquirer APIs run authentication
   themselves. The connector passes the browser data and follows the challenge.
   Nothing in vpay needs EMVCo certification.
2. **A certified third-party 3DS server,** behind a small `ThreeDsServer`
   interface inside the vault.
3. **An open-source 3DS server of vpay's own.** Possible, but a 3DS server
   needs EMVCo compliance testing and registration with each network's
   directory server, obtained through an acquirer. That is certification work,
   not code, and it is last for that reason.

Frictionless and challenge outcomes, the authentication values, and liability
shift are recorded on the charge, because a dispute is argued from them.

### 4. Acquirer connectors

Inside the vault, an `AcquirerConnector` trait:

- `authorize`, `capture`, `void`, `refund`;
- `query_status`, the same rule as the port: the authoritative read, and a
  connector without one is refused;
- the stored-credential fields merchant-initiated charges need (the initial
  customer-initiated flag and the network transaction id).

Protocols are the acquirer's: ISO 8583 in the acquirer's own variant, or the
acquirer's REST API.

**Many acquirer specifications are confidential.** An open-source repository
cannot carry a connector written from a spec under NDA. So the vault speaks a
**documented connector protocol** (HTTP or gRPC, versioned, specified in this
repository) to connectors running as **separate processes**:

- Connectors written from public specifications live in this repository and
  join a vault conformance suite, stubbed by WireMock exactly as rails are.
- Connectors written from confidential specifications live with the operator
  who holds the contract, and speak the same protocol.
- Nothing is dynamically loaded into the vault, which keeps its audited
  surface fixed.

### 5. Card-on-file and subscriptions

A card vaulted with the payer's consent, on a customer-initiated first charge
flagged as such, can be charged later with no payer present. That is
`supports_off_session` on the `vpay_vault` adapter, and it is what makes
RFC-0004's `charge_automatically` real without depending on a PSP's token
programme. Network tokens (Visa Token Service, Mastercard MDES) are a later
step, through the acquirer.

### 6. What the operator takes on

The repository documents this list, and says the list is the operator's:

| Obligation                                                | Provided by the repository                      | The operator's         |
| --------------------------------------------------------- | ----------------------------------------------- | ---------------------- |
| Acquirer contract, and scheme registration through it     | —                                               | ✔                      |
| PCI DSS assessment (ROC by a QSA, or the applicable SAQ)  | A reference architecture and a control map      | ✔                      |
| Network segmentation of the vault                         | Helm values that isolate it; a NetworkPolicy    | ✔ validating it        |
| Key management (KMS or HSM, custody, rotation)            | The envelope-encryption code and rotation job   | ✔ the keys             |
| Quarterly external vulnerability scans, penetration tests | —                                               | ✔                      |
| Access control and logging inside the CDE                 | Structured logs with no card data; a gate on it | ✔ retention and review |
| Incident response                                         | A runbook skeleton                              | ✔                      |

### 7. How it meets the rest of vpay

- **To vpay core the vault is a rail.** `vpay_vault` implements
  `ProviderAdapter` with the `Redirect` flow in v1, and `refund`,
  `charge_stored` and `query_status` are delegated to the chosen connector.
  Rails-behind-the-port holds, and so does "branch on capability".
- **Routing:** the vault serves method type `card`. Because it knows the BIN
  before any acquirer is asked, it is where RFC-0008's **BIN-aware**
  least-cost routing happens. The vault can choose between its own acquirer
  connectors by brand, domestic or international, and price.
- **Pass-through:** the acquirer settles to the **merchant's** account under
  the merchant's own acquiring agreement, or under a payment facilitator
  arrangement the operator has licensed. The second is aggregation, and
  RFC-0001's "do not start the code before the licence conversation" applies
  to it without change.

## Alternatives considered

- **Stay PSP-only** (RFC-0004 § 7). It keeps vpay out of PCI scope entirely,
  and it remains the default. This RFC adds an option; it does not replace
  that one.
- **Adopt Hyperswitch whole** as the card layer, as one adapter. That gives
  breadth at once, but it is a large dependency the operator must run, and its
  routing and vault would sit beside vpay's instead of under them. It remains
  a good **adapter** to write under RFC-0004 § 7.
- **Card handling inside `vpay-server`.** Rejected: the whole deployment,
  including every `/v1` route and the database, would enter PCI scope, and
  every contributor would be working in a CDE.
- **A tokenisation service (VGS, Basis Theory) as the vault.** A reasonable
  deployment choice, and the connector protocol does not preclude it. The
  project should still offer a vault an operator can run themselves, or the
  open-source claim is hollow.

## Open questions

1. **May a Cameroon-registered merchant be acquired by a foreign acquirer**,
   under BEAC foreign-exchange rules and Règlement 04/18? (Counsel.)
2. **Adopt `hyperswitch-card-vault`, or write a vault?** (Maintainer, after a
   code and licence review.)
3. **Which 3DS route first** (§ 3)? (Maintainer, per acquirer.)
4. **Which acquirer first**, and is its specification public, so that its
   connector can live in this repository? (Maintainer.)
5. **GIMAC cards:** do CEMAC-issued cards need a local acquirer regardless of
   brand? (Counsel, with a local bank.)
6. **Who maintains the reference deployment** a QSA would assess, and is the
   project ever willing to run one? (Maintainer.)

## Impact on existing invariants

- **"vpay never sees a card number"** (RFC-0004 § 7, rule 2) becomes
  **"`vpay-server` never sees a card number"**, and a new gate makes that
  checkable.
- **ADR-0021's native-card-field refusal is unchanged.**
- **The port gains a rail** (`vpay_vault`) and, later, `ProviderFlow::Embedded`.
- **No mocks in shipping processes** applies to the vault as a second
  shipping binary: connector stubs are WireMock hosts in configuration.
- **Callbacks are hints** applies inside the vault: a connector's
  `query_status` is the authoritative read.
- **A second binary, a second database, a second deployment.** ADR-0022's
  surface-isolation work is the precedent for the chart shape.
