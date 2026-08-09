# ADR-0002: Payment rails live behind a port, not in the core

- **Status:** Accepted
- **Date:** 2026-08-08
- **Deciders:** vpay maintainers

## Context

vpay ships MTN MoMo and Orange Money, and expects more Central African rails. If rail logic leaks into the core, every new rail becomes a core change and the core accumulates `if provider == …` branches.

## Decision

Define one `ProviderAdapter` trait (`backends/crates/vpay-provider`). The core owns the payment lifecycle, ledger, reconciliation and failure taxonomy; an adapter owns exactly one rail's wire protocol. Providers are rows in a table, never enum variants — adding a rail is an INSERT plus an adapter crate, never a schema migration. The core branches on capability *values* (`flow`, `supports_refunds`), never on a provider code.

## Consequences

Adding a rail is bounded work with a checklist. The cost is indirection: a developer tracing one rail's behaviour reads the trait first. The rule is mechanical — `if provider == "mtn_momo"` outside an adapter crate is a defect, and it is greppable.
