# D8 — `VpayCheckoutMode.externalBrowser` wired end to end

**Date:** 2026-09-14. **Branch:** `claude/flutter-d8-external-browser`, base
`eed4de6d` (merge of `#175`). **Host:** Linux, Flutter 3.47.2 / Dart 3.13.2
(`flutter-toolchain.toml`), Rust 1.98.0 (`rust-toolchain.toml`). **A real
Android emulator was driven for this lane** — a dedicated AVD,
`vpay_d8_avd`, API 35 `google_apis`, created for this run and deleted
afterward. Still **no macOS, no iOS toolchain, no real rail** — every
payment in this repository settles against WireMock.

Every exit code below was read from a file, never from a harness banner —
`<command> > <log> 2>&1; echo $? > <file>` — the same discipline the
2026-09-14 review used, for the same reason: a background run has been
measured reporting exit 0 for a run that exited 1.

## What this lane built

`pigeons/checkout.dart`'s `ShowCheckoutRequest` gained a `CheckoutWindowMode`
field (`inApp`/`externalBrowser`), regenerated for Dart, Kotlin and Swift and
threaded through `VpayCheckoutPlatform.show(mode:)` and
`VpayCheckout.start(mode:)`. The `UnimplementedError` `VpayCheckout.start`
used to throw for `externalBrowser` before the pre-flight ever ran is gone —
the mode is always threaded through, and a platform host decides.

- **Android** — `VpayCheckoutExternalBrowserSession` launches a
  `CustomTabsIntent` (`androidx.browser:browser:1.8.0`) and reports a
  dismissal the moment the host `Activity` itself resumes
  (`Application.ActivityLifecycleCallbacks`) — no `startActivityForResult`,
  no custom URL scheme, ever (D8's own non-negotiable).
- **Web** — `WebVpayCheckoutPlatform.show` accepts `mode` and documents that
  `inApp`/`externalBrowser` collapse to the identical `window.open` popup.
- **iOS** — `VpayCheckoutExternalBrowserSession` wraps
  `SFSafariViewController`, never `ASWebAuthenticationSession` (design doc
  D8 explains why).
- **macOS** — `VpayCheckoutExternalBrowserSession` opens the payer's default
  browser via `NSWorkspace` and watches for this app's own reactivation —
  the maintainer's own call, since no `SFSafariViewController` or
  Custom-Tabs equivalent exists on macOS; recorded and justified in that
  file's own header.
- **Not built**: D8's tier 1 (Android App Links / iOS 17.4+ Associated
  Domains) — needs a merchant-hosted deployment this repository cannot
  provide.

## Gates on the final head

| Gate                                                                                 | Exit code | What it printed                                                                                     |
| ------------------------------------------------------------------------------------ | --------- | --------------------------------------------------------------------------------------------------- |
| `flutter test` (in the package)                                                      | `0`       | **82 passed, 0 skipped** (was 80 before this lane added two)                                        |
| `dart analyze --fatal-infos` (package)                                               | `0`       | No issues found                                                                                     |
| `dart analyze --fatal-infos` (example/)                                              | `0`       | No issues found                                                                                     |
| `dart format --set-exit-if-changed .` (package)                                      | `0`       | 30 files, 0 changed                                                                                 |
| `dart format --set-exit-if-changed .` (example/)                                     | `0`       | 7 files, 0 changed                                                                                  |
| `flutter build apk --debug` (`example/`)                                             | `0`       | built; release DEX unaffected — see below                                                           |
| `flutter build apk --release` (`example/`)                                           | `0`       | built (48.6 MB)                                                                                     |
| `just test-flutter-e2e`                                                              | `0`       | 3 passed, 0 skipped, against the already-running demo stack (not started or torn down by this lane) |
| `cargo run -p xtask -- verify-sdk-parity`                                            | `0`       | **559 proving tests, 35 dated gaps, 32 methods across 35 rows** (was 556 proving / 35 gaps)         |
| `cargo run -p xtask -- verify-links`                                                 | `0`       | 0 broken links, once this page existed to satisfy the two links pointing at it                      |
| `just fmt-check-web` (repo-wide, prettier --check .)                                 | `0`       | All matched files use Prettier code style                                                           |
| `example/integration_test/checkout_external_browser_test.dart` (real AVD, run twice) | `0`, `0`  | **All tests passed!** — see "The emulator run" below                                                |
| `example/integration_test/checkout_dismiss_test.dart` (same AVD, in-app mode)        | `0`       | **All tests passed!** — re-run to confirm the Kotlin plugin changes did not regress `inApp`         |

`just ci` is the orchestrator's gate and is not run here, by instruction.
None of the Flutter recipes are in `just ci` or `just verify` yet (D-M3) —
every count above is a human running the recipe by hand.

