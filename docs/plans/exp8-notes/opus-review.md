# exp8 (opus): sabotage review of `claude/exp8-cargo-chef-opus`

2026-09-05. Reviewing `a81b6b6..59e5738` (2 commits) — the implementer's own
account is [opus.md](opus.md). Every claim below was re-measured; nothing was
taken on report.

**Conditions, and why they differ from the implementer's.** Same host, same
rootless daemon (`unix:///run/user/1000/docker.sock`), buildx v0.36.1,
BuildKit v0.32.2, `CARGO_BUILD_JOBS=4`, on a purpose-made `docker-container`
builder `vpay-exp8-opus-review` created for this pass and removed after it.
The shared default builder was never pruned. A second review agent was
building the same repository on the same daemon for the whole of this pass,
so absolute seconds here run 10-30 % above the implementer's. Every
comparison below is therefore between builds run **back to back under the
same load**, never against a number from an earlier session.

## Verdict

**Safe to merge, after the four corrections in this pass.** The mechanism is
real and every load-bearing property survived mutation: the cook is genuinely
reused, the `ARG` placement genuinely matters, the recipe genuinely tracks the
manifests, and the runtime images are byte-for-byte the shape they were. What
did not survive was one *number* — the claim that the cold path does not
regress — and three docs claims that were stale or self-contradictory. No
build-break, no cache-miss, no fake green.

## Findings

| # | Severity | Finding | Status |
|---|---|---|---|
| R1 | misleading-claim | "Cold is 238 s against the one-stage file's 254 s — the cold path does not regress." Not reproducible. Two matched cold pairs say cargo-chef costs **+36 to +63 s** cold. | fixed (Dockerfile header, release.md §7, opus.md retraction) |
| R2 | misleading-claim | `docs/status.md`'s image-publishing row became self-contradictory: the implementer corrected "six `mode=max` scopes" to eight in `release.yml`, but the same row still said "Six `build` jobs — three images × two architectures", still titled itself `vpay-{server,worker,dashboard}`, and still said "neither declares an `ARG`, so there is **no `--build-arg`** in the release path" — three lines above a new sentence explaining that `release.yml` passes `VPAY_GIT_SHA=${{ github.sha }}` on every push. The `ARG` claim was already false on `master`. | fixed |
| R3 | nit | `just release-dry-run`'s closing echo says "three images built" while its loop builds four. The implementer surfaced it in `docs/status.md` and left it; it is the gate this task is verified by, so it now prints the truth. `docs/runbooks/release.md` had the same stale count in three more places. | fixed |
| R4 | nit | "`docker export` of the final image lists exactly `config/` and `/vpay-server`". It also lists `.dockerenv`, `/dev`, `/etc`, `/proc`, `/sys` — runtime stubs, not layers. The substantive half (no `cargo`, `chef` or `rust` anywhere in it) is exactly right. | fixed |
| R5 | nit | "The planner copies exactly what the builder copies" — the builder also copies `.cargo`, which the planner does not. Measured consequence: the recipe's `config_file` is `null`, so `.cargo/config.toml` reaches the cook only through the builder's own `COPY`. That is what makes the design work, and it was worth a clause rather than a contradiction. | fixed |
| R6 | robustness (docs) | "A cook under different rustflags or a different profile ... caches nothing — pure added cost, and a *silent* one." Half right, and the half that is wrong is the reassuring half: a `--target` mismatch is **loud** (the cook dies in <1 s), a `--profile` mismatch is **silent** and costs 48 s + a full rebuild. Both now measured in the header. | fixed |
| R7 | informational | `apk add --no-cache musl-dev pkgconfig` in the `chef` stage: `rust:1.95.0-alpine3.22` already ships `musl-dev` and `gcc`. Removing `musl-dev` still builds (mutation M5, exit 0). Pre-existing, defensive, and **deliberately left alone** — dropping it would couple this build to the base image's package set. | recorded, not changed |
| R8 | maintainer decision | Nothing records why `[profile.release]` sets `lto = "fat"`: no ADR mentions LTO and `Cargo.toml` has no comment on that line. The implementer attributed it to ADR-0004, which is about musl and mimalloc and says nothing about it. Since fat LTO, not cargo-chef, is what now bounds a release rebuild, "should `[profile.dist]` override it with `thin`?" is a real open question — **surfaced, not answered**. | attribution corrected; decision left open |
| R9 | not a defect | The `COPY backends` cache miss the implementer recorded as unexplained in `release-dry-run`. `DONE 0.0s` is what BuildKit prints for a `COPY` that missed and then took no measurable time; `CACHED` requires a hit, and once one `COPY` misses every later `COPY` in the stage misses with it. Build B here reproduces the identical output at the moment `backends/` was known to have changed, and `just release-dry-run` on the committed tree logs `COPY backends ./backends CACHED` and exits in 5 s. | explained in `opus.md` |

