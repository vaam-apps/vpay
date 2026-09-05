# exp8 (opus): cargo-chef in `backends/Dockerfile`

2026-09-05. Branch `claude/exp8-cargo-chef-opus`, base `master` `a81b6b6`.
Everything below was run on the authoring host, `linux/amd64`, rootless
Docker (`DOCKER_HOST=unix:///run/user/1000/docker.sock`), buildx v0.36.1,
BuildKit v0.32.2.

**Measurement hygiene.** Every build in this document ran on a purpose-made
`docker-container` builder named `vpay-exp8-opus`, created for this task and
removed after it. The default builder was deliberately *not* used and never
pruned: another agent was building on this host at the same time, and
`docker builder prune -f` on the shared builder would have destroyed its
cache and contaminated both sets of numbers. Cold means
`docker buildx prune --builder vpay-exp8-opus -f` immediately beforehand (for
the very first run the builder container did not exist yet — the prune
printed `No such container` and the build was cold by construction).

**One sample per row.** Each number below is a single run on a 24-core host
that was doing other work. Treat the ratios, not the seconds.

## The defect, measured on `master` first

The builder was one stage. `ARG VPAY_GIT_SHA` / `ENV VPAY_GIT_SHA` were its
first instructions, then the workspace was copied and one `cargo build` ran.
`target/` is deliberately not a cache mount (a cache mount's contents are not
part of the layer, so the binary would vanish when the `RUN` exits), so an
invalidated layer means recompiling the whole dependency graph — not the one
crate that changed.

```
$ docker buildx build --builder vpay-exp8-opus -f backends/Dockerfile \
    --target server --tag vpay-exp8-opus:baseline-server --load --progress=plain .
#17 236.6     Finished `dist` profile [optimized] target(s) in 3m 56s
wall = 254 s

$ printf '\n// exp8 cache probe\n' >> backends/apps/vpay-server/src/main.rs
$ docker buildx build ...same...
#16 254.0     Finished `dist` profile [optimized] target(s) in 4m 13s
wall = 260 s
```

**One comment line cost a full rebuild.** That is the number this change
attacks.

## What was built

The canonical three-stage cargo-chef shape, adapted to this file's rules.

| Stage | Content |
|---|---|
| `chef` | `FROM rust:1.95.0-alpine3.22` (tag unchanged), `apk add --no-cache musl-dev pkgconfig`, `cargo install cargo-chef --locked --version 0.1.78` |
| `planner` | `FROM chef`, the workspace COPYs, `cargo chef prepare --recipe-path recipe.json` |
| `builder` | `FROM chef`, `COPY .cargo`, `COPY --from=planner recipe.json`, **cook**, then `ARG`/`ENV VPAY_GIT_SHA`, the workspace COPYs, the existing `cargo build` and `cp` to `/out` |
| `server`, `worker` | untouched — still `FROM scratch`, still `USER 65532:65532` |

### The pin

`cargo-chef 0.1.78`, published 2026-08-12, not yanked — the newest entry in
`https://index.crates.io/ca/rg/cargo-chef` when this was written. Pinned
exactly rather than floated, and with `--locked`, for the same reason the
`FROM` line pins a patch version: this stage compiles the tool that decides
what gets cached.

`cargo install` in the `chef` stage compiles it from source in **33 s** on
this host. That cost is paid once per change to the base image or the pin.

### Three decisions, each with a reason the Dockerfile now records

1. **The cook takes the same flags as the build.** `--profile dist`,
   `--target "$(rustc -vV | sed -n 's/^host: //p')"`,
   `-p vpay-server -p vpay-worker-bin`, and `.cargo/` is copied in *before*
   the cook so `+crt-static` applies. A cook under different rustflags or a
   different profile writes fingerprints the real build rejects: it would
   cache nothing and cost 68 s per build, silently.

   `--locked` is *not* passed to `cook`. The recipe already carries the
   resolved `Cargo.lock`, and cook rewrites the workspace's manifests into a
   skeleton, which `--locked` would reject.

2. **`ARG VPAY_GIT_SHA` sits after the cook.** Proven load-bearing by
   mutation below.

