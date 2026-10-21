# 2026-10-21 — the Flutter example's `pubspec.lock` tracks the plugin's version

Same branch and PR as
[2026-10-21-payer-dot-segment-ids.md](2026-10-21-payer-dot-segment-ids.md),
after `master` was merged in at `73a5b5af` (#312). The change is
`sdks/flutter/vpay_checkout_flutter/example/pubspec.lock` (one line),
`.github/workflows/release-please.yml` (one new step, and one widened), and the
two places that said the lockfile was deliberately left alone: that
workflow's own header and `AGENTS.md`.

## What was wrong

The example depends on the plugin by path, and pub records a path
dependency's version in the lockfile:

```text
  vpay_checkout_flutter:
    dependency: "direct main"
    description:
      path: ".."
      relative: true
    source: path
    version: "0.1.1"        <- the plugin's pubspec.yaml says 0.9.1
```

Nothing enforces it. No `--enforce-lockfile` appears anywhere, and no CI job
runs Flutter. So the stale value failed nothing. Instead, every local
`flutter test` or `flutter pub get` rewrote it to `0.9.1` and left a dirty
tree. This PR's own first `flutter test` did exactly that, and the edit had
to be reverted by hand.

It had been fixed by hand once already.
[2026-09-17-release-please-yaml-rewrite.md](2026-09-17-release-please-yaml-rewrite.md)
moved it from `0.1.0` to `0.1.1`. `release-please.yml` then listed it as
"deliberately NOT bumped", for two reasons: `flutter pub get` rewrites it,
and syncing it would need the Flutter SDK in the workflow. It then sat at
`0.1.1` through every release up to `0.9.1`. A hand fix alone would go stale
again at `0.9.2`.

## The change

- **The lockfile**: `flutter pub get` in `example/` changed exactly that one
  line, from `0.1.1` to `0.9.1`, and nothing else.
- **`release-please.yml`**: a new step, "Sync the Flutter example's
  pubspec.lock", runs on the release branch after "Refresh Cargo.lock".
  - It reads the version from `pubspec.yaml`'s `x-release-please-version`
    line, which release-please has already bumped on that branch.
  - An `awk` then rewrites the `version:` line inside the
    `vpay_checkout_flutter:` block, and exits 1 if no such line exists.
  - The existing commit step now stages `*pubspec.lock` beside `*Cargo.lock`.
    Its commit message became "chore: sync the lockfiles with the release
    version".
  - No Flutter SDK and no new action or permission is involved.
- **The two "deliberately not bumped" lists**, in the workflow header and in
  `AGENTS.md`, now say the lockfile was on them until 2026-10-21, and why
  it came off.

## What ran, on 2026-10-21 (macOS arm64, Flutter 3.50.0-0.1.pre)

The step was tested from the workflow file itself. Its `run:` block was
extracted with Ruby's YAML parser and executed with `bash`. Then
`pubspec.yaml` was set to `0.9.2` to simulate a release branch:

```text
1. the step, on the simulated 0.9.2 tree    exit 0; diff: 389c389  version: "0.9.1" -> version: "0.9.2"
2. flutter pub get on the same tree         cmp: IDENTICAL to what the step wrote
3. the step again                           cmp: unchanged (idempotent)
4. the step on a lockfile with the
   vpay_checkout_flutter block removed      "no vpay_checkout_flutter version line in the lockfile", exit 1
```

`pubspec.yaml` and the lockfile were restored afterwards, and `git status`
showed only the intended changes.

```text
actionlint .github/workflows/release-please.yml     exit 0   (shellcheck installed, so run: blocks are checked)
zizmor .github/workflows/release-please.yml         No findings to report. (2 suppressed; master has the same 2)
pnpm exec prettier --check (changed files)          All matched files use Prettier code style!
```

`just verify`'s output on the finished tree is on
[2026-10-21-payer-dot-segment-ids.md](2026-10-21-payer-dot-segment-ids.md),
which shares this branch.

## What did not run

- **No real release-please run.** The step runs only when release-please
  opens or updates a release PR on `master`. The local run used macOS's BSD
  `sed`/`awk` and bash 3.2. The runner uses Ubuntu's GNU `sed` and whichever `awk`
  its image provides. The script uses only POSIX features, but it has not
  executed there. The
  first release PR after this merges is the proof: it should carry a
  "chore: sync the lockfiles with the release version" commit that touches
  `example/pubspec.lock`.
- **No gate fails on a stale lockfile.** `verify-versions` reads only the
  files `release-please-config.json` lists, and this lockfile is not one of
  them. A hand bump that skips release-please could still strand it. That is
  the same exposure `CLAUDE.md` records for `examples/tauri-checkout`'s
  `Cargo.lock`.
