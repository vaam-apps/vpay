# Mobile Flutter plugin (`vpay_checkout_flutter`)

New page, 2026-09-13. Not a merchant SDK — a payer surface, like
`@vaam-apps/vpay-stripe-js` — so it is not folded into
[merchant-sdks.md](merchant-sdks.md). See
[docs/flows/mobile-checkout.md](../flows/mobile-checkout.md) for the process
and [ADR-0021](../adr/0021-flutter-checkout-plugin.md) for the decisions.

## What is real today

Built by Lane B of
[`docs/plans/2026-09-13-flutter-plugin-brief.md`](../plans/2026-09-13-flutter-plugin-brief.md),
on `claude/flutter-lane-b-gate`:

| Piece                                                       | State                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ----------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo xtask verify-sdk-parity` reads Dart                  | ✅ `dart_test_names` collects `test('…')`/`testWidgets('…')`/`group('…')` titles, single- and double-quoted, with escapes and raw (`r'…'`) strings; `PARITY_SKIPPED_DIRS` grew `.dart_tool` and `build`. Proven against a synthetic fixture tree in `.xtask/src/main.rs`'s `sdk_parity_tests` module — **not** against this repository's own `sdks/`, which has no Dart column yet.                                                                                                                                                                                               |
| The skip property, the one this gate exists for             | ✅ `test('x', skip: true)` and a string `skip:` reason are **not** collected, mirroring `rust_test_names`' drop of `#[ignore]`d tests and `ts_test_names`' drop of `it.skip(…)`. Proven by mutation: `dart_decisive_mutation_a_live_test_passes_verify_sdk_parity` calls `verify_sdk_parity` on a synthetic tree and gets `Ok(())`; the same fixture with `skip: true` added (`dart_decisive_mutation_skip_true_fails_verify_sdk_parity_naming_the_cell`) gets `Err` naming both the cell and the SDK column. See the dated verification page below for the literal before/after. |
| `just install-flutter` / `analyze-flutter` / `test-flutter` | ✅ exist, each refuses with a named, human-readable reason — not a stack trace — when `sdks/flutter/vpay_checkout_flutter/` does not exist (true on this branch) or when the `flutter` SDK is not on `PATH`. **Not in `just ci` or `just verify`** (D-M3): confirmed by reading both recipe lists; neither names any of the three.                                                                                                                                                                                                                                                |
| `flutter-toolchain.toml`                                    | ✅ pins Flutter `3.47.2` / Dart `3.13.2` — the version installed on the host this lane was authored and verified against, not a floor computed from a `pubspec.yaml` (there is none yet). The file's own comment records that the host reports channel `[user-branch]`, not a clean channel pin the way `rust-toolchain.toml`'s `channel` is, and says the maintainer may want to fix that before this ships in an image.                                                                                                                                                         |
| ADR-0021                                                    | ✅ [docs/adr/0021-flutter-checkout-plugin.md](../adr/0021-flutter-checkout-plugin.md) — records D1–D9 and D-M1–D-M6 as accepted.                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `docs/flows/mobile-checkout.md`                             | ✅ the flow doc, carrying D9's Apple 3.1.3(e) and 3.1.1 quotes verbatim (not paraphrased) as a constraint on adoption, and naming — rather than deciding — the one open documentation-structure question against `hosted-checkout/page-memory-and-protocols.md`'s popup table.                                                                                                                                                                                                                                                                                                    |

## What Lanes A and C added, and the 2026-09-14 review

Lane B's table above described the gate and the docs. The plugin itself
landed afterwards, and until 2026-09-14 this page still said "No Dart file
exists in this repository" — true when Lane B wrote it, false from Lane A's
merge onward. Corrected here by the review.