3. **`planner` copies exactly what `builder` copies** — `Cargo.toml`,
   `Cargo.lock`, `clippy.toml`, `.xtask`, `backends`, `sdks/rust`,
   `examples/merchant-demo` — rather than `COPY . .`.

   On the brief's question about `.xtask` and `sdks/rust`: both are needed,
   and so is `examples/merchant-demo`, for exactly the reason the Dockerfile
   already documents for the build stage. `[workspace] members` is a hard
   list; cargo refuses to load the manifest if a member directory is
   missing, and `cargo chef prepare` loads the manifest. Nothing new had to
   be allowed through `.dockerignore` — it excludes `/target`, `**/target`,
   `node_modules`, `.git`, `.github`, `docs/` and env files, none of which
   any workspace manifest lives in, and the planner sees the same context
   the builder already saw.

   `cargo chef prepare --bin` exists and would prune the workspace to the
   members one binary needs. It is deliberately unused: it would make the
   planner's view of the workspace differ from the builder's, and the recipe
   would stop describing the thing the next stage compiles.

## Proof

### Build A — cold

```
$ docker buildx prune --builder vpay-exp8-opus -f
$ docker buildx build --builder vpay-exp8-opus -f backends/Dockerfile \
    --target server --tag vpay-exp8-opus:chef-server --load --progress=plain .
#9  32.66     Finished `release` profile [optimized] target(s) in 32.58s   <- cargo install cargo-chef
#19 75.62     Finished `dist` profile [optimized] target(s) in 1m 15s      <- cargo chef cook
#25 118.5     Finished `dist` profile [optimized] target(s) in 1m 58s      <- cargo build
wall = 238 s
```

Cold is 238 s against the one-stage file's 254 s — i.e. the cold path does
not regress; the same work is split across two `RUN`s.

> **Retracted by the review pass, 2026-09-05.** That conclusion came from two
> unpaired samples taken at different times. Two matched pairs (same isolated
> builder, pruned between the two runs of each pair, runs back to back, the
> second pair in reverse order) give one-stage 193 s / cargo-chef 256 s and
> cargo-chef 248 s / one-stage 212 s. **The cold path regresses by 36-63 s** —
> the `cargo install` (32-58 s here) plus the cook's own pass. The trade is
> still worth making, but this line was wrong about its cost. See
> [opus-review.md](opus-review.md).

### Build B — one comment line added to `backends/apps/vpay-server/src/main.rs`

```
#8  [chef 3/3] RUN ... cargo install cargo-chef --locked --version 0.1.78
#8  CACHED
#16 [planner 7/7] RUN cargo chef prepare --recipe-path recipe.json
#16 DONE 0.1s
#17 [builder  3/10] COPY --from=planner /build/recipe.json recipe.json
#17 CACHED
#18 [builder  2/10] COPY .cargo ./.cargo
#18 CACHED
#19 [builder  4/10] RUN --mount=type=cache,target=/usr/local/cargo/registry ... cargo chef cook --profile dist --target "$target" -p vpay-server -p vpay-worker-bin --recipe-path recipe.json
#19 CACHED
#22 [builder  7/10] COPY backends ./backends
#22 DONE 0.0s
#25 [builder 10/10] RUN ... cargo build --profile dist --target "$target" -p vpay-server -p vpay-worker-bin; ... cp ...
#25 124.1     Finished `dist` profile [optimized] target(s) in 2m 03s
#25 DONE 124.2s
wall = 125 s
```

The cook is `CACHED`; the planner re-ran and produced a byte-identical
`recipe.json`, which is why the `COPY --from=planner` after it is `CACHED`
too. Only the final `cargo build` re-ran. The touch was reverted with
`git checkout --` immediately afterwards and `git status --porcelain`
confirmed a clean tree apart from `backends/Dockerfile`.

**260 s → 125 s.**

### Build C — only `--build-arg VPAY_GIT_SHA` changed

