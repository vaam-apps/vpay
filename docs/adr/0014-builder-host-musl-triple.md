# ADR-0014: The builder's host musl triple, not a hardcoded x86_64 one

- **Status:** Accepted
- **Date:** 2026-09-03
- **Deciders:** vpay maintainers
- **Supersedes:** [ADR-0004](0004-musl-mimalloc.md), in part — only the
  architecture named in its Decision. Everything else ADR-0004 decided
  (static musl, `FROM scratch`, mimalloc) stands unchanged.

## Context

[ADR-0004](0004-musl-mimalloc.md)'s Decision names one triple:
"statically linked `x86_64-unknown-linux-musl` binaries". Two things have
happened since.

`backends/Dockerfile` stopped agreeing with it. Its header explains why at
length: `rust:*-alpine` is a multi-arch image whose toolchain is already
musl-native for whichever architecture Docker pulled, and hardcoding
`x86_64-unknown-linux-musl` on an arm64 host would force a cross-compile
needing a GNU-compatible cross-linker no `rust:*-alpine` image ships — it
"fails outright". So the Dockerfile passes `--target` set to the builder's
*own* host triple, read from `rustc -vV` at build time. That is never a
cross-compile, and it is not what ADR-0004 says.

Step 6 then decided to publish arm64 (step-6 decision (8)), built on native
`ubuntu-24.04-arm` runners rather than under QEMU. That turns the divergence
from a latent inconsistency into a published artifact: `.cargo/config.toml`
scoped `-C target-feature=+crt-static` to `x86_64-unknown-linux-musl` alone,
so an arm64 image would have been statically linked by rustc's default for
the musl target rather than by this repository's explicit instruction. Those
happen to produce the same result today. "Happens to" is not a decision.

## Decision

The shipping binaries are statically linked **musl binaries for the builder's
own host triple** — `x86_64-unknown-linux-musl` on an amd64 builder,
`aarch64-unknown-linux-musl` on an arm64 one — into `FROM scratch` images,
with mimalloc as the global allocator. Both triples carry an explicit
`-C target-feature=+crt-static` entry in `.cargo/config.toml`.

Publishing an architecture means adding its triple to `.cargo/config.toml` in
the same change. A triple that is built but not listed there is a defect, not
a shortcut.

## Consequences

`+crt-static` is stated once per published architecture instead of being
inherited from a default on one of them, so the two images are static for the
same stated reason. `.cargo/config.toml` gains a duplicated stanza; that
duplication is the point — it is greppable by the exact string `--target` is
invoked with.

The build is never a cross-compile, which is what keeps `ring`'s asm and
mimalloc's C build on a native `cc`. The price is a second runner pool and a
manifest-merge job in `.github/workflows/release.yml`, and the fact that an
amd64-only machine cannot reproduce the arm64 image locally without emulation.

**What has not been verified.** `aarch64-unknown-linux-musl` has never been
built. It is not an installed rustup target on any authoring machine, no
arm64 runner has run `release.yml`, and no arm64 image exists. This ADR
records the decision and makes the configuration match it; it is not a claim
that the arm64 build works. The first evidence will be a `release.yml` run.
