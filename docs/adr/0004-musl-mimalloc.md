# ADR-0004: Static musl binaries with mimalloc

- **Status:** Accepted; superseded in part by [ADR-0014](0014-builder-host-musl-triple.md) (the architecture named in the Decision below is now the builder's host musl triple)
- **Date:** 2026-08-08
- **Deciders:** vpay maintainers

## Context

The runtime image for a payment gateway should have the smallest possible attack surface, and allocation-heavy async workloads benefit measurably from a better allocator than the default.

## Decision

Build `vpay-server` and `vpay-worker-bin` as statically linked `x86_64-unknown-linux-musl` binaries into `FROM scratch` images, with mimalloc as the global allocator.

## Consequences

No shell, no package manager and no glibc in the runtime image. Debugging inside the container is not possible — diagnosis happens through logs, traces and the `provider_requests` audit trail, which is where it should happen anyway. musl's allocator is slow under contention, which is precisely why mimalloc is not optional here.

## Addendum (2026-09-20): implementation state, not a revised decision

The Decision above still reads "Build `vpay-server` and `vpay-worker-bin` …
into `FROM scratch` images" — two binaries, two images. That framing is
obsolete and has been since issue #77 (2026-09-07): `backends/Dockerfile` now
builds a single `FROM scratch AS server` stage producing one binary,
`vpay-server`; the worker is the same binary invoked with `args: ["worker"]`
(re-measured 2026-09-20 by reading `backends/Dockerfile` directly — no
`vpay-worker-bin` target exists, and the file's own header comment records
why the cook layer changed: "`-p vpay-server` \[…\] until issue #77 folded the
two packages into one"). This is recorded here rather than silently fixed
because the repository's convention is a dated addendum, never an edit to a
Decision already taken.

What this addendum does **not** touch: the musl target, the mimalloc
allocator, and the `FROM scratch` runtime image are all still exactly as
decided above and are unaffected by the binary count. Only the "two
binaries, two images" framing is retired.