| Piece                                   | State                                                                                                                                                                                                                                                                                                 |
| --------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The Dart core (`lib/`)                  | ✅ browser client, pure state machine, result/error types, redaction, the pigeon seam. `flutter test` green; counts and skips on the dated verification page.                                                                                                                                         |
| The Android host                        | ✅ exists and **compiles**: `flutter build apk --debug` on `example/`, and the merged manifest carries `VpayCheckoutActivity` with `android:exported="false"`. `onReceivedSslError` is not overridden and there is no `addJavascriptInterface` call. No device, no emulator, no instrumentation test. |
| The web host                            | ✅ exists and **compiles**: `flutter build web` on `example/`, and Flutter's generated `web_plugin_registrant.dart` calls `WebVpayCheckoutPlatform.registerWith`. No browser has driven the popup.                                                                                                    |
| iOS and macOS hosts                     | ⛔ the Swift exists and is **compiled by nobody** — Linux host, no `xcodebuild`, reviewed by reading only. Since D8 (below) it also carries `VpayCheckoutExternalBrowserSession`, unaffected by this row.                                                                                             |
| `VpayCheckoutMode.externalBrowser` (D8) | ✅ **closed by D8, 2026-09-14** — see the D8 section below. No longer `⛔`.                                                                                                                                                                                                                           |
| The parity table                        | ✅ `docs/sdks/parity.md`'s third table, every Flutter ✅ cell naming a Dart test that actually ran.                                                                                                                                                                                                   |

### What the review found and fixed

Each was measured by mutation; the before/after exit codes are on the dated
page below.

1. **`externalBrowser` was silently ignored** — the public API accepted a
   mode it did not have and handed back the in-app WebView.
2. **The pigeon-generated `toString` rendered the session secret.**
   `ShowCheckoutRequest.url`'s fragment _is_ the session `client_secret`, and
   the generated Dart, Kotlin and Swift all interpolated it. The design doc
   predicted this by name ("generated code is how this regresses") and the
   D6 parity row was ✅ with no test over that file.
3. **`no_logging_test.dart` excluded member calls**, so
   `developer.log(secret)` in `lib/` passed it.
4. **The gate's Dart reader collected tests that never run** — a skipped
   group's tests, commented-out declarations, and titles quoted in strings.
5. **The window's one event could be dropped**, hanging `start()` forever,
   because the broadcast stream was subscribed only after `show()` returned.
6. **A malformed 200 escaped as a bare `TypeError`** out of a poll, against
   this package's own stated contract.
7. **`allowInsecureUrl` was hard-coded `false`** and never reached the host.

## Lane D — the Dart core against a real, running vpay (2026-09-14)

Until this lane, every server in this package's test suite was
`package:http/testing.dart`'s `MockClient`. That is no longer true for
`test_e2e/real_stack_e2e_test.dart`, run only by `just test-flutter-e2e`,
never by plain `flutter test` (which stays `MockClient`-only, stack-
independent, and still 80 passed / 0 skipped).

