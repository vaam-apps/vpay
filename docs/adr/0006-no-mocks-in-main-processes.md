# ADR-0006: No test doubles in shipping processes

- **Status:** Accepted
- **Date:** 2026-08-08
- **Deciders:** vpay maintainers

## Context

A mock compiled into the server is a code path that exists only outside production. It diverges silently, and 'passes in CI, breaks in prod' becomes structurally possible. It is also how a project convinces itself a feature works.

## Decision

No mock, fake, stub or dummy implementation may be reachable from `vpay-server` or `vpay-worker-bin`. A stub rail is a **WireMock host in configuration** — the same mechanism production uses to reach a real rail. `vpay-testkit`, `wiremock`, `testcontainers` and friends may appear only under `[dev-dependencies]`. `cargo xtask verify-no-mocks` enforces this and runs in CI. Code that is not yet written returns `ProviderError::NotImplemented`, which is honest — it never fabricates a success.

## Consequences

Local development needs Docker rather than an in-process fake, which is slower to start. In exchange there is exactly one code path, so CI exercises the binary that ships. The rule is enforced mechanically, not by review discipline.
