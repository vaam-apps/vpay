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
# NOT hardcode `x86_64-unknown-linux-musl`: doing so on an arm64 host would
# force a cross-compile that needs a GNU-compatible cross-linker no
# `rust:*-alpine` image ships, and fails outright.
#
# It DOES pass `--target`, set to the builder's *own* host triple (read from
# `rustc -vV` at build time, so it is never a cross-compile). The first CI
# build of this file (run 33646048616, 2026-09-02) failed without it:
# `.cargo/config.toml` sets `-C target-feature=+crt-static` for
# `x86_64-unknown-linux-musl`, and when host and target are the same triple
# and no `--target` is given, cargo applies target rustflags to *host*
# artifacts too — build scripts and proc-macros — and a proc-macro cannot be
# a static executable ("cannot produce proc-macro for `async-trait` as the
# target `x86_64-unknown-linux-musl` does not support these crate types").
# With an explicit `--target`, cargo keeps host artifacts free of target
# rustflags, which is the same reason `just build-dist` passes one. Output
# then lands under `target/<triple>/dist/`, hence the copy to `/out`.
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
# The commit this image is built from, surfaced at runtime as
# `vpay_build_info{git_sha="..."}` on /metrics. `vpay_core::metrics::git_sha`
# reads it with `option_env!` at compile time and `vpay-core/build.rs` puts it
# in cargo's fingerprint, so changing it rebuilds rather than reusing the
# previous label.
#
# `unknown` is the default and the honest one: a `docker build` nobody passed
# `--build-arg VPAY_GIT_SHA=...` to genuinely does not know its commit. There
# is deliberately no `git rev-parse` here — the build context is a COPY of
# source trees with no `.git` in it, and a sha read from the build machine's
# own checkout would describe some other tree. `.github/workflows/release.yml`
# passes `${{ github.sha }}`; `compose*.yml` and `just release-dry-run` pass
# nothing and get `unknown`.
#
# ARG and ENV both: ARG makes it settable from outside, ENV puts it in the
# environment cargo (and therefore rustc) is invoked with, which is where
# `option_env!` reads it from. An ARG alone would be invisible to the compiler.
ARG VPAY_GIT_SHA=unknown
ENV VPAY_GIT_SHA=${VPAY_GIT_SHA}
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
# `examples/merchant-demo` is a member for the same reason and with the same
# consequence: without it here the build fails at manifest load with "failed
# to load manifest for workspace member `/build/examples/merchant-demo`",
# which is exactly what happened the first time it was added (2026-09-02).
# Both the manifest AND `src/` are needed — cargo infers a package's targets
# while loading its manifest, so a manifest with no `src/main.rs` beside it is
# an error even for a package this build never compiles. Nothing in either
# shipping binary depends on it; it is two small files, and the alternative
# (dropping it from `[workspace] members`) would mean `cargo run -p
# merchant-demo` and `cargo clippy --workspace` no longer see it.
COPY examples/merchant-demo ./examples/merchant-demo
# Cache the crates.io index/tarball downloads across builds (keyed by content,
# not by layer) without caching `target/` itself: a cache mount's contents are
# not part of the final layer, so building the binary straight into a cached
# `target/` would make it vanish once the RUN exits. Registry/git caches carry
# no such requirement.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    set -eu; \
    target="$(rustc -vV | sed -n 's/^host: //p')"; \
    cargo build --profile dist --target "$target" -p vpay-server -p vpay-worker-bin; \
    mkdir -p /out; \
    cp "target/$target/dist/vpay-server" "target/$target/dist/vpay-worker-bin" /out/

# No shell in `scratch` to run a `HEALTHCHECK CMD`, so none is declared here.
# See compose.e2e.yml for how `vpay-server`'s readiness is (and isn't)
# accounted for downstream.
FROM scratch AS server
COPY --from=builder /out/vpay-server /vpay-server
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
COPY --from=builder /out/vpay-worker-bin /vpay-worker
# Same config layer as `server` above, for the same reasons.
COPY config /config
ENV VPAY_CONFIG=/config/application.yml
USER 65532:65532
ENTRYPOINT ["/vpay-worker"]
