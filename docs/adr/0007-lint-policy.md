# ADR-0007: A panic in a payment path is a defect

- **Status:** Accepted
- **Date:** 2026-08-08
- **Deciders:** vpay maintainers

## Context

A panic mid-payment leaves state ambiguous: the charge may or may not have reached the rail. Ambiguity about whether money moved is the most expensive failure this system can have.

## Decision

Deny `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` and `float_arithmetic` workspace-wide, and forbid `unsafe_code`. Money is integer minor units, so floating point arithmetic is a bug by construction. Tests are exempt via `clippy.toml` — a failing assertion should panic, that is how a test reports.

## Consequences

Error handling is explicit and sometimes verbose. `cargo clippy -- -D warnings` is part of `just ci`, so a new `unwrap` fails the build rather than waiting to fail a payment.
