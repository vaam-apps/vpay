# 2026-10-26 — the procedure-backed lists on `@cratestack/refine`, a generated client, Node 24

**What this page is evidence for:** the dashboard's four procedure-backed
lists (refunds, deliveries, customers, checkouts) and its Overview read
through `@vpay/dash-client`, a TypeScript client generated from
`schemas/vpay.cstack`, with `@cratestack/refine` 0.15.4 as their Refine data
provider; `just check-schema` drift-checks the client; `.nvmrc` moved to Node
24.21.0. **What it is not evidence for:** any browser, any real vpay, any
built image — see "Not run" at the bottom.

Base: `master` at `172a13a`. Toolchain: Node `24.21.0` (`.nvmrc`), pnpm
`11.18.0`, `cratestack` CLI `0.15.0`, rustc `1.98.0`.

## Decisions taken, with the maintainer

- Swap the hand-written provider for `@cratestack/refine` over a generated
  client; keep the BFF, the auth stack and the backend unchanged.
- Read-only: ADR-0008 stands, no create/edit/delete.
- Payments keeps its cursor list on the hand-written provider (RD2 not taken).
- Node 22 → 24, because every `@cratestack/refine` release declares
  `engines.node >= 24` and `engine-strict=true` refuses the install on 22.

## Re-measured upstream, not assumed

`cratestack-core` 0.15.0 and 0.16.0, `src/value.rs:95-102`, read from the
published crates: `Value::from_plain_json` still maps a non-`i64` JSON number
to `Value::Float(number.as_f64().unwrap_or_default())`. Generated model routes
stay off the money tables; nothing here mounts one.

## The generated client and its gate

`just gen-dash-client` → `frontends/packages/dash-client` (`src/models.ts`
2 109 lines, `src/client.ts` 1 475, `src/runtime.ts` 222, `src/queries.ts` 120;
`package.json` from the one-file template override
`schemas/typescript-client/package.json.j2`, so the package is consumed as
source with no `typescript ^7` of its own).

`just check-schema`, each arm mutated and reverted:

| Mutation                                                                  | Result                                                                |
| ------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| none                                                                      | `check-schema: ok — frontends/packages/dash-client matches …`, exit 0 |
| `// drift` appended to `src/models.ts`                                    | `modified: src/models.ts`, `FAIL`, exit 1                             |
| empty `src/stale.ts` added                                                | `unexpected: src/stale.ts`, `FAIL`, exit 1                            |
| `probe Int?` added to `type RefundSummary` in the schema, not regenerated | `modified: src/models.ts`, `modified: src/runtime.ts`, `FAIL`, exit 1 |

Found while wiring it: cratestack 0.15.0's `--check` reported six
`node_modules/decimal.js/*` files as drift on a clean tree. Its walker follows
pnpm's symlink and `git check-ignore` answers exit 128 ("beyond a symbolic
link"), which `drift.rs`'s `git_ignored` reads as "nothing ignored". The
recipe checks a `tar --exclude=./node_modules` copy instead. Upstream bug,
not filed from here.

## Dashboard suite

`pnpm --filter @vpay/dashboard test`, Node 24.21.0:

|                                                       | files | passed | skipped |
| ----------------------------------------------------- | ----- | ------ | ------- |
| `master` (`172a13a`), Node 24.21.0, before any change | 35    | 456    | 1       |
| this change                                           | 40    | 482    | 1       |

Five new files, 26 new cases: `src/dash/procedure-resources.test.ts` (10),
`src/dash/cratestack-client.test.ts` (7), `src/dash/procedure-provider.test.ts`
(3), `src/server/bff-procedures.test.ts` (3),
`src/components/procedure-list-view.test.tsx` (3). The 456 existing cases ran
unchanged — `src/dash/overview.test.ts` stubs `fetch` at the network boundary
and asserts the procedure, the filter and the limit of every call, so it is
evidence that the generated client sends what `callDashProcedure` sent.

Mutations, each run and reverted:

| Mutation                                                                  | Went red                                                                                                 |
| ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `filterFor` passes an unknown key through instead of refusing it          | `refuses a filter the procedure does not declare`, `refuses getMany, whose id__in no procedure declares` |
| `withOneRemint` retries any status `>= 400`, not only `401`               | `never retries a 403` (`cratestack-client.test.ts`) **and** its twin in `dash-read.test.ts`              |
| `procedureListResponse` spreads the procedure's page into the served body | `puts the bearer token in no header and no body`                                                         |

`pnpm --filter @vpay/dashboard typecheck` and `lint`: clean. A runtime import
of `@vpay/dash-client` outside `src/dash/cratestack-client.ts` is refused:
`'@vpay/dash-client' import is restricted from being used` (probe file,
deleted).

## Build

`pnpm --filter @vpay/dashboard build`, First Load JS:

| Route                                   | `master` | this change |
| --------------------------------------- | -------- | ----------- |
| `/refunds`, `/deliveries`, `/checkouts` | 264 kB   | 336 kB      |
| `/customers`                            | 264 kB   | 334 kB      |
| `/overview`                             | 264 kB   | 264 kB      |
| `/payments`                             | 337 kB   | 337 kB      |
| `/payments/[id]`                        | 337 kB   | 338 kB      |
| shared by all                           | 103 kB   | 103 kB      |

The four lists now hydrate Refine's `useList`, which `/payments` already
shipped. The build needed `webpack.resolve.extensionAlias` for `.js` → `.ts`,
because the generated sources' relative imports carry `.js` and their bytes
are held by `--check`.

## `just verify` and `just ci`

`just verify`: `verify: ok — the fifteen gates above passed`.

`just ci` (Node 24.21.0), recipe by recipe — **not one green run, and this
says why**:

- `fmt-check` (cargo fmt, prettier), `clippy -D warnings`, and all fifteen
  `verify` gates: **passed**.
- `lint-web` and `test-web`: **passed** — every workspace package; the
  dashboard 40 files / 482 passed / 1 skipped, `@vpay/config` 67/67 (its
  "every package is linted" gate exempts the generated `dash-client` by name,
  and a new case fails if that package stops being generator output).
- `audit-web`: **passed** ("no moderate, high or critical advisory").
- `test-rust`, `test-doc`, `verify-ignored`: **not run to completion.** The
  first attempts found no `cargo-nextest`; once it was installed the
  workspace test build exhausted this session's disk allowance (`ld
terminated with signal 7 [Bus error]`, then 0 MB free with a 25 GB
  `target/`). No Rust file changed in this work; CI's Rust jobs are the
  evidence for these.
- `deny`: not run (`cargo-deny` not installed); no Cargo dependency changed.

## Images

Built from `frontends/Dockerfile` on `node:24-alpine` (resolved to Node
v24.22.1 — the Dockerfile pins the major only, as it pinned 22), with a
scratch copy of the Dockerfile that adds only this session's proxy CA:
`runner` (dashboard) and `checkout` both build. The dashboard image run as
`docker run --read-only --tmpfs /tmp --user 1000:1000` answered `/healthz`
`200`, `id -u` → `1000`, and Docker's own `HEALTHCHECK` reported `healthy`.
The checkout image was built and not run. `examples/shop`'s image was not
built.

## Not run

- **Cypress / any browser.** `dashboard.cy.ts` was not run against these
  screens.
- **Any real vpay.** Every read in the suites is a stubbed `fetch`.
- **`examples/shop`'s Node 24 image**, and the checkout image's runtime
  behaviour (it was built, not run).