## The redaction re-application (the trap this task named explicitly)

Regenerating pigeon with the new `mode` field overwrote all four hand-edited
`toString`/`description` overrides, exactly as predicted. Redaction was
re-applied in all four files and the counts, measured with
`grep -ic redact`, match the pre-existing baseline exactly:

| File                               | Count |
| ---------------------------------- | ----- |
| `lib/src/platform/messages.g.dart` | 4     |
| `android/.../Messages.g.kt`        | 4     |
| `ios/.../Messages.g.swift`         | 3     |
| `macos/.../Messages.g.swift`       | 3     |

`test/messages_redaction_test.dart` passes against the re-applied redaction
— it was never relaxed, and the test file itself gained a `mode` assertion
(`mode: CheckoutWindowMode.inApp` renders in full, since a mode is not a
credential) rather than being narrowed.

## The DEX counts (release must stay clean, debug must not)

Measured with `unzip` + `strings classes*.dex | grep -c
evaluateJavascriptForTests` on both APKs built from this head:

| Build                         | Count | Expected |
| ----------------------------- | ----- | -------- |
| `flutter build apk --debug`   | `4`   | `> 0`    |
| `flutter build apk --release` | `0`   | `0`      |

Unchanged from the pre-D8 baseline: D8's Android host lives entirely in
`android/src/main/kotlin`, and adds no call to `evaluateJavascript` anywhere.

## The emulator run — the strongest available evidence

A dedicated AVD, `vpay_d8_avd` (`system-images;android-35;google_apis;x86_64`,
device profile `pixel_6`), was created, booted headless (`-no-window
-no-audio -no-boot-anim -gpu swiftshader_indirect -no-snapshot`), and driven.
The pre-existing `webank_kyc` AVD on this host was never touched.

**A browser turned out to be present** — this AVD's `google_apis` image
ships Chrome (`com.android.chrome`), contrary to this task's own stated
expectation that a `google_apis` image "may ship none." Confirmed before
driving anything:

```
$ adb shell cmd package query-services -a android.support.customtabs.action.CustomTabsService
1 services found:
  Service #0:
    name=org.chromium.chrome.browser.customtabs.CustomTabsConnectionService
    packageName=com.android.chrome
```

Two real, independently-minted hosted Checkout Sessions were minted through
`examples/shop`'s real server (`orders.create`, a real `POST
/v1/payment_intents` + `POST /v1/checkout/sessions`), the same fixture shape
`just test-flutter-emulator` mints, passed in as `--dart-define
=VPAY_E2E_FIXTURE_B64=…` (never `Platform.environment` — `integration_test`
runs on the device, a separate process the host shell's environment does
not reach).

**Run 1 and Run 2 of `checkout_external_browser_test.dart`** (identical
procedure, both to confirm the result was not a fluke):

1. `platform.show(mode: externalBrowser)` called from the Dart test running
   ON the device.
2. The test printed `LANE_D8_EXTERNAL_BROWSER_READY` and waited.
3. From the host: `adb shell dumpsys activity activities | grep
topResumedActivity` — both runs showed
   `com.android.chrome/org.chromium.chrome.browser.firstrun.FirstRunActivity`
   as the resumed activity (Chrome's first-run flow, since this AVD had
   never opened Chrome before) — proof the `CustomTabsIntent` really did
   launch a real, separate app.
4. `adb shell input keyevent 4` (hardware back) sent from the host.
5. Both runs: `All tests passed!`, exit `0`, the Dart assertions
   (`event.outcome == CheckoutWindowOutcome.dismissed`,
   `event.reachedUrl == null`) held — the real dismissal arrived over the
   real platform channel within the 45-second timeout both times.

**`checkout_dismiss_test.dart` (in-app mode) on the same device**, to
confirm `VpayCheckoutFlutterPlugin.kt`'s restructuring (splitting `show`
into `showInApp`/`showExternalBrowser`) did not regress the existing
in-app path: `adb dumpsys` confirmed `VpayCheckoutActivity` as the resumed
activity before the back press, and the suite reported `All tests passed!`,
exit `0`.

Cleanup, in order: `adb reverse --remove-all`, `adb emu kill`, then
`avdmanager delete avd -n vpay_d8_avd`. `avdmanager list avd` afterward
showed only `webank_kyc`.

## What this does not prove

- **Not a real rail.** The minted sessions settle against WireMock, like
  every session in this repository's history.
- **Not iOS, not macOS.** Both hosts are compiled by nobody — this host has
  no Xcode, and `swiftc`/`swift` are confirmed absent from `PATH` too.
- **Not tier 1.** No App Links / Associated Domains deployment exists to
  prove a self-closing external window against.
- **Not the web popup's runtime behaviour** for either mode — no browser has
  driven it.
- **Not a real device, not a store review.**
