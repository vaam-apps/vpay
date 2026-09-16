# Flutter checkout plugin — the adversarial review of Lanes A, B and C

**Date:** 2026-09-14. **Branch:** `claude/vpay-flutter-plugin-347b20`, the
merged head of the three lanes of
[`../../plans/2026-09-13-flutter-plugin-brief.md`](../../plans/2026-09-13-flutter-plugin-brief.md).
**Host:** Linux, Flutter 3.47.2 / Dart 3.13.2
([`flutter-toolchain.toml`](../../../flutter-toolchain.toml)), Rust 1.98.0
(`rust-toolchain.toml`). **No macOS, no iOS toolchain, no device, no browser,
no running vpay.**

Every exit code below was read from a file, never from a harness banner —
`<command> > <log> 2>&1; echo $? > <file>` — because this repository has
measured a background run reporting exit 0 for a run that exited 1.

## Gates on the final head

| Gate                                                 | Exit code | What it printed                                                                                                            |
| ---------------------------------------------------- | --------- | -------------------------------------------------------------------------------------------------------------------------- |
| `flutter test` (in the package)                      | `0`       | **80 passed, 0 skipped**                                                                                                   |
| `dart analyze --fatal-infos`                         | `0`       | No issues found                                                                                                            |
| `dart format --set-exit-if-changed lib test example` | `0`       | 22 files, 0 changed                                                                                                        |
| `cargo test -p xtask`                                | `0`       | **249 passed, 0 failed, 0 ignored**                                                                                        |
| `cargo clippy --all-targets -- -D warnings`          | `0`       | clean                                                                                                                      |
| `cargo run -p xtask -- verify-sdk-parity`            | `0`       | **551 proving tests, 35 dated gaps, 32 methods across 35 rows**                                                            |
| `cargo run -p xtask -- verify-links`                 | `0`       | 1 600 links in 352 tracked markdown files                                                                                  |
| `cargo run -p xtask -- verify-docs`                  | `0`       | advisory report, never fails                                                                                               |
| `flutter build apk --debug` (`example/`)             | `0`       | and the **app's** merged manifest carries `dev.vpay.checkout_flutter.VpayCheckoutActivity` with `android:exported="false"` |
| `flutter build web` (`example/`)                     | `0`       | and Flutter's generated `web_plugin_registrant.dart` calls `WebVpayCheckoutPlatform.registerWith`                          |

`just ci` is the orchestrator's gate and is not run here, by instruction.
`flutter test` is **not** in `just ci` or `just verify` (D-M3) — it is a
human running it, and the 80 above is that run.

## The mutations this review measured

A passing suite is also what a suite that asserts nothing looks like. Each
row is a real edit to the tree, the gate re-run, and the tree restored.

| #   | Mutation                                                                                         | Before                                   | After                                                                                                                                     |
| --- | ------------------------------------------------------------------------------------------------ | ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| A   | `VpayCheckout.start` ignores `mode` again (the defect as shipped)                                | `flutter test` rc `0`                    | rc `1`, failing `is refused with UnimplementedError, never silently downgraded to the in-app WebView`                                     |
| B   | `windowEvents` subscribed after `show()` resolves (the code as shipped)                          | rc `0`                                   | rc `1`, failing `an outcome reported after show() resolves is still received…` — `start()` hung and the test's own timeout named it       |
| C   | `_decode` lets `fromJson` throw again (the code as shipped)                                      | rc `0`                                   | rc `1`, four cases                                                                                                                        |
| D   | The Dart reader stops skipping comments and skipped groups (the code as shipped)                 | `cargo test -p xtask` rc `0`, 249 passed | rc `101`, 247 passed / **2 failed**: `a_skipped_group_takes_its_tests_with_it`, `commented_out_and_quoted_declarations_are_not_collected` |
| E   | `messages.g.dart`'s `toString` restored to what pigeon generates                                 | `flutter test` rc `0`                    | rc `1`, three cases naming the session secret                                                                                             |
| F   | `skip: true` on a newly cited Dart test                                                          | `verify-sdk-parity` rc `0`               | rc `1`, naming the cell, the column and the test                                                                                          |
| G   | `developer.log(secret)` added to `lib/`, **old** `no_logging_test.dart`                          | rc **`0` — the leak passed**             | with the fixed regex, rc `1`                                                                                                              |
| H   | `_resolve` stops checking the polled intent's id against the one asked for (the code as shipped) | `flutter test` rc `0`                    | rc `1`, failing `a succeeded intent with a different id is unresolved, never succeeded`                                                   |

