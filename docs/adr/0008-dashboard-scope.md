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

## Addendum (2026-09-20): implementation state, not a revised decision

The Decision above describes the dashboard performing per-record writes
(re-poll a charge, replay a webhook, issue a refund, annotate an unresolved
charge), each producing an `audit_log` row. None of that is built yet.
Re-measured 2026-09-20 by reading the code directly:
`backends/crates/vpay-api/src/dash/` holds only `mod.rs` and
`payment_intents.rs`, and that module's own doc comment says so in as many
words — this surface mounts `GET` and nothing else. The writes this Decision
describes are **designed but unbuilt**; `/dash/v1` is read-only today.

This is not "abandoned" and not "superseded" — that call belongs to the
maintainer, and nothing here makes it. It is a record of what exists against
a Decision that still stands.
