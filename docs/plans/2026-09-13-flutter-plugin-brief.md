# Implementation brief — `vpay_checkout_flutter`

Design: [2026-09-13-flutter-plugin.md](2026-09-13-flutter-plugin.md). Read it
first; this brief does not restate its reasoning, only what to build and what
counts as proof.

- **Base:** `origin/master` at `7a607a68`, fetched and confirmed 2026-09-13.
- **Pipeline:** sonnet implements per lane in its own worktree → Opus sabotage
  review-and-fix over the whole branch → final gate on the final head.
- **The orchestrator briefs and verifies. It does not edit.**

## Rules every lane agent is bound by

Copy these into each agent's prompt verbatim; they are not preamble.

0. **Commit early, and keep committing.** Make a WIP commit as soon as you have
   written anything at all — within your first few minutes, before any gate run
   — and commit again after each meaningful step. Echo `git rev-parse HEAD` and
   confirm your branch has moved **off its base commit**.

   This is not bookkeeping. On 2026-09-13 a Lane B agent worked for twelve
   minutes, reached the `dart_test_names` seam, ran twelve cargo commands, and
   lost **all of it**: its worktree was removed out from under it mid-run and it
   had made no commit. Its branch still sat on the commit it was branched from,
   which is the cheapest possible proof that nothing was delivered. An
   interruption must cost you minutes, not the session.

1. **Work only in your own worktree, on your own branch.** Never `cd` to
   `/home/selast/dev/vpay`. **Worktrees for this work live outside
   `.claude/worktrees/`** — that directory is harness-managed and unlocked
   worktrees in it are pruned when the owning session ends, which is what
   destroyed the 2026-09-13 run. First action, and echo the output into your
   report:
   ```
   git rev-parse --show-toplevel && git rev-parse --abbrev-ref HEAD && git rev-parse HEAD
   ```
   If the branch is not the one this brief names for you, **stop and report**.
2. **Assert the branch at the top of every helper script you write**, and prefix
   every helper file with your lane letter (`laneA-`, `laneB-`, `laneC-`). The
   scratchpad is shared with other agents; an unprefixed `run.sh` has collided
   before.
3. **Do not `git stash`.** Use a WIP commit to set work aside.
4. **Never `pkill -f` by pattern.** Other agents share this host and a pattern
   kill has killed another agent's gate.
5. **Never `cargo install` into the shared bin.**
6. **When you have written your final report, stop. Take no further action** —
   no commits, no edits, no re-runs, and nothing in any other worktree. A
   finished agent that wakes on its own loop has acted inside a reviewer's tree
   before.
7. **An accurately-reported failure is worth more than a green I cannot trust.**
   If you cannot do something, say so plainly and leave it undone. You are
   explicitly permitted to decline or narrow scope with a reason. Do not weaken
   a test, add an allow, or narrow an assertion to make a gate pass.
8. **Exit codes go to a file, not to a banner.** Every gate you run:
   ```
   <command>; echo $? > /run/media/.../scratchpad/lane<X>-<gate>.rc
   ```
   Report the contents of the `.rc` file, and separately grep the log for
   `FAIL`/`error[`/`panicked`. A harness that reports exit 0 for a run that
   exited 1 has been measured in this project.
9. **Toolchain is CI's, not the host's.** Rust is `rust-toolchain.toml`'s
   `1.98.0`. Node is `.nvmrc`. Flutter is at
   `/home/selast/dev/tools/flutter/bin` (3.47.2, Dart 3.13.2).
10. **Do not run `just ci`.** It is the orchestrator's final gate. Run only the
    targeted recipes your lane names.

## What this host can and cannot verify

Stated up front because it decides what any lane may claim:

|                      | Verifiable here | How                                                                                                                              |
| -------------------- | --------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Dart / Flutter core  | **yes, fully**  | Flutter 3.47.2 is installed                                                                                                      |
| `.xtask` gate change | **yes, fully**  | cargo, pinned 1.98.0                                                                                                             |
| Flutter web build    | yes             | `flutter build web`                                                                                                              |
| Android host         | partly          | Android SDK at `~/Android/Sdk`, but `ANDROID_HOME` is unset and `adb`/`gradle` are absent. Treat a Gradle compile as best-effort |
| **iOS / macOS host** | **never**       | this is a Linux host; there is no `xcodebuild` and there cannot be. Swift here is **reviewed by reading only**                   |

No lane may describe iOS or macOS code as verified. Lane C's report must say
"compiled by nobody" in those words.

## Lane order, and why it is not negotiable