Checked and **not** found wanting: the pin (`cargo-chef 0.1.78` is the newest
entry in `https://index.crates.io/ca/rg/cargo-chef` and is not yanked, so
"published 2026-08-12, nothing newer" holds), `--locked` on the install, the
base-image tag (untouched), the planner copying no `.git`/`.github`/secrets
(`.dockerignore` excludes all three and the planner names its directories
anyway), `.dockerignore` itself (unchanged, and `docs/` being in it is why a
docs edit is a 1 s build), `actionlint` on `release.yml` (exit 0, v1.7.12),
and the ADR-0014 host-triple rule (the cook reads `rustc -vV` exactly as the
build does).

## Builds (mine)

`--target server` throughout, `--load`, `--progress=plain`, on the isolated
builder. "cold" = `docker buildx prune --builder vpay-exp8-opus-review -af`
immediately before.

| Build | What changed | Wall | `cargo chef cook` |
|---|---|---|---|
| A | cold, first run of the pass (registry cache mount cold too) | 306 s | ran, 80 s |
| B | one comment line in `backends/apps/vpay-server/src/main.rs` | **105 s** | `CACHED` |
| C | `--build-arg VPAY_GIT_SHA=cafebabe…` only | **101 s** | `CACHED` |
| D | one line appended to `docs/status.md` | **1 s** | `CACHED` (21 layers cached, nothing ran) |

Build B's log, trimmed to the load-bearing lines:

```
#8  [chef 3/3] RUN ... cargo install cargo-chef --locked --version 0.1.78   CACHED
#16 [planner 7/7] RUN cargo chef prepare --recipe-path recipe.json          DONE 0.1s
#17 [builder  3/10] COPY --from=planner /build/recipe.json recipe.json      CACHED
#20 [builder  4/10] RUN ... cargo chef cook --profile dist --target ...     CACHED
#22 [builder  7/10] COPY backends ./backends                                DONE 0.0s
#25 [builder 10/10] RUN ... cargo build --profile dist ...                  DONE 103.0s
```

The planner re-ran and produced a byte-identical `recipe.json` — which is
what makes the `COPY --from=planner` after it `CACHED`, and the cook with it.

### The cold pairs (R1)

Each pair pruned the same builder between its two runs; the second pair
reverses the order so host load cannot explain the sign.

