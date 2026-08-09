# ADR-0003: Administration is YAML in git, not an admin UI

- **Status:** Accepted
- **Date:** 2026-08-08
- **Deciders:** vpay maintainers

## Context

Merchant onboarding, rail credentials and host allowlists all need to change over time. An admin UI makes those changes fast, unreviewed and unversioned — the wrong properties for configuration that decides where real money goes.

## Decision

All administration lives in YAML, loaded at boot, validated, and reconciled into the database in one transaction. Spring Boot's idiom: `application.yml` plus `application-{profile}.yml` overlays, with `\${ENV}` placeholders for secrets. Validation failure means the process exits non-zero without serving traffic. The dashboard cannot change any of it.

## Consequences

Configuration gets code review, diffs and rollback. Changes take a deploy rather than a click, which is the point. A half-configured process never serves traffic.
