# ADR-0004: Static musl binaries with mimalloc

- **Status:** Accepted
- **Date:** 2026-08-08
- **Deciders:** vpay maintainers

## Context

The runtime image for a payment gateway should have the smallest possible attack surface, and allocation-heavy async workloads benefit measurably from a better allocator than the default.

## Decision

Build `vpay-server` and `vpay-worker-bin` as statically linked `x86_64-unknown-linux-musl` binaries into `FROM scratch` images, with mimalloc as the global allocator.

## Consequences

No shell, no package manager and no glibc in the runtime image. Debugging inside the container is not possible — diagnosis happens through logs, traces and the `provider_requests` audit trail, which is where it should happen anyway. musl's allocator is slow under contention, which is precisely why mimalloc is not optional here.