| Pair | one-stage (`a81b6b6`'s Dockerfile) | cargo-chef (this branch) | delta |
|---|---|---|---|
| 1 — one-stage first | 193 s (single `cargo build` 182 s) | 256 s (install 58 s + cook 70 s + build 114 s) | **+63 s** |
| 2 — cargo-chef first | 212 s (single `cargo build` 200 s) | 248 s (install 32 s + cook 60 s + build 143 s) | **+36 s** |

The regression is `cargo install cargo-chef` plus the cook's own front-end
pass, and it is the price of the warm case. It is not a reason not to do
this; it is a reason not to claim the opposite.

## Mutations

Each applied to a copy of the Dockerfile in a scratch directory (the worktree
was never left dirty), rebuilt, recorded, discarded.

| # | Mutation | Result | Reads as |
|---|---|---|---|
| M1 | `ARG`/`ENV VPAY_GIT_SHA` moved above the cook; Build C re-run | **215 s**, cook **not** `CACHED` (ran 82 s) vs 101 s | the placement is load-bearing, and costs 114 s per commit if lost |
| M2 | `anyhow = "1"` → `"1.0"` in the root `Cargo.toml` | cook **not** `CACHED`, ran 55 s | the recipe really is the manifest graph, not a proxy for it |
| M3 | `--profile dist` **and** `--target` dropped from the cook | **build fails in 2 s**: `cannot produce proc-macro for async-trait v0.1.92 as the target x86_64-unknown-linux-musl does not support these crate types` | a `--target` mismatch cannot ship silently |
| M3b | only `--profile dist` dropped | exit 0, **305 s**: cook runs in the `dev` profile (48 s) and the final `dist` build recompiles everything (255 s) | this is the silent one, and it is the one the header now names |
| M4 | `.xtask` and `sdks/rust` removed from the **planner**'s COPYs | `cargo chef prepare` fails in 1 s: `failed to read /build/sdks/rust/Cargo.toml` | prepare does not emit a truncated recipe; it refuses |
| M5 | `musl-dev` removed from the `chef` stage's `apk add` | exit 0, 254 s, image identical | the base image already ships `musl-dev` and `gcc` (R7) |

`recipe.json`, extracted from the planner stage: 18 manifests — the 17
`[workspace] members` plus the root — `lock_file` present, `config_file` and
`rust_toolchain_file` both `null` (R5).

## Runtime image, verified rather than inferred

| Check | Result |
|---|---|
| size, `vpay-server` | 15.9 MB from the pre-change Dockerfile and 15.9 MB from this one |
| layers | 2 and 2; the `config/` layer is the same digest in both, `sha256:571fbc262e375dd7c77adf3d212534a9c2db16ccb93822b965eb6ba2b19c091b` |
| `docker history` | `ENTRYPOINT` / `EXPOSE` / `USER` / `ENV` at 0 B, `COPY config /config` 28.7 kB, `COPY /out/vpay-server` 10.9 MB. Nothing else |
| `docker export \| tar -t \| grep -i 'cargo\|chef\|rust'` | no match |
| `docker run --rm <image> --version` | `vpay-server 0.1.0`, exit 0 — from the cargo-chef image **and** from an image built off `a81b6b6`'s Dockerfile |
| `VPAY_GIT_SHA` still reaches the binary | `strings` finds the Build C sha exactly once in the Build C binary and zero times in the Build A one |
| still statically linked | `file` says `ELF 64-bit LSB pie executable, x86-64, static-pie linked, stripped` — i.e. `+crt-static` really did survive the cook |

## Gates, re-run on the final tree

| Gate | Result |
|---|---|
| `just verify` | ok — five gates, `verify-docs` advisory |
| `just docs-check` | `verify-status` ok (1 unimplemented item, declared), `verify-links` ok (680 links, 116 files) |
| `just fmt-check` | exit 0 (no Rust source changed by either pass) |
| `just release-dry-run` | exit 0. On the delivered tree it completed in **5 s** — every layer of both Rust images `CACHED` on the shared default builder, `helm-check` 17 guards, kubeconform `Valid: 23, Invalid: 0` |
| `actionlint` v1.7.12 on `release.yml` | exit 0 |
| Builds A/B/C/D | above |

## What this review did NOT do

* **No arm64 anything.** Read-only reasoning only: the `chef` stage's
  `cargo install` runs with cwd `/` and no repo `.cargo/config.toml` in
  scope (`$CARGO_HOME/config.toml` does not exist in the base image), so it
  is not exposed to the `+crt-static` proc-macro trap on either
  architecture — which also means **moving `COPY .cargo` up into `chef`
  would break the install**, on amd64 as well as arm64. Nothing else in the
  new stages is architecture-specific. None of that is measured.
* **No GitHub Actions run**, so every `type=gha` statement in `release.yml`
  and `docs/status.md` remains an inference from the file plus a local
  mutation. Nothing was pushed.
* **No repeat sampling beyond the cold pairs.** One run per warm row, on a
  host running another agent's builds throughout.
* **No `frontends/Dockerfile` work**, and no attempt to reproduce the
  implementer's exact seconds — this pass measured its own.
* **No decision on `lto = "thin"`** (R8), and no change to `[profile.dist]`.
