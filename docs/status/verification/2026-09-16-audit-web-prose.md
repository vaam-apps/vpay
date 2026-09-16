# 2026-09-16 — `audit-web`'s prose caught up with its recipe

**No capability moved. No gate changed. This is a documentation correction**,
recorded here because the thing corrected was a claim about what `just ci`
runs, and that claim had been wrong in five places for five days.

## What was wrong

Issue [#103](https://github.com/vaam-apps/vpay/issues/103) decided two things
and commit `96ebd20c` (2026-09-11,
[#126](https://github.com/vaam-apps/vpay/pull/126)) implemented both:

1. `pnpm audit` runs at `--audit-level=moderate`, not `high`;
2. `just audit-web` becomes part of `just ci`.

The recipe body was changed. Five pieces of prose describing it were not:

| Where                   | Said                                                         |
| ----------------------- | ------------------------------------------------------------ |
| `justfile`, `audit-web` | "`--audit-level=high` … A deliberate ceiling"                |
| `justfile`, `audit-web` | "NOT part of `just ci`"                                      |
| `justfile`, `ci`        | coverage list omitting `audit-web`                           |
| `package.json` `//pnpm` | "pnpm audit --audit-level=high"                              |
| `AGENTS.md`             | "Two further gates … in neither `just verify` nor `just ci`" |

All five now say what the recipe does, each carrying the date it was wrong
from, per this repository's convention.

## The consequence that was not being stated

**`just ci` needs the network, and has since 2026-09-11.** `audit-web` reaches
the npm registry's advisory endpoint on every run.

Three other recipes — `test-storybook`, `helm-check` and `check-schema` —
each justify their exclusion from `just ci` on the grounds that it must run
offline. That premise no longer holds on its own. Each of the three has a
second, surviving reason (a 115 MB browser download, two missing binaries, one
missing binary), and those are now what their comments give.

**Reconciling the four is a maintainer decision and this change does not take
it.** Either the offline promise is restored by taking `audit-web` out of
`just ci` — which reopens the hole #103 closed — or it is dropped, and the
other three exclusions are reconsidered on their own merits.

## Still open from #103

The third item in #103's decision — "a documented allowlist mechanism for an
accepted advisory (`pnpm audit`'s `--ignore` or an overrides entry, each with a
dated reason in status.md)" — **is not built.** `pnpm.auditConfig.ignoreCves`
is absent from `package.json` and no advisory is being suppressed (checked
2026-09-16). Until it exists, an accepted advisory has nowhere to be recorded,
and a moderate advisory in a transitive dev dependency blocks every merge with
no sanctioned way to accept it. #103 stays open for that item.

## Evidence

`just verify` — twelve gates and the `verify-docs` report — on this branch,
after the change. No test suite was run: this change touches comments, one
JSON prose array and one paragraph of `AGENTS.md`, and compiles nothing.