**Lane B lands before Lane A.** Lane A adds a third table to
`docs/sdks/parity.md` whose ✅ cells name Dart tests. `test_names_in`
(`.xtask/src/main.rs:4316`) reads `.rs` and TypeScript extensions and **nothing
else** today, so a Dart table landing first makes every ✅ cell name a test the
gate cannot find, and `just verify` fails. Lane C is independent of both.

**Lane C lands after Lane A**, and the earlier draft of this brief was wrong to
call C independent. Lane C implements against `pigeons/checkout.dart`, which
Lane A owns and creates; there is nothing for C to implement against until A
has landed it. Corrected 2026-09-13 before any lane started.

So the order is strictly **B → A → C**, each merged into the same branch before
the review. **Do not run build-heavy agents concurrently** — five concurrent
agents OOM-killed this host on 2026-09-10, and sequential is the honest cost of
a seam that only one lane can define.

---

## Lane B — the gate learns Dart, and the repository makes room

**Branch:** `claude/flutter-lane-b-gate`, worktree `/home/selast/dev/vpay-flutter-lane-b`
**Owns:** `.xtask/`, `justfile`, `docs/adr/`, `docs/flows/`, `docs/status*`,
the Flutter version pin. **Does not touch** `docs/sdks/parity.md` or
`sdks/flutter/`.

### B1 — `verify-sdk-parity` reads Dart

The seam is one `match` arm. In `.xtask/src/main.rs`:

- `test_names_in` (**:4316**) matches on file extension: `Some("rs")` →
  `rust_test_names`, TypeScript → `ts_test_names`. Add `Some("dart")` →
  `dart_test_names`.
- Write `dart_test_names` modelled on `ts_test_names` (**:4441**), which is a
  character walk, not a regex. It must collect the title of `test('…')`,
  `testWidgets('…')` and `group('…')`, handling single quotes, double quotes,
  escapes, and Dart's `r'…'` raw strings.
- **It must not collect a skipped test.** `test('x', skip: true)` and
  `skip: 'reason'` are Dart's spelling of `#[ignore]`, and the Rust reader
  deliberately drops those — `rust_test_names`' doc comment says why. A matrix
  cell citing a skipped test claims a capability proven by a test that never
  runs.
- `PARITY_SKIPPED_DIRS` (**:3141**) is a 5-element array; extend it with
  `.dart_tool` and `build`. Both hold generated and vendored Dart that would
  otherwise let somebody else's test satisfy a ✅ cell. **Update the array
  length in the type.**
- A parity column must be a directory that exists (**:3280**), and a table is
  recognised by its first header cell being `Capability` (`PARITY_TABLE_MARKER`,
  **:3136**).

**Tests: synthetic fixtures, the way the existing ones do it.** `.xtask`'s
parity tests build a synthetic SDK tree in a tempdir (**:8961** — `sdks/rust`
with a live `#[test]` and an `#[ignore]`d one, `sdks/nodejs` with an `it(...)`).
Extend that pattern; **do not** depend on `sdks/flutter/` existing, because in
this lane it does not.

**The decisive mutation, which you must run and report:** add a Dart test to
your synthetic tree, name it in a ✅ cell, then **make it `skip: true`** and
confirm the gate goes from exit 0 to **exit 1 naming that cell**. A reader that
collects skipped tests passes every other test you could write. Report the
before/after exit codes from `.rc` files.

### B2 — recipes and the pin

- `install-flutter`, `analyze-flutter` (`dart analyze --fatal-infos`),
  `test-flutter` in the `justfile`. They must work when `sdks/flutter/` does not
  yet exist — fail with a clear message, not a stack trace.
- Pin the Flutter version **in a file** the way `rust-toolchain.toml` pins Rust,
  so the day the gate lands it lands on a version. Note in the file that the
  host's install reports channel `[user-branch]`, which is not a clean channel
  pin and is a thing the maintainer may want to fix.
- **Do not add any of these to `just ci`.** That is decision D-M3.

### B3 — the honest record of a gate that does not run

This is the part most likely to be done quietly and must not be:

- `docs/status.md`'s gate table lists what `just verify` refuses. **Flutter is
  not in it.** Do not add a row implying otherwise.
- The dated ⛔ belongs in `docs/sdks/parity.md`, which **Lane A owns**. Lane B
  does not write it. Instead, Lane B's report must state that the row is owed,
  so the orchestrator can confirm Lane A wrote it.

### B4 — docs

- `docs/flows/mobile-checkout.md` — the flow, from the design doc. It must carry
  the design's **D9** (App Store / Play constraints) as a constraint on
  adoption, quoting Apple 3.1.3(e) and 3.1.1 rather than paraphrasing.
