# Verification log — 2026-09-13, Flutter Lane B (`verify-sdk-parity` learns Dart)

Last verified: 2026-09-13, worktree `/home/selast/dev/vpay-flutter-lane-b`,
branch `claude/flutter-lane-b-gate`, rustc `1.98.0` (`rust-toolchain.toml`'s
pin, not the host default).

## Scope

Lane B of
[`docs/plans/2026-09-13-flutter-plugin-brief.md`](../../plans/2026-09-13-flutter-plugin-brief.md):
`cargo xtask verify-sdk-parity` reads Dart test titles and drops skipped ones,
`PARITY_SKIPPED_DIRS` grew `.dart_tool`/`build`, three `just` recipes and a
version pin exist for a plugin that does not, and the docs this brief names.
`sdks/flutter/` was not created and `docs/sdks/parity.md` was not touched, per
the hard rule for this lane. Every exit code below was read from a file the
command wrote itself (`; echo $? > file`), never from a harness banner.

## The required gates

| #   | Command                                                                                | Exit                        | Notes                                                                                                                                                                                                                                                                                                                                                                                            |
| --- | -------------------------------------------------------------------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | `cargo test -p xtask`                                                                  | **0**                       | **245 passed, 0 failed, 0 ignored, 0 measured, 0 filtered out.** Ten of those are the new Dart tests (`sdk_parity_tests::dart_*` and `sdk_parity_tests::a_dart_column_reads_test_titles_and_a_skipped_one_cannot_satisfy_a_tick`, `sdk_parity_tests::dart_tool_and_build_directories_are_skipped_like_node_modules`); the rest are the pre-existing suite, unchanged and still green.            |
| 2   | `cargo clippy -p xtask --all-targets -- -D warnings`                                   | **0**                       | One finding during development, fixed before this run: `chars[start]` in `skip_dart_string_literal` tripped `clippy::indexing_slicing` (a workspace `warn`, promoted to deny by `-D warnings`); replaced with `chars.get(start)` and an early return. `--all-targets` was used throughout, per the standing rule that `verify` never compiling tests has cost this project two CI rounds before. |
| 3   | `cargo run -p xtask -- verify-links`                                                   | **0**, after one real catch | First run (before this file and its cross-reference existed): `xtask: 1 broken link(s) — docs/status/README.md:27: mobile-flutter-plugin.md -> …` (the new status page was untracked; `verify-links` reads `git ls-files`) and, once staged, a second miss naming this very file before it existed. Green once both were written and staged.                                                     |
| 4   | `cargo run -p xtask -- verify-sdk-parity` (the real, unmodified `docs/sdks/parity.md`) | **0**                       | `verify-sdk-parity: ok — 469 proving test(s) named in docs/sdks/parity.md all exist, 31 dated gap(s), 32 SDK method(s) enumerated across 35 row(s)`. Unchanged behaviour on the existing two tables (Rust/Node) — this lane added no third column to the real document, and neither `sdks/rust` nor `sdks/nodejs` contains a `.dart_tool`/`build` directory for the new skip list to matter to.  |

## The decisive mutation (B1)

**Why not a subprocess against a synthetic tree.** `repo_root()`
(`.xtask/src/main.rs`) is `env!("CARGO_MANIFEST_DIR")`'s parent — fixed to
this checkout at **compile time** — so `cargo run -p xtask --
verify-sdk-parity` cannot be pointed at a tempdir, and the hard rule for this
lane forbids creating anything under `sdks/flutter/` or editing the real
`docs/sdks/parity.md` to fake a real invocation. The mutation is instead run
against [`verify_sdk_parity`](../../../.xtask/src/main.rs) itself — the exact
function `main()` dispatches the `verify-sdk-parity` command to, and whose
`Ok(())`/`Err(String)` `main` turns into `ExitCode::SUCCESS`/`FAILURE` — with
a synthetic `root` built from scratch in a tempdir (`docs/sdks/parity.md` and
an `sdks/flutter-fixture/test/` directory that exist nowhere else). This is
the same technique every other decisive mutation already proven in this file
uses (e.g. `an_ignored_or_skipped_test_cannot_satisfy_a_tick`,
`a_tick_naming_a_test_that_does_not_exist_fails_and_names_the_cell`).