| Piece                                            | State                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A real Checkout Session, minted for real         | ✅ the recipe mints two, through `examples/shop`'s real server — a real `POST /v1/payment_intents` + `POST /v1/checkout/sessions`, authenticated with a real `private_key_jwt` `client_credentials` exchange (the same two calls `examples/shop/src/server/orders.ts` makes for a paying customer) — never a credential this package itself holds.                                                                                                                         |
| `BrowserClient`/`CheckoutController` end to end  | ✅ `mints, preflights, confirms and polls a real session to succeeded` — a real session read, a real pre-flight, a real confirm (`POST /v1/browser/payment_intents/{id}/confirm`, standing in for what vpay's own hosted page submits — this package has no `confirm` method by design), a real poll through the package's own code to a real terminal outcome, re-checked twice more independently (once more through `BrowserClient`, once with no package code at all). |
| The uniform 404                                  | ✅ `an unknown id, a wrong secret and a wrong key all answer the same 404` — against the real server, not a stub.                                                                                                                                                                                                                                                                                                                                                          |
| A session that is not `open` refuses the confirm | ✅ `the intent read still answers, the pre-flight fails closed, and the confirm is refused with checkout_session_expired` — the recipe expires a real session with a real merchant access token (minted the same `private_key_jwt` way `examples/merchant-demo` does, off whichever key the running shop container already has), then proves the intent read still works, the pre-flight fails closed, and the confirm answers the real `409 checkout_session_expired`.    |
| A real bug this found                            | ✅ fixed the same day — `CheckoutSession.fromJson` required a `client_secret` field the real `GET /v1/browser/checkout/sessions/{id}` never sends back (it only ever renders the intent's). Every `MockClient` fixture in `test/` had been fabricating that field, so `flutter test` stayed green while every real pre-flight failed with `unexpected_response(200)`.                                                                                                      |
| The decisive test — a stack that is down         | ✅ `just demo_port=<nothing listening> test-flutter-e2e` fails LOUDLY (exit 1) before any Flutter process runs, at the `/healthz` check. A green run with nothing listening was measured to be possible before this check existed and is exactly what this recipe refuses.                                                                                                                                                                                                 |

**Still true, narrower than before:** the rail behind that real vpay is
WireMock, exactly as it is everywhere else in this repository
(`docs/status.md`'s banner). `docs/sdks/parity.md` now carries this as two
rows rather than one, for exactly this reason.

## D8 — the external-browser mode, wired end to end (2026-09-14)

Until this lane, `VpayCheckoutMode.externalBrowser` was designed and unwired:
`pigeons/checkout.dart`'s `ShowCheckoutRequest` carried no `mode` field, so
`VpayCheckout.start` refused it with `UnimplementedError` before the
pre-flight ever ran. That special case is gone.

| Piece                                        | State                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The pigeon seam                              | ✅ `pigeons/checkout.dart`'s `CheckoutWindowMode` (`inApp`/`externalBrowser`) and `ShowCheckoutRequest.mode`, regenerated for Dart, Kotlin and Swift; the D6 redaction hand-edit was re-applied in all four generated files after regeneration wiped it, and re-verified by `test/messages_redaction_test.dart`. Counts: Dart 4, Kotlin 4, iOS Swift 3, macOS Swift 3 — unchanged from the pre-D8 baseline.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Dart threading                               | ✅ `VpayCheckoutPlatform.show` gains a required `mode`; `VpayCheckout.start` always threads it through rather than special-casing `externalBrowser` — `is threaded to the platform host as CheckoutWindowMode.externalBrowser`, `the default mode is inApp, threaded to the platform host as CheckoutWindowMode.inApp`, `with no platform host at all, still throws UnimplementedError (not special-cased any more — the seam itself has none)`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Android — Custom Tabs                        | ✅ **compiled and run for real.** `VpayCheckoutExternalBrowserSession` launches a `CustomTabsIntent` (`androidx.browser:browser:1.8.0`) and reports a dismissal on the host `Activity`'s own resume (`Application.ActivityLifecycleCallbacks` — no `startActivityForResult`, no scheme). `flutter build apk --debug`/`--release` on `example/` both exit 0; DEX string count for the debug-only JS test hook unchanged (debug 4, release 0). A dedicated AVD (`vpay_d8_avd`, created and deleted for this run) drove `checkout_external_browser_test.dart`: a real `CustomTabsIntent` opened Chrome (`com.android.chrome` as the resumed activity, confirmed present on this image), a real back press returned control to the host `Activity`, and a real `dismissed` event arrived over the real channel. Run twice, both exit 0. `checkout_dismiss_test.dart` (in-app) re-run on the same device to confirm no regression: exit 0. |
| Web                                          | ✅ `WebVpayCheckoutPlatform.show` accepts `mode` (never dropped from the signature) and documents that `inApp`/`externalBrowser` collapse to the identical `window.open` popup — there is no in-app WebView on Flutter web to distinguish them.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| iOS — `SFSafariViewController`               | ✅ wired, **compiled by nobody**. Not `ASWebAuthenticationSession` — no scheme to call back on below 17.4, and its "sign in" consent alert is the wrong sentence on a payment (design doc D8). `SFSafariViewController` has no navigation delegate, so the only signal is the payer tapping Done, reported as `dismissed`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| macOS — `NSWorkspace`                        | ✅ wired, **compiled by nobody**, maintainer's own call (recorded in `VpayCheckoutExternalBrowserSession.swift`'s header): no `SFSafariViewController` or Custom-Tabs equivalent exists on macOS, so this opens the payer's default browser and watches for this app's own reactivation (`NSApplication.didBecomeActiveNotification`), mirroring Android's own tier-0 shape.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| D8's tier 1 (App Links / Associated Domains) | ⛔ 2026-09-14 — not implemented on any platform. Needs a merchant-hosted `assetlinks.json`/`apple-app-site-association` deployment this repository cannot provide or prove against; D8 itself says a merchant may stop at tier 0, which is what ships.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |

Evidence:
[verification/2026-09-14-flutter-d8-external-browser.md](verification/2026-09-14-flutter-d8-external-browser.md).

## A real installed app never opened the window at all, until 2026-09-16

Every ✅ above for the Android/web window was earned by `example/lib/main.dart`
compiling and by `example/integration_test/*.dart` suites that called a
`support/ensure_platform_registered.dart` helper to register the platform
host **by hand** — the one thing a real app never does. A hand-driven walk
on a real installed APK (`flutter clean` → `flutter build apk --debug` →
`adb uninstall` → `adb install` → launch → fill fields → tap "Start
checkout") threw `UnimplementedError: ... VpayCheckoutPlatform.windowEvents
has no platform host yet` and never opened `VpayCheckoutActivity`. Root
cause: the Flutter engine calls `pubspec.yaml`'s `dartPluginClass`-generated
plugin registrant **before** the app's own `main()`/
`WidgetsFlutterBinding.ensureInitialized()`/`runApp()` runs, and
`MethodChannelVpayCheckoutPlatform`'s constructor used to call
`VpayCheckoutFlutterApi.setUp(this)` eagerly, which touches
`ServicesBinding.instance` immediately — with no binding yet, that threw
`Binding has not yet been initialized`, silently swallowed by the engine's
own generated wrapper.

**A first fix attempt the same day — deferring `VpayCheckoutFlutterApi
.setUp` out of the constructor into the first call to `show` — was written
up as done in this section and in the dated verification page below, but
was never actually applied to `method_channel_checkout_platform.dart`; a
second pass the same day found the constructor still calling it eagerly and
the real device still failing exactly as before.** Repeated cold launches
of the same APK, read through `adb logcat`, then showed the race is real
but genuinely intermittent: `_PluginRegistrant.register()` sometimes runs
before `WidgetsFlutterBinding.ensureInitialized()` and sometimes after, so a
single successful hand-driven walk proves nothing on its own. The fix that
actually landed has two parts. First, the constructor now catches the
`FlutterError` `VpayCheckoutFlutterApi.setUp` throws with no binding yet,
and `show`/`dismiss`/`windowEvents` each retry it — every one of those only
ever runs after a merchant's own `main()` has, when a binding always
exists. Second — load-bearing, since the first part alone still depends on
`dartPluginClass` eventually winning the race — `checkout_platform.dart`'s
`VpayCheckoutPlatform.instance` getter is now platform-aware on its own,
via a `dart:io`-vs-web conditional import
(`native_mobile_host_stub.dart`/`native_mobile_host_io.dart`): the first
read that finds `UnimplementedVpayCheckoutPlatform` on Android, iOS or
macOS resolves `MethodChannelVpayCheckoutPlatform` lazily, at a point
guaranteed to be after the app's own `main()` has run. A
`defaultTargetPlatform`/`kIsWeb` check was tried first for that and
rejected: `flutter test` forces `defaultTargetPlatform` to `android`
whenever `FLUTTER_TEST` is set, which would have silently defeated
`test/vpay_checkout_test.dart`'s own "no platform host exists yet" case.
`dartPluginClass` registration winning the race is now purely an
optimisation, never a requirement. `support/ensure_platform_registered.dart`
stays deleted; all three `example/integration_test/*.dart` suites rely on
the same automatic registration a real app depends on, and `just
test-flutter-emulator` (`VPAY_EMULATOR_SERIAL=emulator-5554`, the
maintainer's own device) is still exit 0. The regression test,
`test/method_channel_checkout_platform_registration_test.dart`, is
unchanged and reproduces the pre-`main()` state; it still fails with
exactly `Binding has not yet been initialized` against the pre-fix
constructor (exit 1) and passes against the actual fix (exit 0) — both
re-measured.

Evidence:
[verification/2026-09-16-flutter-real-app-registration.md](verification/2026-09-16-flutter-real-app-registration.md).

## Modal checkout sheet, not a full-screen window (2026-09-16)

Requested by the maintainer, verbatim: the full-screen Android/iOS window
"feels like the user is quitting the app." This revises design D5 —
[`../plans/2026-09-13-flutter-plugin.md`](../plans/2026-09-13-flutter-plugin.md)'s
own "D5, revised 2026-09-16" section carries the full reasoning and every
alternative considered; this page records what was proven.

| Piece                                                              | State                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Android — same Activity, translucent theme                         | ✅ `VpayCheckoutActivity` is unchanged as a class of window — `android:exported="false"` untouched — only `AndroidManifest.xml`'s theme moved from `Theme.NoTitleBar.Fullscreen` to a translucent `Theme.Vpay.CheckoutSheet` (`res/values/styles.xml`), and `onCreate` now builds a scrim + `CoordinatorLayout`/Material `BottomSheetBehavior` sheet around the `WebView` instead of a bare full-bleed one. Deliberately NOT a `BottomSheetDialogFragment` hosted by the merchant's own Activity — that would dissolve the Activity isolation `exported=false` rests on for a cosmetically identical result.                                                                                         |
| The detent                                                         | ✅ `halfExpandedRatio = 0.9f` with `isFitToContents = false` — the maintainer's explicit "~90% of screen" decision — draggable further to `STATE_EXPANDED` (full height); `skipCollapsed = true` so a drag past the detent goes straight to hidden, never a small peek state.                                                                                                                                                                                                                                                                                                                                                                                                                        |
| One dismissal signal, three triggers                               | ✅ back press, a scrim tap, and a drag past the detent all set `BottomSheetBehavior.state = STATE_HIDDEN`; the sheet's own `BottomSheetCallback.onStateChanged` is the ONE place that then calls the existing `finishAsDismissed()` — no second, separate "cancel" path (design D4). Proven on-device, not just read: a real `adb shell input swipe` (drag-down) and a real tap on the scrim area both resolved through the existing `VpayCheckoutPending` result (D4's poll-before-report) with a real payment intent mid-flight, exactly as a real hardware back press already did.                                                                                                                |
| Non-negotiables, re-measured on the new theme                      | ✅ none assumed unchanged. Fresh `flutter clean` → `flutter build apk --debug`/`--release`, both exit 0. The merged manifest inside BOTH built APKs — read with `aapt2 dump xmltree` against the actual APK file, not the source `AndroidManifest.xml` — still carries `VpayCheckoutActivity` with `android:exported="false"`. DEX string count for `evaluateJavascriptForTests`: debug 4, release 0 — unchanged. `onReceivedSslError` still not overridden; still no `addJavascriptInterface`; `allowFileAccess`/`allowFileAccessFromFileURLs`/`allowUniversalAccessFromFileURLs` still all `false` (read, since none of these has a dedicated test).                                               |
| The hand-driven walk, three cold launches                          | ✅ on the maintainer's own `emulator-5554` (`flutter clean` → `flutter build apk --debug` → `adb uninstall` → `adb install` → launch → fill fields → "Start checkout"), all three showing `VpayCheckoutActivity` on top of `dumpsys activity activities` with the sheet visibly over the merchant app's own screen. Run 1 was driven all the way through a real MTN MoMo push (steering MSISDN `237600000100`) to `VpayCheckoutSucceeded` and back to the merchant screen through the page's own forward button — the existing `stopUrlReached` path, unchanged. Runs 2 and 3 each independently exercised a different dismissal trigger (drag-down; a scrim tap) instead of completing the payment. |
| `just test-flutter-emulator`, `VPAY_EMULATOR_SERIAL=emulator-5554` | ✅ exit 0, all three suites green (window, dismiss, external-browser) — the dismiss suite's real hardware back press still reports a real dismissal through the sheet exactly as it did through the old full-screen window.                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| iOS — `UISheetPresentationController` on 15+                       | ✅ **compiled by nobody**, reviewed by reading only. `.pageSheet` (13/14, unchanged) already rendered as a card with the app visible behind it; `viewDidLoad` now sets an explicit `.large()` detent via `UISheetPresentationController` on 15+, matching the maintainer's "draggable to full height" decision as an actual API call rather than `.pageSheet`'s own implicit default. iOS 12 (D-M4 floor) has no non-full-screen modal presentation API at all and necessarily stays `.fullScreen` — a stated consequence of supporting that floor, not an oversight.                                                                                                                                |
| macOS                                                              | ⛔ unaffected on purpose — `VpayCheckoutViewController` there already presents `as a sheet` (that file's own header, unchanged); the maintainer's complaint was about the full-screen Android/iOS shape, not macOS's existing sheet.                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Web                                                                | ⛔ unaffected on purpose — the web host has never been a native window (`window.open` popup, surrounded by the browser's own chrome); there is no full-screen takeover to fix.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |

Screenshots (not committed to the repository, held by the agent that ran the
walk): the merchant app before checkout, the sheet with the merchant app
visibly behind it (the money shot), the paid outcome rendered inside the
sheet, and the result back on the merchant's own screen.

Evidence:
[verification/2026-09-16-flutter-bottom-sheet.md](verification/2026-09-16-flutter-bottom-sheet.md).

## What is still not real

- **No `just ci` gate** (D-M3). `install-flutter`/`analyze-flutter`/
  `test-flutter`/`test-flutter-e2e` exist; none is in `just ci` or
  `just verify`, and `docs/status.md`'s gate table does not claim otherwise.
  Every count this repository quotes for this package is a human running it
  by hand.
- **No iOS or macOS compile.** Not "not yet run" — there is no toolchain on
  this host and there cannot be. Confirmed again by D8: `swiftc`/`swift` are
  both absent from this host too.
- **No browser** for web's `inApp` popup — proven by compiling
  (`flutter build web`) and, separately, by `just test-flutter-web` running
  `web_checkout_platform_test.dart` in a real Chrome (2026-09-15), but no
  human or automated walk has ever opened the popup end to end against the
  real hosted page. **Android's `inApp` window is the exception as of
  2026-09-16**: corrected below and in the "A real installed app never
  opened the window" and "Modal checkout sheet" sections — it has been
  opened for real, repeatedly, on the maintainer's own `emulator-5554`.
  `externalBrowser` was already the exception before that: Android's Custom
  Tabs path was run for real on a headless emulator (D8, above).
- **D8's tier 1** (Android App Links / iOS 17.4+ Associated Domains) is not
  implemented on any platform — see the D8 section above.
- **No real rail.** `just test-flutter-e2e` (Lane D, above) proved the
  running-vpay half; the rail behind that stack is still WireMock.
- **No App Store or Play review.** ADR-0021 and D9 read the published rules;
  a reviewer's verdict is a different thing this repository will not have.
- **The Android 21 / iOS 12 floor is a claim nobody will test.**
- **No desktop Linux or Windows.**

## Verification

- [verification/2026-09-13-flutter-lane-b-gate.md](verification/2026-09-13-flutter-lane-b-gate.md)
  — Lane B's `cargo test -p xtask`, `cargo clippy --all-targets`,
  `just verify-links` and `just verify-sdk-parity` runs, and the literal
  before/after of the decisive skip mutation.
- [verification/2026-09-14-flutter-review.md](verification/2026-09-14-flutter-review.md)
  — the review's gate output on the merged head, and every mutation it
  measured, with the exit code read from a file in each case.
- [verification/2026-09-14-flutter-e2e-real-stack.md](verification/2026-09-14-flutter-e2e-real-stack.md)
  — Lane D's real-stack run: `just test-flutter-e2e` green with real `cs_…`/
  `pi_…` ids, the same recipe failing loudly against a down stack, and
  `just test-flutter` still 80 passed / 0 skipped throughout.
- [verification/2026-09-14-flutter-d8-external-browser.md](verification/2026-09-14-flutter-d8-external-browser.md)
  — D8's gate output: `flutter test` (82/0), `dart analyze`/`dart format`,
  the four redaction counts after regeneration, both APK builds and DEX
  counts, and the real headless-emulator run of `VpayCheckoutMode
.externalBrowser`, exit codes read from files throughout.
- [verification/2026-09-16-flutter-real-app-registration.md](verification/2026-09-16-flutter-real-app-registration.md)
  — the real-installed-APK defect, its root cause, the fix, the hand-driven
  walk's `adb dumpsys`/screenshot evidence, the decisive regression test's
  both exit codes, and every "do not break" gate rerun on the fix.
- [verification/2026-09-16-flutter-bottom-sheet.md](verification/2026-09-16-flutter-bottom-sheet.md)
  — the modal-sheet revision: the merged-manifest and DEX re-measurements on
  the new theme, the three-cold-launch hand-driven walk's `dumpsys` evidence
  and dismissal-trigger results, `just test-flutter-emulator`'s exit code,
  and every "do not break" gate rerun on the change.
