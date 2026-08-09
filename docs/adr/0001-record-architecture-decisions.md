# ADR-0001: Record architecture decisions

- **Status:** Accepted
- **Date:** 2026-08-08
- **Deciders:** vpay maintainers

## Context

This project makes several non-obvious choices whose reasoning is easy to lose. A reader six months from now needs to know not just what was chosen but what was rejected and why.

## Decision

Record every architecturally significant decision as a numbered ADR in `docs/adr/`. An ADR is immutable once accepted; to change a decision, write a new ADR that supersedes it.

## Consequences

Slightly more ceremony per decision. In exchange, no one re-litigates a settled question without first reading why it was settled, and reversals are visible rather than silent.