```
$ docker buildx build ... --build-arg VPAY_GIT_SHA=deadbeefcafedeadbeefcafedeadbeefcafe1234 ...
#19 [builder  4/10] RUN ... cargo chef cook ...
#19 CACHED
#20 [builder  5/10] COPY Cargo.toml Cargo.lock clippy.toml ./          CACHED
#22 [planner 2/7]  COPY Cargo.toml Cargo.lock clippy.toml ./           CACHED
#20 [builder  7/10] COPY backends ./backends                           CACHED
#25 [builder 10/10] RUN ... cargo build ...
#25   9.380    Compiling vpay-server v0.1.0 (/build/backends/apps/vpay-server)
#25 115.0     Finished `dist` profile [optimized] target(s) in 1m 54s
#25 DONE 115.1s
wall = 116 s
```

Every layer up to and including the cook is `CACHED`. Only `cargo build`
re-runs, which is exactly what `vpay-core/build.rs`'s
`cargo::rerun-if-env-changed=VPAY_GIT_SHA` asks for, and it is unchanged.

The label really does land in the binary, and really is absent otherwise:

```
$ strings <C image>/vpay-server | grep -c deadbeefcafedeadbeefcafedeadbeefcafe1234
1
$ strings <A image>/vpay-server | grep -c deadbeefcafedeadbeefcafedeadbeefcafe1234
0
```

### The mutation that proves decision 2 is load-bearing

`ARG`/`ENV VPAY_GIT_SHA` moved back to the top of the `builder` stage —
where it sat until today — and Build C re-run with a different sha:

```
#19 [builder  4/10] RUN ... cargo chef cook ...
#19  68.00     Finished `dist` profile [optimized] target(s) in 1m 07s
#19 DONE 68.6s
wall = 251 s
```

The cook is **not** `CACHED`; the dependency graph is recompiled; the build
costs 251 s instead of 116 s. The `ARG`'s position, not merely its presence,
is what makes the cache work.

This is also the mechanism by which `release.yml`'s `type=gha` cache scopes
were near-useless for `backends/Dockerfile` before today: every `master` push
passes a different `github.sha`, so every layer after the `ENV` missed on
every run, and `cache-from` could only restore the base image and the `apk
add`. That inference is recorded in `release.yml`'s comment. **It is an
inference from the file plus the local mutation above — nobody has read a
GitHub Actions cache-hit rate for this repository, before or after.**

### The runtime images did not change

```
$ docker images vpay-exp8-opus --format '{{.Tag}} {{.Size}}'
baseline-server 15.9MB
chef-server     15.9MB
chef-worker     12.7MB

$ docker image inspect vpay-exp8-opus:baseline-server --format '{{json .RootFS.Layers}}'
["sha256:39263117c91d...","sha256:571fbc262e37..."]
$ docker image inspect vpay-exp8-opus:chef-server     --format '{{json .RootFS.Layers}}'
["sha256:58ae12b54f48...","sha256:571fbc262e37..."]
```

Two layers before, two layers after; identical size; the `config/` layer is
the *same digest* in both. The binary layer differs because the two binaries
are different builds (the baseline image was the one built from the touched
source) — this is not a reproducible-build claim and nothing here tests one.

cargo-chef is not in the runtime image, checked directly rather than
inferred:

```
$ docker export <chef-server container> | tar -t | grep -i 'cargo\|chef\|rust'
(no output)
$ docker history --no-trunc vpay-exp8-opus:chef-server
ENTRYPOINT ["/vpay-server"]                     0B
EXPOSE [8080/tcp]                               0B
USER 65532:65532                                0B
ENV VPAY_CONFIG=/config/application.yml         0B
COPY config /config # buildkit                  28.7kB
COPY /out/vpay-server /vpay-server # buildkit   10.9MB
```

Both binaries run:

```
$ docker run --rm vpay-exp8-opus:chef-server --version
vpay-server 0.1.0
$ docker run --rm vpay-exp8-opus:chef-worker --version
vpay-worker-bin 0.1.0
```

`--target worker` built in **1 s** — it shares the whole `builder` stage.

### Summary table

