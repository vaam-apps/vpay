# Cargo cache mounts are locked

_Written 2026-11-28 on a branch from `origin/master` (`7a9fee5`). The status row is in
[infrastructure.md](../infrastructure.md); this page is what was run, and what was not._

## What failed

CI run 46086046163 (job 140795888213), the `e2e (compose)` job, step "build both images and bring up the real stack":
the `vpay-shop` sealer stage ran `cargo install wasm-bindgen-cli` and failed with
`failed to unpack package hashbrown v0.16.1 … .cargo-ok … File exists (os error 17)`. At the same time the
`vpay-worker` chef stage was running `cargo install cargo-chef`. Both write the same
`/usr/local/cargo/registry` cache mount, and BuildKit's default is `sharing=shared`; cargo's unpack is not safe for
concurrent writers. The failure is intermittent because it needs the two stages to overlap.

## What changed

`,sharing=locked` on every cargo registry and cargo git cache mount, 10 mounts in all, and nothing else:

- `backends/Dockerfile`: the `chef`, `planner` and `builder` stages (lines 202-203, 237-238, 312-313).
- `examples/shop/Dockerfile`: the sealer's two `cargo install` and wasm build steps (lines 36-37, 51-52).

Left alone on purpose: `/build/target` in `examples/shop/Dockerfile` (one stage uses it, no second writer in the same
build). No pnpm or npm cache mount exists in any Dockerfile (`git ls-files '*Dockerfile*'`). No xtask gate or doc quotes
these lines.

Cost: two stages that use the same cache now wait for each other instead of overlapping. Layers that are already cached
are not affected.

## What was run

- `docker buildx build --check` on `backends/Dockerfile` and `examples/shop/Dockerfile`: no warnings.
- `cargo xtask` gates, run one by one because `just` is not installed here: all 16 passed (`verify-no-mocks` to
  `verify-adr-numbers`). `verify-doc-counts` first failed on the verification directory's file count (150 to 151, the
  page this change adds) and passed after `docs/status/README.md` was corrected. `just check-schema`, `verify-ui` and
  `verify-docs` (advisory) were not run.
- `prettier --check` (3.9.6) on the three markdown files: clean.

## What was not run

- No image was built, and the `e2e (compose)` job has **not** yet run on this change. That the race is gone is
  **unproven** until that job has passed on several runs, since the failure was intermittent.
