# ADR-0008: The dashboard observes; it does not administer

- **Status:** Accepted
- **Date:** 2026-08-08
- **Deciders:** vpay maintainers

## Context

Merchant API keys are bearer credentials with full payment authority and no expiry or revocation story. Anything a browser touches eventually leaks. Separately, configuration changed through a UI is unreviewed and unversioned.

## Decision

The Next.js dashboard reads state and performs per-record operations (re-poll a charge, replay a webhook, issue a refund, annotate an unresolved charge). It cannot create merchants, edit provider configuration or issue API keys — those are YAML (ADR-0003). It authenticates with OIDC sessions against a separate `/dash/v1` API and never holds a merchant secret key. Every write produces an `audit_log` row.

## Consequences

Support work that must happen at 2am is possible without a deploy; configuration changes still take a pull request. The rule that generates the boundary: the dashboard acts on records, never on configuration.
