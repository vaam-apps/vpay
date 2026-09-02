# syntax=docker/dockerfile:1
# Static musl builds: no glibc, no shell, no package manager in the runtime
# image. See docs/adr/0004-musl-mimalloc.md.
#
# Build host note: this image is built on both amd64 and arm64 (e.g. Apple
# Silicon) hosts. `rust:*-alpine` is a multi-arch image whose toolchain is
# *already* musl-native for whichever architecture Docker pulls — the alpine
# variant's default (host) target is `x86_64-unknown-linux-musl` on an amd64
# host and `aarch64-unknown-linux-musl` on an arm64 host. That already
# satisfies ADR-0004's static-musl goal, so this Dockerfile deliberately does
# NOT `rustup target add x86_64-unknown-linux-musl` or pass `--target`: doing
# so on an arm64 host would force a cross-compile that needs a GNU-compatible
# cross-linker no `rust:*-alpine` image ships, and fails outright. Building
# for the implicit host target avoids the cross-compile entirely, on both
# architectures, with no emulation and no extra linker.
#
# Version pin: authored and verified against rustc 1.95.0. Since 2026-09-02
# rust-toolchain.toml pins `channel = "1.95.0"` too (it used to say `stable`,
# which merely *resolved* to 1.95.0 at the time) and CI reads that pin; this
# `FROM` line is the one place that cannot read the file, so bump both
# together.
# Cargo.toml's `rust-version = "1.88"` is NOT that: it is `cargo metadata`'s
# max `rust_version` across the resolved dependency graph (currently driven
# by `time` 0.3.47 and `testcontainers`/`testcontainers-modules` 0.27/0.15) —
# a theoretical floor nobody has actually compiled this workspace with, and
# rust-toolchain.toml says so explicitly (63 of 317 packages declare no
# `rust_version` at all, so the true floor could be higher yet). Pinning the
# builder to that unverified number would be trading a known-good compiler
# for a guess. This pins the one version this workspace has actually been
# built and tested against, at the exact patch + Alpine base
# (`1.95.0-alpine3.22`) rather than the floating `1.95-alpine` tag, so a
# same-tag Alpine security respin can't silently change the musl/gcc under
# this build without a deliberate version bump here.
FROM rust:1.95.0-alpine3.22 AS builder
RUN apk add --no-cache musl-dev pkgconfig
WORKDIR /build
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY .cargo ./.cargo
COPY .xtask ./.xtask
COPY backends ./backends
# Every `[workspace] members` entry in Cargo.toml must be present or cargo
# refuses to load the manifest at all ("failed to load manifest for
# workspace member `sdks/rust`") — a workspace member list is a hard list,
# not a glob that tolerates zero matches the way pnpm-workspace.yaml does
# in frontends/Dockerfile. `sdks/rust` is a member (the Rust merchant SDK,
# `publish = false`), so it ships in the build context even though neither
# binary depends on it. Reconstructing this exact context and running the
# build command below is how the omission was found, on 2026-09-02, before
# this Dockerfile had ever been built.
COPY sdks/rust ./sdks/rust
# Cache the crates.io index/tarball downloads across builds (keyed by content,
# not by layer) without caching `target/` itself: a cache mount's contents are
# not part of the final layer, so building the binary straight into a cached
# `target/` would make it vanish once the RUN exits. Registry/git caches carry
# no such requirement.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --profile dist -p vpay-server -p vpay-worker-bin

# No shell in `scratch` to run a `HEALTHCHECK CMD`, so none is declared here.
# See compose.e2e.yml for how `vpay-server`'s readiness is (and isn't)
# accounted for downstream.
FROM scratch AS server
COPY --from=builder /build/target/dist/vpay-server /vpay-server
# The deployment configuration is baked into the image (ADR-0003: a profile
# selects a *file*, and `--config` / `VPAY_CONFIG` is required — both
# binaries exit 78 without it). Secrets never live in these files: every
# credential is a `${VAR}` placeholder resolved from the environment at
# boot, so an image layer containing `config/` contains no secret. A
# config change is therefore a rebuild, not a restart; that is the trade
# accepted for an immutable, self-contained `scratch` image. Until
# 2026-09-02 nothing put a config file in this image at all, so the image
# could never have booted.
COPY config /config
ENV VPAY_CONFIG=/config/application.yml
# Numeric, non-root UID. `scratch` has no /etc/passwd, so a named USER is not
# possible, but a raw UID needs no passwd entry — the kernel only ever sees
# the number.
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/vpay-server"]

FROM scratch AS worker
COPY --from=builder /build/target/dist/vpay-worker-bin /vpay-worker
# Same config layer as `server` above, for the same reasons.
COPY config /config
ENV VPAY_CONFIG=/config/application.yml
USER 65532:65532
ENTRYPOINT ["/vpay-worker"]
