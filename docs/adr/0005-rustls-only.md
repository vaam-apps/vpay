# ADR-0005: rustls everywhere; native-tls is banned

- **Status:** Accepted
- **Date:** 2026-08-08
- **Deciders:** vpay maintainers

## Context

Mixing TLS backends means two trust stores, two configuration surfaces and an OpenSSL build dependency that defeats the static-musl goal.

## Decision

Every TLS client uses rustls. `deny.toml` bans `openssl`, `openssl-sys` and `native-tls` outright, so a transitive dependency that pulls one in fails CI rather than silently linking.

## Consequences

One trust store and one configuration path. A dependency that only supports native-tls cannot be adopted without an explicit, reviewed exception — which is the intended friction. TLS verification is never disabled, including against stub hosts.
