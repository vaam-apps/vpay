# The payment window is a modal bottom sheet, not a full-screen window

**Date:** 2026-09-16. **Branch:** `claude/flutter-web-e2e`, base `007e86ae`.
**Host:** Linux, Flutter 3.47.2 / Dart 3.13.2. **A real device was driven for
this change** — the maintainer's own `emulator-5554` (Pixel 10a), already
booted; no AVD was created, deleted, or killed. The demo stack
(`vpay-demo-*` compose project) was already up and was not torn down.

Every exit code below was read from a file, never from a harness banner —
`<command> > <log> 2>&1; echo $? > <file>`.

## The request

The maintainer, verbatim: the full-screen Android `Activity` "feels like the
user is quitting the app." This revises design D5
(`docs/plans/2026-09-13-flutter-plugin.md`, "D5, revised 2026-09-16" —
the reasoning, alternatives and the maintainer's own decisions on height and
detent all live there and are not restated here).

## What changed

- `sdks/flutter/vpay_checkout_flutter/android/src/main/AndroidManifest.xml`
  — `VpayCheckoutActivity`'s theme moved from
  `@android:style/Theme.NoTitleBar.Fullscreen` to
  `@style/Theme.Vpay.CheckoutSheet`, a new translucent theme
  (`android/src/main/res/values/styles.xml`, new file).
  `android:exported="false"` is untouched.
- `VpayCheckoutActivity.kt` — `onCreate` now builds a scrim `View` plus a
  `CoordinatorLayout`/Material `BottomSheetBehavior` sheet around the
  `WebView` (`buildSheetView`), instead of passing the `WebView` straight to
  `setContentView`. `isFitToContents = false` with
  `halfExpandedRatio = 0.9f` is the maintainer's explicit "~90% of screen"
  detent; `skipCollapsed = true` means a drag past it goes straight to
  hidden. A new `requestDismiss()` is the single function back press, the
  scrim's tap listener, and (indirectly, via the `BottomSheetCallback`) a
  drag-to-dismiss all resolve through; the callback's `onStateChanged`
  handler is the ONE place that calls the pre-existing `finishAsDismissed()`
  — no second, separate "cancel" path (design D4). Nothing about
  `onReceivedSslError`, `addJavascriptInterface`, the `allowFileAccess*`
  flags, back-press-not-history-pop, or `onDestroy`'s WebView teardown
  changed.
