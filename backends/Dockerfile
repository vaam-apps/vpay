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
#
# ---------------------------------------------------------------------------
# Dependency caching (added 2026-09-05)
#
# Until this date the builder was one stage: copy the workspace, run one
# `cargo build`. Every layer after `COPY backends ./backends` was invalidated
# by *any* source edit, and because `target/` is deliberately NOT a cache
# mount (see the note on the build RUN below), an invalidated layer means
# recompiling all ~317 packages of the dependency graph, not just the crate
# that changed. Measured on the authoring host on 2026-09-05, on a
# purpose-made `docker-container` builder: a cold build took 254 s, and
# appending one comment line to `backends/apps/vpay-server/src/main.rs` and
# rebuilding took **260 s** — i.e. touching one file cost a full rebuild.
#
# `cargo-chef` (https://github.com/LukeMathWalker/cargo-chef) splits that in
# two. `cargo chef prepare` reduces the workspace to a *recipe*: the lockfile
# plus every member's manifest, and nothing else — no source. `cargo chef
# cook` rehydrates a skeleton workspace from that recipe (dummy `main.rs` /
# `lib.rs` / `build.rs` per member), builds it, and then discards the
# skeleton's own artifacts, leaving `target/` holding the compiled
# *dependencies*. The recipe changes only when a manifest or the lockfile
# changes, so BuildKit's content-addressed cache keeps the cook layer for
# every build that only edits source.
#
# Three rules this file's own constraints impose on that shape, each of which
# is load-bearing rather than decorative:
#
#  1. **The cook must run with the same flags as the build.** Same
#     `--profile dist`, same `--target <host triple>`, same
#     `-p vpay-server -p vpay-worker-bin`, and with `.cargo/` already copied
#     in so `+crt-static` applies. A cook under different flags writes
#     fingerprints the real build rejects and caches nothing. The two halves
#     fail differently — measured by mutation in the 2026-09-05 review pass,
#     not asserted. Dropping `--target` is **loud**: the cook dies in under a
#     second with `cannot produce proc-macro for async-trait v0.1.92 as the
#     target x86_64-unknown-linux-musl does not support these crate types` —
#     the same failure this header describes for the build itself, and for
#     the same reason (`.cargo/config.toml` is in scope by then). Dropping
#     `--profile dist` alone is **silent**: the cook succeeds in the `dev`
#     profile, exit 0, and the next `cargo build` recompiles everything —
#     48 s of wasted cook and a 255 s `dist` build, 305 s in total for what
#     costs 105 s when the flags match.
#  2. **`ARG VPAY_GIT_SHA` is declared AFTER the cook.** It changes on every
#     commit (`release.yml` passes `github.sha`), and an `ARG`/`ENV` pair
#     ahead of the cook would invalidate the dependency layer on every single
#     release build — which is the whole cache, thrown away for a label.
#     `vpay-core/build.rs`'s `cargo::rerun-if-env-changed=VPAY_GIT_SHA` is
#     unchanged and still does its job: `vpay-core` is a workspace member, so
#     it is rebuilt by the final `cargo build` regardless.
#  3. **The planner copies the same workspace directories as the builder.**
#     Not `COPY . .`: the recipe must describe the same workspace the next
#     stage compiles, and a planner that saw a different member set would
#     produce a recipe that cooks the wrong dependency graph. It is the same
#     list, for the same reason — cargo refuses to load a workspace whose
#     `members` list names a missing directory (see the per-COPY comments
#     below). Measured 2026-09-05: dropping `.xtask`/`sdks/rust` from the
#     planner fails `cargo chef prepare` outright (`failed to read
#     /build/sdks/rust/Cargo.toml`); it does not quietly shrink the recipe.
#     The one deliberate difference is `.cargo`, which only `builder` copies.
#     The recipe's `config_file` is `null` in consequence, so
#     `.cargo/config.toml` reaches the cook through that stage's own `COPY` —
#     which is also what makes a change to it invalidate the cook.
#
# `cargo chef prepare --bin` exists and is deliberately NOT used: it prunes
# the workspace to the members one binary needs, which would make the
# planner's view of the workspace differ from the builder's and break rule 3.
#
# cargo-chef is pinned to `0.1.78` — the current release (published
# 2026-08-12; `https://index.crates.io/ca/rg/cargo-chef` has nothing newer,
# and it is not yanked). Pinned exactly, not floated, for the same reason the
# `FROM` line above is: this stage compiles a tool that decides what gets
# cached, and a silent bump is a silent change to every image built here.
# `--locked` so the pin covers cargo-chef's own dependency graph too.
#
# What this does NOT change: the runtime images. cargo-chef lives in the
# `chef`/`builder` stages only; `server` and `worker` are still `FROM
# scratch`. Measured on the authoring host 2026-09-05, before and after:
# `vpay-server` 15.9 MB both times, two layers both times, and the `config/`
# layer is the same digest
# (`sha256:571fbc262e375dd7c77adf3d212534a9c2db16ccb93822b965eb6ba2b19c091b`)
# in both. `docker export` of the final image lists `config/` and
# `/vpay-server` and nothing else this build put there — the tar also carries
# the `/dev`, `/etc`, `/proc`, `/sys` and `.dockerenv` stubs the runtime
# creates for any container, which are not layers — and a grep of that
# listing for `cargo`, `chef` or `rust` matches nothing. Every number and
# digest in this paragraph was reproduced independently in review the same
# day, including on an image built from the pre-cargo-chef Dockerfile.
#
# What it costs: the `chef` stage compiles cargo-chef from source, 32-58 s
# across six runs on this host, once per change to the base image or the pin.
# The `planner` stage re-runs on every build and takes 0.1 s.
#
# **So a build with nothing cached is SLOWER than it was.** An earlier
# version of this comment said the cold path did not regress, on one unpaired
# sample; two matched pairs in the 2026-09-05 review say otherwise. Each pair
# pruned the same isolated `docker-container` builder between its two runs
# and ran them back to back: one-stage 193 s then cargo-chef 256 s, and —
# order reversed, to control for a host several agents were building on —
# cargo-chef 248 s then one-stage 212 s. Cold costs **+36 to +63 s**: the
# `cargo install` plus the cook's own front-end pass over the graph. That is
# the trade this file makes — every cold build pays ~45 s so that every warm
# build saves ~150 s. In CI the `chef` stage is one of the layers
# `cache-from: type=gha` can restore, so the install should be paid on a
# cache miss rather than on every run; like everything else about that cache,
# nobody has measured it.
#
# What it does NOT buy, stated because the number is smaller than the
# advertising: `[profile.dist]` inherits `release`, which is `lto = "fat"`
# with `codegen-units = 1`. A fat-LTO link re-consumes every dependency's
# LLVM IR, so the final `cargo build` stays expensive no matter how much of
# the graph is already compiled. The measured incremental build went from
# 260 s to 125 s — a little over half, not the near-instant rebuild
# cargo-chef gives a project that does not use fat LTO. Reproduced in review
# on the same host under heavier load: 105 s for a source-touch rebuild and
# 101 s for a sha-only one, against 193-212 s for a cold one-stage build.
# Whether `[profile.dist]` should override `release`'s `lto = "fat"` with
# `"thin"` is a real question this change deliberately does not answer: no
# ADR records the fat-LTO choice and no comment in `Cargo.toml` explains it,
# so it is a maintainer's call, not a build-plumbing one.
# ---------------------------------------------------------------------------