- An **ADR**. **Do not hard-code the number.** `origin/master` has up to `0019`
  (and two files both numbered `0018`), and there is uncommitted work in the
  main checkout that renumbers and claims `0020`. Take the next free number at
  branch time, check open PRs, and say in your report which number you took and
  what you checked.
- Status pages per `CLAUDE.md` step 2: the area page under `docs/status/`, plus
  a dated page under `docs/status/verification/` carrying your gate output.

### B5 — acceptance

- `cargo test -p xtask` (or the workspace equivalent) green, exit code from
  `.rc`.
- `cargo clippy --all-targets -- -D warnings` green. **`--all-targets`**: this
  project has lost two CI rounds to `verify` never compiling tests.
- `just verify-links` green — it reads markdown links only, so relative links in
  your new docs must resolve to tracked paths.
- The B1 mutation measured, both exit codes reported.

---

## Lane A — the Dart core

**Branch:** `claude/flutter-lane-a-core` (branched from Lane B's head), worktree `/home/selast/dev/vpay-flutter-lane-a`
**Owns:** `sdks/flutter/vpay_checkout_flutter/`, `docs/sdks/parity.md`.
**Does not touch** `.xtask/`, `justfile`, or any other doc.

### A1 — what to build

Everything in the design's D1, D2, D4, D6, D7 — **pure Dart, no platform code**:

- `src/browser_client.dart` — the read half of `@vaam-apps/vpay-stripe-js`,
  ported. `GET /v1/browser/checkout/sessions/{id}?key&client_secret` and
  `GET /v1/browser/payment_intents/{id}?key&client_secret`. Injectable HTTP
  client (the TS package does this and its tests depend on it).
- `src/checkout_controller.dart` — the state machine. **Pure**: no HTTP, no
  timers, no platform calls; it is handed a clock and a client. This mirrors
  `frontends/apps/checkout`'s pure reducer, for the same reason.
- `src/result.dart`, `src/errors.dart`, `src/redaction.dart`.
- `pigeons/checkout.dart` — the platform interface **definition only**. Lane A
  owns this file; Lane C implements against it and must not change it. Generate
  the Dart side; the platform stub throws `UnimplementedError` in this lane.
- `test/` — unit tests, no device.
- `example/` — a runnable app pointed at the `compose.demo.yml` stack.

### A2 — the properties that must be tests, not promises

Each of these is a parity row. The design's D1/D4/D6 say why:

- **The outcome is never read off a URL.** Reaching `success_url` with an intent
  that is `processing` must produce `pending`, not `succeeded`.
- **Dismissal polls before it reports.** A dismissal with a `succeeded` intent
  reports succeeded, not canceled.
- **`{CHECKOUT_SESSION_ID}` is substituted** before a URL becomes a stop rule.
- **Stop-URL matching is scheme+host+port+path**, query and fragment ignored.
- **An `embedded` session is refused** before any window opens.
- **The uniform 404** is mapped as one error, exactly as `browser::authenticate`
  renders it — the confidentiality property depends on the client not
  distinguishing the six causes either.
- **Redaction**: `toString()` on every type holding a session URL, a session
  secret or an intent secret renders `[N chars redacted]`; and there is **no
  `print`, `debugPrint` or `log` anywhere in `lib/`**, asserted by a test that
  reads the package's own source (`sdks/stripe-js` asserts its `console` rule
  this way).
- **A non-`https` base is refused** unless the named insecure opt-in is passed.

### A3 — the parity table

Add the **third table** to `docs/sdks/parity.md`, below the
`@vaam-apps/vpay-stripe-js` one, with the same framing (a payer surface, not a
third merchant SDK, so it shares no row with the merchant tables).

- First header cell must be exactly `Capability`.
- The single column header must be exactly:
  `` `sdks/flutter/vpay_checkout_flutter` `` — it must be a real directory.
- Every ✅ names Dart tests that exist and are not skipped.
- **The dated ⛔ rows this lane owes**, each with `2026-09-13`, a reason and an
  owner:
  - the tests are **not run by `just ci`** (D-M3);
  - **no platform host exists** — every platform call throws
    `UnimplementedError` (Lane C);
  - **nothing has run against a real rail**, like everything else here.

### A4 — acceptance

- `dart analyze --fatal-infos` clean; `flutter test` green with **the counts
  reported, including how many are skipped**. "The tests pass" without a skip
  count is half an answer.
- `cargo run -p xtask -- verify-sdk-parity` **exit 0**, and report its printed
  counts.
- **The decisive mutation:** delete one ✅ cell's named test from the Dart source
  and confirm `verify-sdk-parity` goes to **exit 1 naming that cell**. If it
  stays green, Lane B's reader is not working and you must say so rather than
  proceed — that is the exact failure this repository measured in 2026-09-06 and
  again on 2026-09-08.
