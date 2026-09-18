# 2026-09-17 — the first release rewrote two YAML files, and what the repair left behind

**No capability moved.** This is release-tooling fallout across three pull
requests. It is dated 2026-09-17 because that is the night the break landed;
every measurement below was taken on 2026-09-18, on
`fix/release-please-yaml-extra-files`, on top of
[#204](https://github.com/vaam-apps/vpay/pull/204).

## The sequence

| PR                                                              | Date       | What it did                                                                                                                         |
| --------------------------------------------------------------- | ---------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| [#201](https://github.com/vaam-apps/vpay/pull/201)              | 2026-09-17 | release-please proposes the version bump, and `verify-versions` becomes `just verify`'s thirteenth gate                             |
| [#203](https://github.com/vaam-apps/vpay/pull/203) (`eb078020`) | 2026-09-17 | the first release, `v0.1.1`. It destroyed `deploy/helm/vpay/Chart.yaml` (48 lines → 13) and `sdks/flutter/…/pubspec.yaml` (60 → 45) |
| [#204](https://github.com/vaam-apps/vpay/pull/204) (`aeb9e242`) | 2026-09-18 | restored both files, made every `extra-files` entry `{"type": "generic"}`, taught the gate to refuse a bare string                  |
| this branch                                                     | 2026-09-18 | the tests neither of those landed, the hole they left named, and the three pieces of #203's fallout still on `master`               |

## What broke, and the mechanism

CI run
[35275177452](https://github.com/vaam-apps/vpay/actions/runs/35275177452) on
`master` was red in two jobs after #203:

| Job           | Step                                   | What it said                                                                                                                       |
| ------------- | -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `self-checks` | `verify-versions`                      | both YAML files are listed in `extra-files` "but carries no x-release-please-version line, so release-please will never change it" |
| `web`         | `fmt-check-web` (`prettier --check .`) | `AGENTS.md`, `CHANGELOG.md`, `deploy/helm/vpay/Chart.yaml`, `sdks/flutter/vpay_checkout_flutter/pubspec.yaml`                      |

The first was the gate reporting damage it could not prevent. The cause is a
wrong assumption in #201, not a bug in release-please: a **bare-string**
`extra-files` entry does not get the annotation-only `Generic` updater.
release-please infers one from the file extension in
`src/strategies/base.ts`'s `extraFileUpdates`, and `.yaml` gets
`CompositeUpdater(GenericYaml('$.version'), Generic)` — document updater first,
and `GenericYaml` is js-yaml's `loadAll` followed by `dump`, whose own doc
comment says the parser "does reformat the document and removes all comments".
[status/gates.md](../gates.md) § 2026-09-18 carries the full dispatch table and
why the `Cargo.toml` and `.ts` entries came through untouched.

#204 fixed the configuration and the gate. **It did not finish the fallout**,
and that is what this branch is.

## What was still broken on `master` at `aeb9e242`

Measured, not assumed:

```text
$ pnpm exec prettier --check .
Checking formatting...
[warn] AGENTS.md
[warn] CHANGELOG.md
[warn] Code style issues found in 2 files. Run Prettier with --write to fix.
```

So CI's `web` job was **still red**, ten hours after the release and after the
pull request that fixed the release. Three more, none of which any gate catches:

1. `grep -n "release_please_extra_files\|classify_extra_file\|value_after"` over
   `.xtask/src/main.rs` found the definitions and the call sites and **no test**.
   Two pull requests moved this gate; neither landed one.
2. `deploy/helm/vpay/Chart.yaml` had `appVersion: "0.1.1"` and
   `version: 0.2.0`. The file's own comment three lines below the chart version
   says `values.yaml`'s `images.*.tag` defaults to `appVersion`, "so a chart
   bump and an image bump are the same edit" — so a chart still called `0.2.0`
   now resolves a different image than the `0.2.0` anyone already has.
3. `AGENTS.md` and [docs/status.md](../../status.md) both said `just verify` is
   **twelve** gates, and `docs/status.md`'s gate table had no `verify-versions`
   row at all, while the `justfile` recipe echoes "the thirteen gates above
   passed". AGENTS.md's own paragraph asks the next change to fix precisely
   this; it took three.

`sdks/flutter/vpay_checkout_flutter/example/pubspec.lock` was also still
recording `version: "0.1.0"` for its path dependency on the plugin, whose
`pubspec.yaml` #204 moved to `0.1.1`.

## The repair

| File                                                              | Change                                                                                                                          |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `.prettierignore`                                                 | `CHANGELOG.md`, with the reason paragraph every entry there carries                                                             |
| `AGENTS.md`                                                       | `prettier --write` (two emphasis markers, formatting only), and the gate count twelve → thirteen with `verify-versions` named   |
| `.xtask/src/main.rs`                                              | a `version_tests` module: nine tests over `release_please_extra_files`, `classify_extra_file`, `value_after` and `first_semver` |
| `deploy/helm/vpay/Chart.yaml`                                     | chart `version:` `0.2.0` → `0.2.1`, with the reason and #203's downgrade recorded in the file                                   |
| `docs/status.md`, `docs/status/gates.md`, `docs/status/README.md` | the missing gate row, the counts, and the record                                                                                |
| `sdks/flutter/vpay_checkout_flutter/example/pubspec.lock`         | one line: the path dependency's recorded version `0.1.0` → `0.1.1`                                                              |

**Deliberately not changed: `release_please_extra_files` itself.** #204 states
that this parser is kept identical to `vsms`' own copy so the two cannot drift.
It has one hole — a single-line object with a `"path"` and no `"type"` matches
no branch and is skipped in silence, where the multi-line spelling of the same
mistake is a hard error — and closing it needs the same edit in both
repositories. It is pinned by
`a_single_line_object_with_no_type_is_skipped_silently_and_that_is_a_known_hole`
and named in `gates.md`, so it is a known gap rather than an unknown one.

## Mutations

Applied by hand to the real files, gate run, exit code recorded, restored.
`verify-versions` printed `ok — 21 version references all say 0.1.1` again
afterwards.

| #   | Mutation                                                               | Exit | Refused with                                                                                                                                                                          |
| --- | ---------------------------------------------------------------------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `Chart.yaml`'s entry back to a bare string — **#203's own config**     | 1    | "is a BARE STRING. release-please picks an updater from the file extension for those… it destroyed this repo's Chart.yaml (48 lines -> 13, every comment gone) on the v0.1.1 release" |
| 2   | `"type": "generic"` → `"type": "yaml"` on the same entry               | 1    | "has type \"yaml\". Only \"generic\" and \"json\" are used here; anything else either reparses the file or needs this check taught about it"                                          |
| 3   | `# x-release-please-version` stripped from `Chart.yaml`'s `appVersion` | 1    | "listed in release-please-config.json's extra-files but carries no x-release-please-version line, so release-please will never change it"                                             |

Mutations 1 and 2 also exist as tests now, which is the point: **mutation 1's
config was this repository's own state two commits ago, and every gate here was
green on it.** A rule proven by one hand-edit is proven until the next edit.

## Evidence

Run on this branch, one invocation each, 2026-09-18. `just ci` was **not** run:
sibling agents held the Rust workspace build and the web suites on this host,
and every recipe this change can affect is below.

| Command                             | Result                                                                                                                                     |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `just fmt-check`                    | `cargo fmt --all -- --check` clean; `prettier --check .` — "All matched files use Prettier code style!" **(this is the job that was red)** |
| `just clippy`                       | `--workspace --all-targets -- -D warnings`, exit 0, no warnings                                                                            |
| `just verify`                       | `verify: ok — the thirteen gates above passed; the verify-docs report is advisory`                                                         |
| `cargo xtask verify-versions`       | `ok — 21 version references all say 0.1.1`                                                                                                 |
| `cargo xtask verify-links`          | `ok — 1712 repository link(s) in 398 tracked markdown file(s) resolve to a tracked path`                                                   |
| `cargo xtask verify-status`         | `ok — 1 unimplemented item(s)` — unchanged; this retires no token                                                                          |
| `cargo test -p xtask`               | **259 passed, 0 failed, 0 ignored** (250 before, nine new in `version_tests`)                                                              |
| `just test-doc`                     | 121 passed, 0 failed, **1 ignored** — a separate runner from nextest                                                                       |
| `just helm-check`                   | `ok — lint, 4 renders, 24 guards, … kubeconform`; "68 resources found in 4 files - Valid: 68, Invalid: 0". No cluster was involved.        |
| `flutter test` (plugin)             | **315 passed**, 0 skipped                                                                                                                  |
| `verify-coverage.mjs` (vpay-skills) | exit 0 — `20 skills, 205 vpay paths claimed, 45 feature pages, 0 exempt`                                                                   |

`just ci` was deliberately not run; `just test-rust`, `just test-web`,
`just lint-web`, `just audit-web` and `just deny` were not run either. Nothing
in this change is reachable from a shipping binary, and the Rust workspace and
web suites were held by sibling agents on this host.

## What this does not do

- **It does not prove the next release is correct.** Nothing here runs
  release-please. The gate reads two JSON files as text: it can say the config
  is in a shape whose behaviour is known and that every version it names agrees
  today, not what release-please will write. **The first real confirmation is
  the next release pull request's own diff — read it before merging it.**
- **It does not close the single-line-object hole**, for the drift reason above.
- **It changes no `docs/flows/` page**, because it moves no flow — the release
  pipeline has no flow page.
- **It does not touch `CHANGELOG.md`.** release-please owns that file; the fix
  is the `.prettierignore` entry, not an edit.
- **It retires no `NotImplemented` token.** `verify-status` still reports 1.
