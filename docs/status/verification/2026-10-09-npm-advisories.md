# 2026-10-09 — The npm advisories that failed `trivy` and `web` on every PR

## What this is, and what it is not

A dependency-only change on `master` at `a33aac6`: the `next`, `brace-expansion`,
`sharp` and `source-map-js` advisories that failed CI's `trivy / Trivy` step
`Gate` and its `web` job's `audit-web` step on every pull request. No source
behaviour changed. Two source comments, the dashboard README and
`status-read-seam-and-bff.md`, which said "this app resolves Next 15.5.25", now
say it resolved that then and has resolved 15.5.27 since today.

**It does not turn `just audit-web` green.** One advisory is left, and it has no
fix to bump to: `braces` `<=3.0.3` (GHSA-vfj7-8cjw-p6xm, high) names `>=3.0.4`
as patched, and npm has no `braces` newer than `3.0.3`. See "What is left".
Trivy, which runs with `--ignore-unfixed`, is green.

## What the registry said, measured, not the brief

The brief listed eight Trivy findings and said `audit-web` reported seven. Run
on a clean `a33aac6` install (`pnpm install --frozen-lockfile`, pnpm 11.18.0,
Node 22.22.2), `pnpm audit --audit-level=moderate` — both of the recipe's runs
report the same totals — reported **18**: 1 critical, 8 high, 8 moderate,
1 low. More than the brief, for two reasons: advisories published after it was
written (the registry moves daily), and fixed versions later than the ones it
named.

| Package                     | Advisory                                                                                                               | Severity              | In the lockfile | Fixed in (this change)                                              |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------- | --------------------- | --------------- | ------------------------------------------------------------------- |
| `next` (`examples/shop`)    | GHSA-vcvr-r3jv-pc5j                                                                                                    | critical              | 16.3.4          | 16.3.6 → **16.3.8**                                                 |
| `next` (`examples/shop`)    | GHSA-cjq9-62q9-8jv4; GHSA-3w37-wq28-93x7, -4jqv-mc3x-m676, -f87g-xv8r-7p7x, -mcj8-r9mp-w47p; GHSA-39w2-rjm5-chcv (low) | high; 4 moderate; low | 16.3.4          | **16.3.8** (Trivy: CVE-2026-94483, high, 16.3.8)                    |
| `next` (`frontends/apps/*`) | GHSA-4jqv-mc3x-m676, GHSA-mcj8-r9mp-w47p                                                                               | moderate              | 15.5.25         | **15.5.27** (the range was `^15.5.25`; the floor is now `^15.5.27`) |
| `@next/eslint-plugin-next`  | rides `next`                                                                                                           | -                     | 16.3.4          | **16.3.8**                                                          |
| `brace-expansion` 1.x       | GHSA-6j4f-fj2g-mc7p (CVE-2026-102276), GHSA-qhr7-859c-m2p7 (CVE-2026-102278) high; GHSA-q2hr-2g5m-vwhr moderate        | high, moderate        | 1.1.18          | 1.1.19, 1.1.20 → **1.1.21**                                         |
| `brace-expansion` 5.x       | the same three                                                                                                         | high, moderate        | 5.0.9           | 5.0.10, 5.0.11 → **5.0.12**                                         |
| `sharp`                     | GHSA-wq5f-xc86-pv6w                                                                                                    | high                  | 0.35.4          | **0.35.5** (and `@img/sharp-libvips-*` 1.3.3 → 1.3.4 with it)       |
| `source-map-js`             | GHSA-68fv-2mgg-jv7q (CVE-2026-93749)                                                                                   | high                  | 1.2.1           | **1.2.2**                                                           |

`next` 15.5.x **is** affected (moderate, two advisories, fixed in 15.5.27), which
the brief asked to be checked rather than assumed.

The `brace-expansion` fixed version moved while this was being prepared: the
brief's 1.1.19/1.1.20 and 5.0.10/5.0.11 clear the two high advisories and leave
the moderate one. `1.1.21` and `5.0.12` (published 2026-09-14) clear all three.

## What changed

- `examples/shop/package.json`: `next` `16.3.4` → `16.3.8`, kept exact.
- `frontends/packages/config/package.json`: `@next/eslint-plugin-next` `16.3.4` →
  `16.3.8`, kept exact, in lockstep with the above.
- `frontends/apps/checkout/package.json`, `frontends/apps/dashboard/package.json`:
  `next` `^15.5.25` → `^15.5.27`.
- `pnpm-workspace.yaml`: four new `overrides` floors, one per major present in
  the lockfile — `brace-expansion@<1.1.21: ^1.1.21`,
  `brace-expansion@>=4.0.0 <5.0.12: ^5.0.12`, `sharp@<0.35.5: ^0.35.5`,
  `source-map-js@<1.2.2: ^1.2.2` — with a dated, advisory-by-advisory comment in
  the file's existing style, an update note on `next>postcss` (Next 16.3.8 now
  pins `postcss 8.5.23`, which is fixed; Next 15.5.27 still pins `8.4.31`, so
  the override stays load-bearing), and a "NOT cleared" paragraph for `braces`.
