# The in-app WebView is gone — every platform opens the payer's own browser

**Date:** 2026-09-16, later the same day as the modal-bottom-sheet revision
(`2026-09-16-flutter-bottom-sheet.md`). **Branch:** `pr-181-ios`, HEAD
`1fd7c46` (merges `master` at `7a79684`). **Host for this docs pass:**
macOS 26.6.2, Xcode 26.6, Android SDK 37.0.0/build-tools 37.0.0, Flutter
3.48.0-1.0.pre-696 / Dart 3.14.0 — **not** the pinned `flutter-toolchain.toml`
version (3.47.2 / 3.13.2); noted because a version drift is exactly the kind
of thing this repository's own rules ask not to gloss over, not because it
changed any result below.

This page has two kinds of evidence in it. Re-measured here, on this host,
in this pass: `flutter test`, `dart analyze`, both Android APK builds, and
the merged-manifest/DEX reads. Carried over from the change itself, not
re-run here: the iOS Simulator walk, which this host's booted
`iPhone 17 Pro` simulator (confirmed booted via `xcrun simctl list devices`
during this pass) is consistent with but which this pass did not repeat
end to end. Each claim below says which.

## What changed

`git status --short sdks/flutter/vpay_checkout_flutter/` on this branch:

- **Deleted:** `android/src/debug/kotlin/.../VpayCheckoutActivityTestHarness.kt`,
  `android/src/main/kotlin/.../VpayCheckoutExternalBrowserSession.kt`
  (superseded by a same-named Swift-only concept — the Kotlin external-
  browser session is folded into `VpayCheckoutActivity`/the new Custom Tab
  path), `android/src/main/res/values/styles.xml`,
  `example/android/.../TestHarnessInitProvider.kt`,
  `example/integration_test/checkout_window_test.dart`,
  `example/integration_test/support/test_js_harness.dart`,
  `ios/.../VpayCheckoutViewController.swift`,
  `macos/.../VpayCheckoutViewController.swift`.
- **Added (untracked):** `android/src/main/kotlin/.../VpayCheckoutAppLinkActivity.kt`,
  `example/ios/` (a new iOS example project — this is what made the
  Simulator walk possible in the first place; there was no runnable iOS
  example app before this pass).
- **Modified:** `android/build.gradle.kts`, `android/src/main/AndroidManifest.xml`,
  `android/.../Messages.g.kt` (pigeon regeneration, `mode` field removed),
  `android/.../VpayCheckoutActivity.kt`, `android/.../VpayCheckoutFlutterPlugin.kt`,
  the iOS/macOS `Messages.g.swift`/`VpayCheckoutExternalBrowserSession.swift`/
  `VpayCheckoutFlutterPlugin.swift` (three files each, both platforms),
  `pigeons/checkout.dart` (the frozen contract itself — `mode` removed, the
  security reasoning added as a doc comment, `stopUrlReached`'s meaning
  redefined), `lib/src/platform/checkout_platform.dart`,
  `lib/src/platform/messages.g.dart`,
  `lib/src/platform/method_channel_checkout_platform.dart`,
  `lib/src/platform/web_checkout_platform.dart`, `lib/src/vpay_checkout.dart`,
  `lib/vpay_checkout_flutter.dart`, and the Dart test files
  (`test/messages_redaction_test.dart`, `test/method_channel_checkout_platform_test.dart`,
  `test/vpay_checkout_test.dart`, `test/web/web_checkout_platform_test.dart`,
  `example/integration_test/checkout_dismiss_test.dart`,
  `example/integration_test/checkout_external_browser_test.dart`,
  `example/integration_test/support/fixture.dart`, `example/lib/main.dart`).

`pigeons/checkout.dart`'s own doc comments now carry the security reasoning
verbatim — `VpayCheckoutHostApi.show`'s comment states why a `WebView` was
removed, and `CheckoutWindowOutcome.stopUrlReached`'s comment states that it
is unverified on every platform. This page does not restate that reasoning;
see the pigeon file itself, `docs/adr/0021-flutter-checkout-plugin.md`'s
matching dated section, or `docs/plans/2026-09-13-flutter-plugin.md`'s "D5,
revised 2026-09-16, later" section.

## Re-measured on this host, this pass