| | one-stage (before) | cargo-chef (after) |
|---|---|---|
| cold build | 254 s | 238 s — **retracted, see above; matched pairs give 193/256 and 212/248** |
| source touched | 260 s | **125 s** |
| `VPAY_GIT_SHA` changed | (~260 s by construction: `ENV` was the first builder instruction) | **116 s** |
| `--target worker` after a `server` build | full rebuild | 1 s |
| `vpay-server` image | 15.9 MB, 2 layers | 15.9 MB, 2 layers |

### One result from `release-dry-run` that disagrees with the isolated runs

`release-dry-run` builds `--target server` then `--target worker` from the
same Dockerfile, back to back. On the dedicated builder that second build
took 1 s (everything cached). Inside `release-dry-run`, on the shared default
builder, the `chef` stage and the **cook were `CACHED`** — which is the
cross-image reuse this change is for, and it saved the 76 s cook — but the
`COPY backends ./backends` layers logged `DONE 0.0s` rather than `CACHED`,
so the final `cargo build` re-ran for 126 s.

```
#11 [chef 3/3] RUN ... cargo install cargo-chef --locked --version 0.1.78
#11 CACHED
#17 [builder  4/10] RUN ... cargo chef cook ...
#17 CACHED
#22 [builder  7/10] COPY backends ./backends
#22 DONE 0.0s
#25 [builder 10/10] RUN ... cargo build ...
#25 DONE 126.2s
```

**I did not work out why, and I am not going to guess in a status row.** The
obvious confound is uncontrolled: another agent was building the same
repository against the same rootless daemon and the same default builder
while this ran, and the default builder uses the `docker` driver rather than
`docker-container`. It is recorded here and in `docs/status.md` as an
observation. It does not affect Builds A/B/C, which ran on an isolated
builder with nothing else on it.

## What this does not buy, and why the number is only ~2x

`[profile.dist]` inherits `release`, which is `lto = "fat"` with
`codegen-units = 1`. A fat-LTO link re-consumes every dependency's LLVM IR at
link time, so the final `cargo build` costs ~2 minutes however much of the
graph is already compiled. cargo-chef removes the *frontend* compilation of
~317 packages (75 s of cook) and cannot remove the LTO link. A project on
thin or no LTO would see a much larger ratio. Nothing here trades away the
LTO — that is ADR-0004's performance decision and not this task's to reopen.

## Gates

| Gate | Result |
|---|---|
| `just verify` | ok — five gates passed, `verify-docs` report advisory |
| `just fmt-check` | `cargo fmt --all -- --check`, exit 0 (no Rust source was changed) |
| `just docs-check` | see the commit; `verify-status` + `verify-links` |
| `just release-dry-run` | **exit 0.** Four images for `linux/amd64` (`vpay-server` 15.9 MB, `vpay-worker` 12.7 MB, `vpay-dashboard` 344 MB, `vpay-checkout` 344 MB), then `helm-check`: 17 guards all fired by name, `/v1` `limit-rps=20` / token `limit-rps=5`, kubeconform `23 resources … Valid: 23, Invalid: 0, Errors: 0, Skipped: 0`. Ran on the *shared default* builder (the recipe drives `docker buildx build` directly), so it is not one of the controlled measurements above |
| `actionlint` (v at `/home/selast/go/bin/actionlint`) on `.github/workflows/release.yml` | exit 0, clean |

No Rust source file was changed by this task. The only source edit was the
comment line added and reverted twice as a cache probe.

## What I did not do

* **No arm64 build.** This host is amd64 and the release workflow builds
  arm64 on a native `ubuntu-24.04-arm` runner; reproducing it here means
  QEMU, which is the path step-6 decision (8) exists to avoid. The cook
  reads its target from `rustc -vV` exactly as the build does, so the
  host-triple invariant is preserved by construction — but preserved by
  construction is not measured.
* **No GitHub Actions run.** Nothing was pushed. The `type=gha` cache
  behaviour above is inference plus a local mutation, not a measured
  cache-hit rate.
* **No repeat runs.** One sample per row, on a busy host.
* **No change to `frontends/Dockerfile`.** It has its own layer story
  (pnpm) and was out of scope.
* **`cargo chef cook --check` / `--clippy`** were not used; CI's Rust job
  builds outside Docker and this file exists to produce release binaries.