- `pnpm-lock.yaml`: regenerated by `pnpm install`, never by hand. Its package
  set differs from `a33aac6`'s only by the versions above.
- Docs: `docs/status/infrastructure.md` (a row), `docs/status/frontend.md` and
  `examples/shop/README.md` (the Next version), the four "resolves 15.5.25" sites above, this page and its index entry in
  `docs/status/README.md`.

**No `minimumReleaseAgeExclude` was needed or added.** Every target is past
pnpm 11's 24 h window — the newest, `source-map-js@1.2.2`, was published
2026-09-30 and the rest between 2026-09-14 and 2026-09-30. The setting was not
lowered or bypassed. (`@vaam-apps/ui@0.4.0`'s exclusion, whose own comment says
to remove it after 2026-09-25, is still in the file; not touched here.)

## The commands, and what they printed

All in a fresh worktree of `a33aac6`, `CI=1`, `CYPRESS_INSTALL_BINARY=0`.

**`just audit-web`, before** (`a33aac6`): exit 1, `18 vulnerabilities found`
(`{"low":1,"moderate":8,"high":8,"critical":1}`), both `--prod` and whole-workspace.

**`just audit-web`, after:** exit 1, `1 vulnerabilities found`, `Severity: 1 high`
— `braces`, path `frontends__packages__config>@next/eslint-plugin-next>fast-glob>micromatch>braces`.
Zero critical, zero moderate, and the high that remains is the one with no fix.

**Trivy 0.75.0** (`aquasec/trivy:latest` through Docker; the daemon was not
running when this started, so `dockerd` was started by hand), the exact CI gate:
`fs --scanners vuln,secret,misconfig --skip-dirs '**/node_modules,**/target,**/.git' --severity CRITICAL,HIGH --ignore-unfixed --exit-code 1`.

- before, on `git archive a33aac6`: **exit 1**, `pnpm-lock.yaml (pnpm) Total: 8 (HIGH: 7, CRITICAL: 1)` —
  brace-expansion CVE-2026-102276 and CVE-2026-102278 on both 1.1.18 and 5.0.9,
  `next` GHSA-vcvr-r3jv-pc5j and CVE-2026-94483, `sharp` GHSA-wq5f-xc86-pv6w,
  `source-map-js` CVE-2026-93749.
- after: **exit 0**, `pnpm-lock.yaml` row `pnpm | 0`.

Trivy does not report `braces`, because it is unfixed and the gate says
`--ignore-unfixed`. That is why Trivy is green and `audit-web` is not.

**The rest of the `web` job**, same worktree:

| Step                                                               | Result                                                                                                                                                                              |
| ------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pnpm install --frozen-lockfile`                                   | up to date after the regenerated lockfile                                                                                                                                           |
| `just fmt-check-web`                                               | exit 0, `All matched files use Prettier code style!`                                                                                                                                |
| `just lint-web` (`build-sdk-node`, typecheck, lint)                | exit 0                                                                                                                                                                              |
| `pnpm -r test`                                                     | exit 0 — **1506 passed, 2 skipped** (config 66, tokens 10, stripe-js 146, nodejs 229, api-client 4, tauri-plugin 71, shop 108, checkout 556 + 1 skipped, dashboard 316 + 1 skipped) |
| `examples/shop` `pnpm build` (`next build`, Next 16.3.8 Turbopack) | exit 0                                                                                                                                                                              |
| `frontends/apps/checkout` `pnpm build` (Next 15.5.27)              | exit 0                                                                                                                                                                              |
| `frontends/apps/dashboard` `pnpm build` (Next 15.5.27)             | exit 0                                                                                                                                                                              |
| `just build-storybook`                                             | exit 0                                                                                                                                                                              |
| `just test-storybook`                                              | exit 0 — checkout 26 passed, dashboard 33 passed                                                                                                                                    |

## What is left, and what was not done

- **`braces` GHSA-vfj7-8cjw-p6xm (high) is not cleared and cannot be by a bump.**
  `registry.npmjs.org/braces` has `dist-tags.latest = 3.0.3`, last modified
  2024-09-18, checked directly on 2026-10-09. The only path is
  `@vpay/config > @next/eslint-plugin-next > fast-glob@3.3.1 > micromatch >
braces`: a lint-time glob matcher, but `@vpay/config` lists the plugin under
  `dependencies`, so it also appears in the `--prod` run. `audit-web` therefore
  stays red until `braces` publishes `3.0.4` or the dependency leaves the tree.
  Not done, deliberately: `pnpm.auditConfig.ignoreGhsas`, or any override that
  swaps the glob stack. Silencing the one remaining finding is a maintainer's
  call, not a side effect of a lockfile refresh.
- The `cargo` half of CI (`just ci`'s Rust gates, `test-doc`, `test-rust`) was
  not run: no Rust file changed. `just verify` was.
- The `e2e` job (Cypress against the compose stack) was not run.
- Nothing was pushed and no pull request was opened.