| Claim | How it was checked | Result |
| --- | --- | --- |
| `flutter test` (package) | `flutter pub get && flutter test` in `sdks/flutter/vpay_checkout_flutter/` | **80 passed, 0 skipped** — matches the figure this pass's own narrative claimed; independently reproduced here, not just trusted |
| `dart analyze --fatal-infos` (package) | Same directory | **No issues found** |
| `flutter build apk --debug` (`example/`, fresh) | `flutter build apk --debug` | **Exit 0** — `Built build/app/outputs/flutter-apk/app-debug.apk` |
| `flutter build apk --release` (`example/`) | `flutter build apk --release` | **Exit 0** — `Built .../app-release.apk (48.8MB)` |
| `VpayCheckoutActivity` still `android:exported="false"` | `aapt2 dump xmltree` against **both built APKs**, not the source manifest | `false` in both debug and release |
| `VpayCheckoutAppLinkActivity` exists, is exported, and ships no `<intent-filter>` of its own | Same `aapt2 dump xmltree` reads | Present in both APKs: `android:exported="true"`, `android:excludeFromRecents="true"`, `android:noHistory="true"`, and — read down through the whole element — **no nested `<intent-filter>`**, unlike `MainActivity`'s own entry a few lines above it, which does carry one. Confirms the plugin declares no filter of its own for the merchant to merge over. |
| `evaluateJavascriptForTests` is gone from both DEXes, not merely absent from release | `unzip classes*.dex` from both built APKs, then `strings classes*.dex \| grep -c evaluateJavascriptForTests` | **0 in debug, 0 in release** — before this cutover the figure was debug 4 / release 0 (a live capability kept out of release); now it is 0/0 because the symbol does not exist anywhere in the source tree, debug or release |

`dart format --output=none --set-exit-if-changed .` was also run: it
reports one file, `lib/src/platform/messages.g.dart` (pigeon-generated),
as not matching the formatter's current output. Recorded rather than
silently omitted; this is a formatting drift in a generated file, not a
behavioural claim, and this pass did not touch generated code (out of
scope for a docs-only change) — it is left for whoever next regenerates the
pigeon output to notice via `just fmt-check-web`.

## Carried over from the change itself, not re-run in this pass

**The iOS Simulator walk.** On a real `iPhone 17 Pro` simulator running
iOS 26.5, the app was built and run (not merely compiled): the
`SFSafariViewController` sheet presented with an explicit `.large()` detent
and rendered with Safari's own address bar; a payer tapped through the
hosted checkout page to `success_url`, and the sheet stayed open — there is
no navigation interception left on any platform, by design (see "what
changed" above). Tapping Done reported `dismissed` over the platform
channel; D4's poll ran; the checkout resolved `VpayCheckoutSucceeded`. This
page did not repeat that walk — re-running a full checkout flow through a
live Simulator UI is outside a docs-only pass's scope — but this host's
`iPhone 17 Pro` simulator was confirmed **booted** during this pass
(`xcrun simctl list devices`), consistent with, though not proof of, the
walk having been run here.

**Everything Android beyond compiling.** No emulator or device was booted,
installed to, or driven in this pass, and none was in the change this page
documents either. The Lane E/D8 emulator evidence on earlier dated pages in
this directory (`2026-09-14-flutter-d8-external-browser.md`,
`2026-09-16-flutter-bottom-sheet.md`) predates this cutover — it exercised a
`WebView`-based window and an opt-in `externalBrowser` Custom Tab mode,
neither of which exists any more — and does not carry forward as evidence
for the partial-bottom-sheet Custom Tab (`CustomTabsIntent
.setInitialActivityHeightPx`) or the `VpayCheckoutAppLinkActivity` deep-link
path this cutover introduces. **Android is compile-only as of this page.**

**macOS.** Not exercised, compiled, or reviewed beyond a source read in
this pass. `NSWorkspace.shared.open(url)` and the removal of the old
focus-regained dismissal signal are read from source, not run.

## What this does not prove

- **`CheckoutWindowOutcome.stopUrlReached` (an incoming deep link) is
  unverified on every platform.** A real Android App Link or iOS/macOS
  Universal Link needs an HTTPS origin serving
  `assetlinks.json`/`apple-app-site-association` for the merchant's own
  `success_url` host — this repository can neither deploy nor prove that.
  Every checkout observed anywhere in this repository's history, including
  the iOS Simulator walk above, has ended as `dismissed`, never
  `stopUrlReached`. D1/D4's poll is what makes that correctness-complete
  regardless of whether the deep link ever fires.
- **macOS has no dismissal signal at all**, verified or otherwise, unless a
  Universal Link arrives (itself unverified) or the merchant calls
  `dismiss()`. This is a stated consequence of removing the previous fake
  focus-regained signal, not a regression nobody noticed.
- **The rail is still WireMock** (`docs/status.md`'s repository-wide
  banner). Nothing in this cutover, or anywhere else in this repository, has
  been driven against a real MTN or Orange endpoint.
- **No suite drives a full MTN push through the real hosted checkout page
  end to end on Android any more.** `checkout_window_test.dart` was the
  only one that did, and it was deleted along with the `WebView` it drove —
  see `docs/status/mobile-flutter-plugin.md`'s "Browser, not WebView"
  section for the accounting.
- **No App Store or Play review.** `docs/flows/mobile-checkout.md`'s D9
  section and ADR-0021 read the published rules; a reviewer's verdict is a
  different thing this repository will not have. The digital-goods trap
  (Apple 3.1.1(a)) now applies to the plugin's *only* surface rather than to
  one of two modes, which changes nothing about the analysis but means the
  plugin's README can no longer scope the warning to `externalBrowser`
  alone — it applies unconditionally.
- **The Android 21 / iOS 12 floor remains a claim nobody has tested on
  real hardware**, unchanged by this cutover.
