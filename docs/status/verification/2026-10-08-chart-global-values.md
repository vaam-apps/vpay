# 2026-10-08 — The chart accepts Helm's `global` key ([#269](https://github.com/vaam-apps/vpay/issues/269))

Branch `claude/chart-global-values-269`, from `master` at `a33aac61`. A schema
change, a new step in `just helm-check`, a test fixture and docs. **No
template, no `values.yaml` default, no chart version, no migration, no route
and no `NotImplemented` token moved.**

## What was wrong

`deploy/helm/vpay/values.schema.json` is `additionalProperties: false` and its
top-level `properties` did not list `global`. Helm passes a parent chart's
`global` values to every subchart and validates them against that subchart's
schema, so a chart that listed this one under `dependencies:` failed `helm lint`
and `helm template`. Every check in `just helm-check` rendered the chart as the
**root**, which is why none of them noticed.

Helm adds the key to a subchart's values whether or not the parent sets it: the
same failure was measured with the parent's `values.yaml` empty (Helm 3.16.1
and 4.0.0).

## The fixture

`deploy/helm/fixtures/wrapper/` is a parent chart (`Chart.yaml`, `values.yaml`,
`.gitignore`). Its one dependency is `deploy/helm/vpay` by
`repository: file://../../vpay`, version `>=0.0.0-0` so that release-please
moving the chart's version cannot break it. Its `values.yaml` sets
`global.imageRegistry`. It sits beside `deploy/helm/vpay`, not inside it, so
`helm package deploy/helm/vpay` in `release.yml` and `just release-dry-run`
never ships it. CI's `deploy` path filter is `deploy/helm/**`, so a change to it
runs the job.

No `Chart.lock` is committed: `helm dependency build` without one resolves like
`helm dependency update`, and a committed lock carries a digest of the chart
that goes stale at the next release.

## Reproduction, on unmodified `a33aac61`

The fixture alone, no recipe, from `deploy/helm/fixtures/wrapper`
(`helm dependency build .` first):

| Helm   | Command         | Output                                                                                                                                                  |
| ------ | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 3.16.1 | `helm lint .`   | `[ERROR] templates/: values don't meet the specifications of the schema(s)` … `- (root): Additional property global is not allowed`, exit 1             |
| 3.16.1 | `helm template` | `Error: values don't meet the specifications of the schema(s) in the following chart(s):` `vpay:` `- (root): Additional property global is not allowed` |
| 3.18.6 | `helm template` | `vpay:` `- at '': additional properties 'global' not allowed`                                                                                           |
| 3.19.0 | `helm template` | `vpay:` `- at '': additional properties 'global' not allowed`                                                                                           |
| 4.0.0  | `helm template` | `vpay:` `- at '': additional properties 'global' not allowed`                                                                                           |

3.16.1 is the version CI pins (`azure/setup-helm`, `version: v3.16.1`); the
text from 3.18 on is the reporter's, character for character.

With `values.yaml` emptied, 3.16.1 and 4.0.0 fail identically.

## The change

```json
"global": {
  "type": "object",
  "description": "Helm's reserved key. …"
},
```

at the top of `values.schema.json`'s `properties`. Nothing else in the schema
changed. `grep -rn "Values.global\|\.global" deploy/helm/vpay/templates
deploy/helm/vpay/values.yaml deploy/helm/vpay/README.md deploy/helm/vpay/Chart.yaml`
finds no template or default that reads it, so it is accepted and ignored.

## After, the fixture alone

| Helm                  | `helm lint .`                          | `helm template vpay .` |
| --------------------- | -------------------------------------- | ---------------------- |
| 3.16.1                | `1 chart(s) linted, 0 chart(s) failed` | 6 objects (`^kind:`)   |
| 3.18.6, 3.19.0, 4.0.0 | `1 chart(s) linted, 0 chart(s) failed` | 6 objects              |

`diff <(helm template vpay deploy/helm/vpay | grep -v '^# Source:') <(helm
template vpay deploy/helm/fixtures/wrapper | grep -v '^# Source:')` prints
nothing: the wrapper renders exactly the chart's default objects.

## `just helm-check`

The recipe gained one step, "wrapper chart", between the management-tier check
and `kubeconform`. It runs `helm dependency build`, `helm lint` and `helm
template` on the fixture, then asserts three things: a `Deployment` was
rendered (the dependency contributed something), the render equals the root
default render apart from the `# Source:` comments, and
`helm template vpay "$wrapper" --set vpay.bogus=1` fails with a message naming
`bogus`. The recipe's `trap` removes the fixture's `charts/` and `Chart.lock`.

Tools: Helm v3.16.1 from `get.helm.sh`; kubeconform v0.6.7, its tarball checked
with `sha256sum -c` against `95f14e87…` (the digest `ci.yml` pins); `just`
1.43.0. All run from a scratch directory on `PATH`. Network: kubeconform's
schema fetches went through the session proxy and succeeded.

**Before** (schema reverted, everything else in place), exit 1:

```
==> wrapper chart (vpay as a subchart of a parent that sets global)
==> Linting deploy/helm/fixtures/wrapper
[INFO] Chart.yaml: icon is recommended
[ERROR] templates/: values don't meet the specifications of the schema(s) in the following chart(s):
vpay:
- (root): Additional property global is not allowed


Error: 1 chart(s) linted, 1 chart(s) failed
error: Recipe `helm-check` failed with exit code 1
```

**After**, exit 0:

```
==> wrapper chart (vpay as a subchart of a parent that sets global)
==> Linting deploy/helm/fixtures/wrapper
[INFO] Chart.yaml: icon is recommended

1 chart(s) linted, 0 chart(s) failed
    global accepted, same objects as the root render, unknown keys still refused
==> kubeconform (downloads schemas — needs network)
Summary: 68 resources found in 4 files - Valid: 68, Invalid: 0, Errors: 0, Skipped: 0
helm-check: ok — lint, 4 renders, 24 guards, rate limit (both paths), rail callback (both paths), management route/policy coherence, wrapper chart (global), kubeconform. No cluster was involved.
```

Unchanged by this step, as it should be: `25 fixtures, 24 guards, all fired
by name (24 expected)`, and 68 resources across the same four renders. The
wrapper's render is not passed to `kubeconform`; it is the same objects as the
default render, which is.

## Negative controls

Each was applied to the tree, `just helm-check` run, and the change reverted.

| Mutation                                                                                      | Result                                                                                                                         |
| --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| The schema change reverted                                                                    | Fails at `helm lint` of the wrapper, as above.                                                                                 |
| Top-level `additionalProperties` set to `true` (the over-broad fix)                           | `helm-check: FAIL — an unknown key under vpay: was accepted; the schema is no longer closed`, exit 1                           |
| A template reading `.Values.global` (three lines appended to `templates/serviceaccount.yaml`) | `helm-check: FAIL — vpay rendered under a parent chart is not what it renders as the root; `global` changed something`, exit 1 |

The third is why the equality assertion exists: "accepted" and "ignored" are
two claims, and the schema can only prove the first.

## The gates

`just verify`, run from the worktree with the files staged (`verify-links`
reads tracked files, so it calls a new page broken until `git add`):
**exit 0, `verify: ok — the fifteen gates above passed; the verify-docs report
is advisory`**. Per gate:

```
verify-no-mocks: ok — no test double reachable from a shipping binary
verify-status: ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code
verify-errors: ok — 20 error type(s), all classified; 17 `#[from]` variant(s) delegate every `Classify` method they match on; anyhow confined to binaries
verify-sdk-parity: ok — 767 proving test(s) named in docs/sdks/parity.md all exist, 45 dated gap(s), 35 SDK method(s) enumerated across 40 row(s)
verify-links: ok — 2119 repository link(s) in 435 tracked markdown file(s) resolve to a tracked path (anchors and http(s) URLs are not checked)
check-schema: ok — schemas/vpay.cstack type-checks under cratestack 0.15.0
verify-serde: ok — 104 serialisable type(s) spell the workspace's wire convention, 17 exempted with a reason in docs/adr/0016-engineering-standards.md
verify-toolchain: ok — rust-toolchain.toml pins 1.98.0 and all 1 `FROM rust:` instruction(s) in backends/Dockerfile name it (rust:1.98.0-alpine3.22)
verify-migrations: ok — 49 migration file(s) in backends/migrations/ all match their entries in backends/migrations/MANIFEST.sha256
verify-versions: ok — 24 version references all say 0.5.0
verify-doc-counts: ok — 13 documented count(s) in 11 of 277 markdown file(s) agree with what this tree measures
```

(`verify-npm-scope`, `verify-repositories`, `verify-ui` and
`verify-privacy-inventory` also passed; their lines are omitted for width.)

Two gates failed on the way, both on this change's own docs, both by name:
`verify-links` called the verification page missing until it was staged, and
`verify-doc-counts` reported `docs/status/README.md:44: this line says 70, but
count:files-with-suffix docs/status/verification .md measures 71`. The count,
the "names 47 of them" beside it and the 23-it-omits arithmetic were
corrected in the same change.

`cratestack` 0.15.0 was not on `PATH` at first, so `check-schema` failed (as a
failure, not a skip); it was built with `cargo install cratestack-cli --locked
--version 0.15.0` into a scratch directory and the gate then passed. Nothing in
this change touches `schemas/`.

`prettier --check` (3.9.6, the repository's pin, from a scratch install) over
`docs`, `deploy` and `.github`: all matched files use Prettier code style.

`cargo xtask verify-doc-counts` agrees with the tree: the recipe's count of
gates is untouched, since `helm-check` is not one of `just verify`'s gates.

## Docs and skills

Updated in this change: the chart README (a subsection and a step in
"Verifying the chart"), `docs/flows/deployment.md` (§ 7 and the Status
bullets), the Helm rows of `docs/status/infrastructure.md`, `.github/workflows/ci.yml`'s
comment above `just helm-check`, and the index in `docs/status/README.md`.

Docs↔skills parity (`CLAUDE.md`, item 4 of "When you finish a task"):
`vaam-apps/vpay-skills` at `1fc8dfc`, `node tools/verify-coverage.mjs
<this worktree>` reported `parity` (20 skills, 216 vpay paths claimed, 46
feature pages, 0 exempt). The gate passes because no path a skill cites moved
and no `docs/flows/` page was added. It does not see prose. Two skill passages
describe what `helm-check` proves and are stale: `vpay-tooling/references/recipes.md`
§ Helm says it lints "three value sets" (the recipe has had four since
2026-09-16), and `vpay-ops/references/deployment.md` § Verifying the chart says
the same. Neither mentions the wrapper step. This change is a new assertion in
an existing recipe, not a new gate, so it does not strictly trigger the rule;
the follow-up PR to `vpay-skills` is recommended and is **not** opened here.

## What this does not establish

- That the published OCI chart works as a dependency from a registry. The
  fixture uses `file://`, which reads the same `values.schema.json`; no
  `oci://` dependency was resolved.
- That the new step passes on a GitHub runner. It has run only here.
- Anything about a cluster. The chart's status stays 🟡.
- That any template should read `global`. None does; reading it (a shared image
  registry, say) is a separate decision and would need its own values, schema and
  guard.
- `just ci`, `just test-doc` and the Rust suites were not run: no Rust code
  changed.