| Step   | Fixture                                                 | `verify_sdk_parity(dir.path())` | Test                                                                       |
| ------ | ------------------------------------------------------- | ------------------------------- | -------------------------------------------------------------------------- |
| Before | `test('starts idle', () {});` cited by a ✅ cell        | `Ok(())`                        | `dart_decisive_mutation_a_live_test_passes_verify_sdk_parity`              |
| After  | the same test, `skip: true` added, nothing else changed | `Err(message)`                  | `dart_decisive_mutation_skip_true_fails_verify_sdk_parity_naming_the_cell` |

`cargo test -p xtask sdk_parity_tests::dart_decisive_mutation_a_live_test_passes_verify_sdk_parity -- --exact`
exits **0** (the `Ok(())` assertion holds). `cargo test -p xtask
sdk_parity_tests::dart_decisive_mutation_skip_true_fails_verify_sdk_parity_naming_the_cell
-- --exact` also exits **0** — the test's own assertion that the mutation
produces an `Err` naming the cell holds — and the literal message
`verify_sdk_parity` returned, captured with a temporary `eprintln!` during
this run and removed before committing, was:

```
sdk parity violations:
  - docs/sdks/parity.md:3 `checkout controller starts idle` / sdks/flutter-fixture: names the test `starts idle`, which does not exist under `sdks/flutter-fixture` (looked for a Rust `#[test]`/`#[tokio::test]` fn, a TypeScript `it("…")`/`test("…")`, or a Dart `test('…')`/`testWidgets('…')`/ `group('…')` with exactly that name, ignoring anything `#[ignore]`d or `skip:`ped)
```

That is the literal string a caller of `main()`'s `"verify-sdk-parity"` arm
would see on stderr, with `ExitCode::FAILURE` (`1` on this platform), naming
both the cell (`checkout controller starts idle`) and the column
(`sdks/flutter-fixture`) — the property B1 asked to be proven, proven with
the real function and a real mutation rather than asserted about it from a
distance. The message text itself was widened in this lane, once the first
capture (below) showed it still read "a Rust … or a TypeScript …" with no
mention of Dart or `skip:` — a payer reading this error for a Dart cell miss
would have been told to look for the wrong two languages. Both the message
(`test_names_in`'s caller, `.xtask/src/main.rs`) and its doc comment now name
all three readers and the skip rule; the quote above is the corrected text,
re-verified after the edit (`cargo test -p xtask` and `cargo clippy
--all-targets -- -D warnings` both re-run clean afterward, see the table
above).

A second, narrower Dart-only unit test
(`sdk_parity_tests::a_dart_column_reads_test_titles_and_a_skipped_one_cannot_satisfy_a_tick`)
proves the same property at the `parity_outcome`/`problems()` level already
used throughout this file's test suite, with a real `group()`/`test()` fixture
carrying one live and one `skip: true` test side by side, and asserts the
problem names the skipped title
("`reports pending, never succeeded, off a bare redirect`") rather than the
live one.

## Other properties proven, by unit test, against synthetic fixtures only

- `test`, `testWidgets` and `group` titles are all collected; single and
  double quotes, backslash escapes, and Dart raw strings (`r'…'`, where a
  backslash is literal) are all read correctly.
- `skip: false` does **not** suppress collection — only a truthy `skip:`
  (`true`, or any string reason) does.
- A `skip:` argument on a **nested** call inside a test's own body does not
  leak into the outer test's skip determination (`dart_call_is_skipped`
  tracks bracket depth and skips over string literals while scanning).
- `testHarness('…')` (keyword not immediately followed by `(`) and a bare
  `test` preceded by an identifier character are not mistaken for a real
  declaration, mirroring `ts_test_keyword_at`'s own boundary rules.
- `.dart_tool` and `build` are skipped exactly like `node_modules`: a test
  inside either cannot satisfy a ✅ cell
  (`dart_tool_and_build_directories_are_skipped_like_node_modules`).

## What this run does not prove, stated so it is not assumed

- **Nothing here ran against `sdks/flutter/vpay_checkout_flutter`** — it does
  not exist on this branch. Every Dart fixture above is synthetic, built and
  torn down inside a `TempDir` per test.
- **`just install-flutter`/`analyze-flutter`/`test-flutter` were run by hand,
  not as part of this table** — they are not `just verify`/`just ci` gates
  and this page does not claim they are. Manually confirmed: all three fail
  with the directory-missing message (not a stack trace) with Flutter on
  `PATH`, and `install-flutter`'s SDK-missing message was confirmed
  separately, against a scratch fixture outside this repository, with
  `flutter` stripped from `PATH`.
- **This did not touch `docs/status.md`'s gate table.** It still lists twelve
  gates; none of them is Flutter's.