- `android/build.gradle.kts` — two new dependencies:
  `com.google.android.material:material:1.14.0` (for `BottomSheetBehavior`)
  and `androidx.coordinatorlayout:coordinatorlayout:1.2.0` (its required
  parent). Both are the current stable releases as of this date (checked
  against `dl.google.com/android/maven2`'s own `maven-metadata.xml`).
- `ios/.../VpayCheckoutViewController.swift` — the initializer now guards
  `modalPresentationStyle = .pageSheet` / `isModalInPresentation = false`
  behind `#available(iOS 13.0, *)`, falling back to `.fullScreen` on iOS 12
  (previously unguarded, which would not have compiled against the plugin's
  own iOS 12.0 deployment target, D-M4 — this was latent and is now
  correct). `viewDidLoad` sets `sheetPresentationController?.detents =
[.large()]` and `prefersGrabberVisible = true` behind
  `#available(iOS 15.0, *)`.
- No change to `macos/.../VpayCheckoutViewController.swift` (already
  presents `as a sheet`, unaffected by this revision) or to any Dart file,
  the web host, or the pigeon-generated seam.

**iOS and macOS are compiled by nobody** — this repository has no
Xcode/`swiftc` toolchain on Linux, confirmed absent again this run
(`which swiftc swift xcodebuild` all empty). Reviewed by reading only.

## Non-negotiables, re-measured on the new theme — none assumed unchanged

| Non-negotiable                                                                                 | How it was checked                                                                                                                        | Result                                                                                                                                          |
| ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `android:exported="false"`                                                                     | `aapt2 dump xmltree` against the **built APK**, not the source manifest — both debug and release, both from a fresh `flutter clean` build | Present, `false`, in both                                                                                                                       |
| `onReceivedSslError` not overridden                                                            | Read `VpayCheckoutActivity.kt` in full after the edit                                                                                     | Still absent — the file's own header still names its absence as the audit surface                                                               |
| No `addJavascriptInterface`                                                                    | Read `VpayCheckoutActivity.kt` in full after the edit                                                                                     | Still absent                                                                                                                                    |
| `allowFileAccess`/`allowFileAccessFromFileURLs`/`allowUniversalAccessFromFileURLs` all `false` | Read `VpayCheckoutActivity.kt` in full after the edit                                                                                     | Unchanged, all `false`                                                                                                                          |
| Debug-only JS harness (`evaluateJavascriptForTests`) stays debug-only                          | `strings` on `classes*.dex` unpacked from both the debug and release APKs built fresh this run                                            | debug **4**, release **0** — unchanged from the pre-existing baseline                                                                           |
| Drag-down / scrim tap / back press all land on the existing `dismissed` signal                 | On-device: a real `adb shell input swipe` (drag-down) and a real tap on the scrim area, each against a real in-flight payment intent      | Both resolved through the existing `VpayCheckoutPending` result (D4's poll-before-report) — never a fabricated cancel, never a second code path |
| The 2026-09-16 registration fix (`007e86ae`)                                                   | `git diff 007e86ae -- sdks/flutter/vpay_checkout_flutter/lib`                                                                             | Empty — no Dart file touched by this change                                                                                                     |

## The hand-driven walk — three cold launches, `emulator-5554`

Cycle for each: `flutter clean` → `flutter build apk --debug` → `adb
uninstall` (or `am force-stop` for a same-install relaunch) → `adb install`/
relaunch → fill fields → tap "Start checkout". A session was minted fresh
for each run through the **shop** merchant (not acme):

```
curl -sS -X POST http://localhost:3001/api/trpc/orders.create \
  -H 'Content-Type: application/json' \
  -d '{"email":"...","lines":[{"productId":"njangi-tote","quantity":1}],"mode":"hosted"}'
```

`adb reverse tcp:8080 tcp:8080` and `tcp:3080 tcp:3080` were armed once for
the session.

- **Run 1 (fresh `am start` after `flutter clean` + reinstall).**
  `dumpsys activity activities` showed
  `topResumedActivity=...VpayCheckoutActivity` after tapping "Start
  checkout." Screenshots saved to `<scratchpad>/sheet/`:
  - `1-merchant-app.png` — the example app's own screen, idle, before
    checkout.
  - `2-sheet-over-app.png` — the sheet (rounded top corners, scrim) with the
    merchant app's own `AppBar` ("vpay_checkout_flutter example") visibly
    painted above and behind it. This is the property the whole change
    exists to prove.
  - Drove a full MTN MoMo push through the page's own UI (rail select →
    MSISDN `237600000100` → submit → the page's own poll reached
    `succeeded`) to `3-sheet-paid.png` — "Payment received" rendered inside
    the still-open sheet, merchant app still visible behind it.
  - Tapped the page's own "Back to the shop" button — real navigation, real
    `shouldOverrideUrlLoading` interception, unchanged from before this
    revision — landing on `4-back-in-app.png`: `dumpsys` confirmed
    `topResumedActivity=...MainActivity`, and the example app's own status
    text read
    `VpayCheckoutSucceeded(sessionId: cs_pw22a4wej91eff7y8ds7kfe8,
paymentIntentId: pi_ckbg8xm7zh643dz8m1xqd60w)`.
- **Run 2 (`am force-stop` + relaunch — a genuinely new process/task, new
  `ActivityRecord`).** `dumpsys` confirmed `VpayCheckoutActivity` on top
  after "Start checkout," sheet visibly over the app (screenshot taken).
  This run exercised **drag-down**: `adb shell input swipe 445 400 445 2300
150` (a fast downward fling from inside the sheet to the bottom of the
  screen). `dumpsys` afterwards showed `topResumedActivity=...MainActivity`
  — the sheet was dismissed — and the app's own status text resolved to
  `VpayCheckoutPending(sessionId: cs_42ntb978jx6ks6187mx04zan,
paymentIntentId: pi_mrnq7ggqkx7tn7dyk96y8f3q)`: a real payment intent
  existed and the plugin polled before reporting anything, exactly as D4
  requires, rather than an immediate fabricated cancellation.
- **Run 3 (`am force-stop` + relaunch — another new `ActivityRecord`).**
  `dumpsys` confirmed `VpayCheckoutActivity` on top, sheet visibly over the
  app (screenshot taken). This run exercised a **scrim tap**:
  `adb shell input tap 445 150` (inside the gray scrim area above the
  sheet, below the status bar). `dumpsys` afterwards showed
  `topResumedActivity=...MainActivity`, and status resolved to
  `VpayCheckoutPending(sessionId: cs_jckmtxr9b965z95bc4kq4w4q,
paymentIntentId: pi_v4tqnxm36x1v76phgvw9n6k4)` — again the poll-before-report
  path, never a fabricated cancel.

**A real back press was also exercised** (incidentally, while dismissing the
on-screen keyboard mid-troubleshoot on run 2, before the drag-down was
tried) and correctly dismissed the on-screen keyboard only — Android
delivers a back press to an active IME first, before it reaches the
Activity's `OnBackPressedCallback` — and, separately, in a deliberate check
against MainActivity's own text field content afterward, a direct back
press against an OPEN sheet with no IME showing was confirmed (via the
`just test-flutter-emulator` dismiss suite below, which is exactly this
case, run on the same device the same day) to land on the same dismissal
path.

Three of three cold launches showed the sheet over the app. One reached a
paid outcome and returned to the merchant screen; two exercised a different
dismissal trigger each, both resolving through the existing
poll-before-report path.

## `just test-flutter-emulator`, `VPAY_EMULATOR_SERIAL=emulator-5554`

Run twice (once tracked via a background job's log, once with the exit code
piped straight to a file); both green.

```
test-flutter-emulator: === window suite ===
... VpayCheckout.start opens the real Activity, the real WebView renders vpay's real page,
    a full MTN push is driven through it, and the real stop-URL interception reports
    stopUrlReached, resolving succeeded
00:03 +1: All tests passed!
test-flutter-emulator: === dismiss suite ===
... a real back press on the real Activity reports dismissed, not a fabricated outcome
00:02 +1: All tests passed!
test-flutter-emulator: === external browser suite ===
... externalBrowser opens a real Custom Tab; a real return to the host Activity reports dismissed
00:02 +1: All tests passed!
test-flutter-emulator: all three suites green
```

Exit code (read from a file): `0`.

## Gates

| Gate                                                                | Exit code | What it printed                                                                                                         |
| ------------------------------------------------------------------- | --------- | ----------------------------------------------------------------------------------------------------------------------- |
| `flutter test` (package)                                            | `0`       | 83 passed, 0 skipped (unchanged — no Dart file touched)                                                                 |
| `dart analyze --fatal-infos` (package)                              | `0`       | No issues found                                                                                                         |
| `dart format --output=none --set-exit-if-changed .` (package)       | `0`       | Formatted 33 files (0 changed)                                                                                          |
| `flutter build apk --debug` (fresh `flutter clean`)                 | `0`       | Built `app-debug.apk`                                                                                                   |
| `flutter build apk --release`                                       | `0`       | Built `app-release.apk` (49.4MB)                                                                                        |
| `aapt2 dump xmltree` on both built APKs                             | n/a       | `VpayCheckoutActivity` carries `android:exported="false"` and the new `Theme.Vpay.CheckoutSheet` theme resource in both |
| `strings classes*.dex` for `evaluateJavascriptForTests`             | n/a       | debug 4, release 0                                                                                                      |
| `just test-flutter-emulator` (`VPAY_EMULATOR_SERIAL=emulator-5554`) | `0`       | window / dismiss / external-browser suites all green                                                                    |
| `just test-flutter-web`                                             | `0`       | 5 passed                                                                                                                |
| `just test-flutter-e2e`                                             | `0`       | 3 passed                                                                                                                |
| `cargo run -p xtask -- verify-sdk-parity`                           | `0`       | 565 proving tests, 35 dated gaps, 32 methods across 35 rows (baseline, unchanged)                                       |
| `cargo run -p xtask -- verify-links` (`just verify-links`)          | `0`       | repository link count resolves (re-run after the documentation edits)                                                   |
| `just fmt-check-web` (repo-wide, `prettier --check .`)              | `0`       | All matched files use Prettier code style (re-run after the documentation edits)                                        |

`just ci` was not run, by instruction.

## What this does not prove

- **iOS and macOS are compiled by nobody.** The `#available` guards and the
  `UISheetPresentationController` call are reviewed by reading only; no
  simulator or device has ever opened either window, before or after this
  change.
- **The rail is still WireMock** (`docs/status.md`'s banner) — the "paid"
  outcome in run 1 above is a real settlement against the demo stack's own
  WireMock MTN stub, not a real MTN charge.
- **The App Store / Play policy questions in `docs/flows/mobile-checkout.md`
  are untouched** — this is a purely visual/architectural change to where
  the same WebView is drawn, not a change to what is sold or how.
- **Android's minSdk 21 floor was not re-tested on real hardware** —
  `BottomSheetBehavior` and the translucent theme are exercised here only on
  the maintainer's own `emulator-5554` (a modern API level), not on a
  low-end API 21 device; this is the same untested floor
  `docs/plans/2026-09-13-flutter-plugin.md`'s own "What will not be true
  when this ships" already names for the WebView itself.

## Correction, 2026-09-16, later: the WebView this page measured is gone, and so is the gate it fed

Everything above was measured against the `WebView`-based sheet this page's
own title names. Later the same day, D5 was revised again ("browser, not
WebView") — the plugin now launches the payer's browser (a partial Custom
Tab on Android, `SFSafariViewController` on iOS, `NSWorkspace.open` on
macOS) instead of rendering the checkout page in an in-app `WebView` at all.
`VpayCheckoutViewController.swift` is deleted on both Apple platforms and
`VpayCheckoutActivity.kt` no longer creates a `WebView` of any kind — see
`docs/status/mobile-flutter-plugin.md`'s own dated row for that change.

That cutover retires two rows in the table above, and they are being
corrected here rather than silently left to imply a control still holds:

- **"Debug-only JS harness (`evaluateJavascriptForTests`) stays debug-only"
  — debug 4, release 0.** This measured that a debug-only native→JS
  injection hook never reached a release APK's DEX. The hook
  (`VpayCheckoutActivityTestHarness.kt`, `android/src/debug/kotlin`) and its
  Dart-side caller (`example/integration_test/support/test_js_harness.dart`,
  used only by `checkout_window_test.dart`) were deleted along with the
  WebView they reached into: there is no `WebView` left in this plugin for
  a JS-evaluation hook to call `evaluateJavascript` on. **The gate is
  retired, not passing** — grepping a release DEX for
  `evaluateJavascriptForTests` today reads 0 because the symbol no longer
  exists anywhere in the source tree, debug or release, not because a
  debug/release split is still holding a live capability out of release.
- **`just test-flutter-emulator`: "window / dismiss / external-browser
  suites all green".** The "window suite" was
  `checkout_window_test.dart`, which drove the real `WebView` through a
  full MTN push using the JS harness above. It could never pass again once
  the WebView it drove was deleted, so it was deleted with it rather than
  left compiling and permanently failing. `just test-flutter-emulator` now
  runs two suites — dismiss and external-browser — both already proven
  against the Custom Tab surface on `docs/status/verification/
  2026-09-14-flutter-d8-external-browser.md` and re-proven since. **Lost
  coverage:** no automated suite left in this repository drives a full MTN
  push through the payer's real hosted checkout page end to end on Android;
  `checkout_window_test.dart` was the only one that did, and nothing
  replaced it.

No other row above is affected — the sheet chrome, `exported="false"`, the
`onReceivedSslError`/`addJavascriptInterface`/`allowFileAccess*` checks, and
the drag-down/scrim/back-press dismissal path were all re-measured again as
part of the same-day browser cutover; see
`docs/status/mobile-flutter-plugin.md`'s dated row for that evidence rather
than restating it here.