# The tool stage. `musl-dev`/`pkgconfig` are installed here rather than in
# `builder` so `planner` and `builder` share one apk layer; the builder needs
# them for the same reason it always did.
FROM rust:1.95.0-alpine3.22 AS chef
RUN apk add --no-cache musl-dev pkgconfig
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo install cargo-chef --locked --version 0.1.78

# Produces `recipe.json` — manifests and lockfile, no source. This stage is
# re-run by every build (its COPYs carry the source), but it compiles
# nothing: it is a few seconds of manifest parsing whose *output* is what the
# builder's cache is keyed on.
FROM chef AS planner
WORKDIR /build
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY .xtask ./.xtask
COPY backends ./backends
COPY sdks/rust ./sdks/rust
COPY examples/merchant-demo ./examples/merchant-demo
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
WORKDIR /build
# Before the cook, not after: `.cargo/config.toml` carries the
# `+crt-static` rustflags for both musl triples, and dependencies compiled
# without them are dependencies the real build discards. A change to that
# file therefore invalidates the cook, which is correct — it IS a change to
# how every dependency is compiled.
COPY .cargo ./.cargo
COPY --from=planner /build/recipe.json recipe.json
# The dependency layer. Same profile, same target, same package selection as
# the real build below; see rule 1 in the header. `--locked` is not passed:
# the recipe already carries the resolved `Cargo.lock`, and cook writes its
# own skeleton manifests, which `--locked` would reject.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    set -eu; \
    target="$(rustc -vV | sed -n 's/^host: //p')"; \
    cargo chef cook --profile dist --target "$target" \
        -p vpay-server -p vpay-worker-bin --recipe-path recipe.json

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
#
# Position matters as much as content: this pair sits AFTER the cook above
# (header rule 2). It used to be the first thing in the builder stage, which
# was harmless when there was nothing above it to invalidate.
ARG VPAY_GIT_SHA=unknown
ENV VPAY_GIT_SHA=${VPAY_GIT_SHA}
COPY Cargo.toml Cargo.lock clippy.toml ./
# Every `[workspace] members` entry in Cargo.toml must be present or cargo
# refuses to load the manifest at all ("failed to load manifest for
# workspace member `sdks/rust`") — a workspace member list is a hard list,
# not a glob that tolerates zero matches the way pnpm-workspace.yaml does
# in frontends/Dockerfile. `sdks/rust` is a member (the Rust merchant SDK,
# `publish = false`), so it ships in the build context even though neither
# binary depends on it. Reconstructing this exact context and running the
# build command below is how the omission was found, on 2026-09-02, before
# this Dockerfile had ever been built.
#
# The same list appears in `planner` above and the two must stay identical
# (header rule 3).
COPY .xtask ./.xtask
COPY backends ./backends
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
#
# That constraint is exactly why `cargo chef cook` above earns its keep:
# `target/` must live in a layer, and a layer is what BuildKit can reuse.
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