Mutation G is the one worth reading twice: the D6 parity row was ✅ on a test
that a real credential leak walked straight past, because its lookbehind
excluded `.` and so excluded every member call.

## What was found, by severity

**Credential exposure.** The pigeon-generated `ShowCheckoutRequest.toString`
(Dart), `toString` (Kotlin) and `description` (Swift) all interpolated
`url` — whose fragment _is_ the checkout session's `client_secret`. The
design doc predicted exactly this ("generated code is how this regresses")
and the D6 row was ✅ with no test over that file. All three redact now; the
Dart one is pinned by `test/messages_redaction_test.dart`, the Kotlin one is
compiled but untested, and the Swift ones are **compiled by nobody**.
`no_logging_test.dart` missed qualified calls; fixed and measured.

**A public API that claimed a capability it did not have.**
`VpayCheckoutMode.externalBrowser` was accepted and silently ignored. It
throws `UnimplementedError` now.

**A gate that could be satisfied by tests that never run.** The Dart reader
collected a skipped group's tests, commented-out declarations and titles
quoted inside strings. Fixed; `verify-sdk-parity` stayed green at 551, which
is the other half of the proof — no ✅ cell was leaning on one.

**A hang, not a lie.** `start()` subscribed to a broadcast stream after
`show()` returned, so the one event it waits for could be dropped.

**A poll answer about a different object was reported as this session's
outcome.** `_resolve` never compared the returned intent's `id` with the one
it asked about, so a server answering about a different intent would have had
its `succeeded` reported here. It is an unresolved now.

**A contract violated.** `BrowserClient`'s own doc says nothing throws out of
a poll; a 200 with the right `object` and a wrong body threw a bare
`TypeError` through `CheckoutController` and out of `start`.

**Documents that had stopped being true.**
`docs/status/mobile-flutter-plugin.md` and `docs/flows/mobile-checkout.md`
both still said "No Dart file exists in this repository", written truthfully
by Lane B and left unamended by Lanes A and C (CLAUDE.md steps 2 and 3).
`docs/status.md`'s gate table still recorded `verify-sdk-parity` printing
463 proving tests and `verify-links` 1 450 links, both of which stopped being
true when Lane A's table landed. ADR-0021's Consequences section still said
"no Dart source exists at all yet"; it carries a dated note now.

## What this run did **not** verify

- **iOS and macOS: compiled by nobody.** There is no `xcodebuild` on this
  host and there cannot be. The Swift was reviewed by reading, including the
  four D5 non-negotiables (no `WKScriptMessageHandler`, no
  `evaluateJavaScript` bridge, `didReceive challenge` not implemented, the
  default persistent `websiteDataStore`). Reading is not compiling.
- **No device and no emulator.** Android is proven by compiling. No
  instrumentation test exists, and `VpayCheckoutActivity`'s WebView settings
  are asserted by reading the source, not by running it.
- **No browser.** `WebVpayCheckoutPlatform`'s popup, its `vpay:complete`
  listener and its `closed` poll have no automated test of any kind; the web
  host is proven by compiling and by the generated registrant, nothing more.
- **No running vpay and no real rail.** Every server in this package's suite
  is `package:http/testing.dart`'s `MockClient`. `just demo-up` was not run.
- **No CI gate** (D-M3), so none of the above is re-checked by anything but a
  human.