- **Evidence against a real stack, if you can get one up:** `just demo-up`, then
  drive `browser_client` against it for a real session read and a real intent
  poll. If it does not come up, say so; do not fake it.

---

## Lane C — the platform hosts

**Branch:** `claude/flutter-lane-c-platforms`, worktree `/home/selast/dev/vpay-flutter-lane-c`, branched from Lane A's head —
C cannot start before A, see "Lane order".
**Owns:** `sdks/flutter/vpay_checkout_flutter/{android,ios,macos}/` and the web
implementation. **Does not touch** `pigeons/checkout.dart` (Lane A owns it),
`docs/sdks/parity.md`, `.xtask/`, or `justfile`.

Build the design's D5 and D8. The non-negotiables are listed there and are
repeated here because they are the ones a reviewer will check first:

- Android `VpayCheckoutActivity` is **`android:exported="false"`**.
- `onReceivedSslError` is **not overridden**; `decidePolicyFor` does not bypass
  certificate validation.
- **No `addJavascriptInterface`, no `WKScriptMessageHandler`** (design D3).
- `allowFileAccess`, `allowFileAccessFromFileURLs`,
  `allowUniversalAccessFromFileURLs` all **false**.
- Persistent WebView data store (D-M5); `minSdk` 21, iOS 12.0 (D-M4).
- External mode: Custom Tabs on Android, **`SFSafariViewController`** on iOS
  below 17.4 — **not** `ASWebAuthenticationSession`, for the reason in D8.
- **No custom URL scheme anywhere.** If you find yourself adding one, stop and
  report: D8 decided against it and the reason is a hijack class, not a taste.

**What you may claim.** Android and web: whatever you actually compiled, with
the command and the `.rc`. **iOS and macOS: nothing.** Your report must say
"compiled by nobody" and the parity ⛔ rows must say the same. Do not write a
test that asserts Swift source text matches a regex and call it proof.

---

## The review stage

**Branch:** the merged head of A+B+C. **Model: Opus. Sabotage review-and-fix,
free to rewrite.** Give it this brief and the design doc.

Before it starts: **`TaskStop` every implementer.** A finished implementer
wakes on its own stale loop and has acted inside a reviewer's tree before.

The reviewer's own instructions:

1. **Re-run the original failures.** Re-run every mutation this brief names —
   B1's `skip: true`, A4's deleted test — and confirm each still flips the gate
   from 0 to 1. Compiling and passing is also what a fix that papers over the
   problem does.
2. **Install and gate under CI's exact pins**, not the host's newer toolchain. A
   sample failed CI on a Node engine constraint every local run had satisfied.
3. **Lenses to apply, in this order:**
   - **Does anything claim success it did not observe?** The single highest-risk
     defect in this plugin is a code path that reports `succeeded` from a URL,
     a dismissal, or a timeout. Hunt for it specifically.
   - **Can a credential reach a log, a `toString`, an Android extra that
     outlives the window, or an error message?** Generated code is how the
     redaction regresses.
   - **Is every ⛔ in the parity table true, and is every ✅ proven by a test
     that actually runs?** Check the skip handling by mutation, not by reading.
   - **Does any document claim something ran that did not?** Especially iOS.
4. **Review the remediation too.** Whatever you fix, re-check that the fix did
   not weaken a test or narrow an assertion until the complaint stopped
   applying. This stage is unreviewed by default and is where that pressure
   concentrates.

## Final gate — the orchestrator's, not any agent's

On the final head, in the worktree, `just ci`, with the exit code read from a
file and not from a banner:

```bash
just ci > /tmp/vpay-flutter-ci.log 2>&1; echo $? > /tmp/vpay-flutter-ci.rc
```

Then: read the `.rc`, `grep -nE 'FAIL|error\[|panicked|warning: unused' ` the
log, and confirm `verify-sdk-parity`'s printed counts moved by the number of
rows Lane A added. `just ci` does **not** run the Flutter tests (D-M3), so run
`just test-flutter` separately and report its counts and skips beside the `ci`
result, labelled as the un-gated run it is.

## What "done" means for this brief

- `just ci` exit 0, read from the file.
- `just test-flutter` green, counts and skips reported, labelled un-gated.
- Both mutations measured, before/after exit codes reported.
- Every ⛔ in the new parity table dated `2026-09-13` with a reason and an owner.
- The summary states explicitly what was **not** done — at minimum: no real
  rail, no iOS or macOS compile, no device, no CI gate, no store review.
