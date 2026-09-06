# CLAUDE.md

Claude Code and other coding agents: **read [AGENTS.md](AGENTS.md) first.** It is
the source of truth for how to work in this repository. This file adds only what
is specific to working here as an agent.

## Before you start

```bash
just verify    # ten self-checks, all of which must pass before AND after your
               # change, plus the `verify-docs` report, which never fails
cat docs/status.md
```

*(This said "three" until 2026-09-06 and had been wrong since 2026-09-03.
[AGENTS.md](AGENTS.md) carries the count and the history of every gate that
moved it; that is the copy to trust, and this one now agrees with it.)*

`docs/status.md` tells you what is actually built. Do not infer capability from
the presence of a file — most of this repo is scaffold, and it says so.

## The failure mode to avoid

The most likely way to damage this project is to make it *look* more finished
than it is:

- filling an unimplemented function with something plausible that returns a
  hard-coded success,
- writing a test that asserts nothing so a suite goes green,
- adding a mock adapter to make local development easier,
- rendering fake rows in the dashboard so a screenshot looks good,
- marking something ✅ in `docs/status.md` because it compiles.

Each of these is worse than leaving the gap visible. This is a payment system;
someone will eventually trust it with real money on the strength of what the
repo claims about itself.

If you cannot implement something properly, leave
`ProviderError::NotImplemented`, list it in `docs/status.md`, and say so plainly
in your summary.

## When you finish a task

1. `just ci` — which since Step 7 also runs `just test-doc`
   (`cargo test --doc --workspace`). `cargo nextest` runs no doctests, so an
   example in a doc comment is only checked by that step.
2. Update `docs/status.md` — in the same commit, not a follow-up.
3. Update the relevant `docs/flows/*.md` **Status** section.
4. In your summary to the user, state explicitly what you did **not** do.

## Verifying rather than assuming

This repo prefers evidence over confidence:

- Ran the tests? Say how many passed *and how many are ignored*. Doctests are
  a separate runner and a separate count (`just test-doc`); "the tests pass"
  without one is half an answer.
- Changed the schema? Apply it to a real Postgres and prove the constraint fires.
- Changed a diagram? Render it and look at it — "renders without error" is not
  "renders correctly".
- Claimed an adapter works? Point at the conformance case that proves it.

## Things that will waste your time

- `rust-toolchain.toml` pins `1.98.0` — it was `1.95.0` until 2026-09-05 — and
  that is the same version `backends/Dockerfile` builds with; CI reads the pin
  from the file. The musl target needs
  `rustup target add x86_64-unknown-linux-musl`.
- Cypress needs `pnpm exec cypress install` on a network that can reach its CDN;
  `CYPRESS_INSTALL_BINARY=0` skips it.
- `clippy.toml` exempts tests from the `unwrap`/`expect`/`panic` deny. If clippy
  complains about `expect` in a test, you are outside a `#[cfg(test)]` module.
- `schemas/vpay.cstack` **is** wired into the build (2026-09-06). This bullet
  said the opposite — "not wired into the build, its syntax is unverified, do
  not try to make it compile" — and had been wrong in two stages: `just
  check-schema` began verifying the syntax on 2026-09-05, and `vpay-db`'s
  private `mod schema` began *compiling* the file on 2026-09-06. A syntax
  error in it is now a `cargo build` failure. Two things follow. The CLI and
  the library must stay on one version — `justfile`'s `cratestack_version`
  and `Cargo.toml`'s `cratestack = "=0.11.1"` — so bump them together. And
  the generated module is private to `vpay-db` on purpose: `cargo xtask
  verify-repositories` fails if `mod schema` is made `pub` or re-exported,
  because the module the macro creates exists in no source file and nothing
  else would object. Adding a `model` is not free either: it must match the
  live table, and `postgres_smoke.rs`'s drift test pins the exact gap. See
  `docs/reference/vpay-db.md` § CrateStack.
