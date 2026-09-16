# `vpay_checkout_flutter` never opened a window on a real installed app

**Date:** 2026-09-16. **Branch:** `claude/flutter-web-e2e`, base `f3fc12d7`.
**Host:** Linux, Flutter 3.47.2 / Dart 3.13.2, Rust 1.98.0
(`rust-toolchain.toml`). **A real device was driven for this fix** — the
maintainer's own `emulator-5554` (Pixel 10a), already booted; no AVD was
created, deleted, or killed.

Every exit code below was read from a file, never from a harness banner —
`<command> > <log> 2>&1; echo $? > <file>`.

This page was first written earlier the same day claiming a fix that had
not actually landed in the source file. The section **"What the first pass
got wrong"** below says so plainly; everything under **"The actual fix"** is
what a second pass measured against the real code.

## The defect, reproduced by hand

`flutter clean` → `flutter build apk --debug` → `adb uninstall` → `adb
install` → launch → fill fields → tap "Start checkout" on `example/`, against
a real session minted through `examples/shop`'s `orders.create` (the `shop`
merchant, `pk_test_shopmerchantsandbox1`), threw:

```
Pre-flight ran; no platform host is registered on this platform:
UnimplementedError: vpay_checkout_flutter: VpayCheckoutPlatform.windowEvents
has no platform host yet.
```

`VpayCheckoutActivity` never opened. All three suites under
`example/integration_test/` had passed because each imported
`support/ensure_platform_registered.dart`, which called
`MethodChannelVpayCheckoutPlatform.registerWith()` by hand — the one thing a
real app never does. `example/lib/main.dart`, the only entrypoint a real
merchant app uses, called it zero times.

## Root cause

Measured with a temporary `print` in `registerWith()` and `adb logcat`
(reverted before landing — `git status` was checked clean of it afterwards):
the Flutter engine invokes `pubspec.yaml`'s `dartPluginClass`-generated
`_PluginRegistrant.register()` when the root isolate starts — **before** the
app's own `main()` body runs, in particular before
`WidgetsFlutterBinding.ensureInitialized()`/`runApp()`, on some launches.
Confirmed via `adb logcat`:

```
`vpay_checkout_flutter` threw an error: Binding has not yet been initialized.
The "instance" getter on the ServicesBinding binding mixin is only available
once that binding has been initialized. ...
```

`MethodChannelVpayCheckoutPlatform`'s constructor called
`VpayCheckoutFlutterApi.setUp(this)` eagerly, which reaches into
`ServicesBinding.instance` immediately to register a `BasicMessageChannel`
handler. With no binding yet, that threw; the engine's own generated wrapper
swallows the exception (only prints it, never rethrows), leaving
`VpayCheckoutPlatform.instance` stuck at `UnimplementedVpayCheckoutPlatform`
for the app's whole lifetime.

**The decisive extra fact, found only on the second pass: this race is
intermittent, not one-directional.** Cold-launching the exact same APK
repeatedly (`adb shell am force-stop` then relaunch) sometimes printed the
diagnostic with no error at all — the binding was already initialized by
the time `register()` ran on that particular launch — and sometimes printed
the exact `Binding has not yet been initialized` throw above. A single
successful hand-driven walk therefore proves nothing about whether the
defect is fixed; only a fix that removes the dependency on this timing
altogether does.

## What the first pass got wrong

Earlier the same day, this page (and `docs/sdks/parity.md`,
`docs/flows/mobile-checkout.md`, `docs/status/mobile-flutter-plugin.md`)
claimed the fix was "`VpayCheckoutFlutterApi.setUp` deferred from the
constructor to the first call to `show`," backed the claim with a
regression test (`test/method_channel_checkout_platform_registration_test
.dart`, still present and still correct as a test), and reported a passing
hand-driven walk with a screenshot.

**That code change was never applied.** `git diff` against
`method_channel_checkout_platform.dart` at the start of the second pass was
empty — the constructor still read `VpayCheckoutFlutterApi.setUp(this)`,
unguarded, exactly as before any fix. Running the regression test against
that unmodified file failed with precisely the error the test's own doc
comment describes, confirming the code this page described was never in the
tree. The hand-driven walk the first pass reported passing is consistent
with the race described above resolving in the lucky direction on that one
run — `ensure_platform_registered.dart` was already deleted from the suites
by that point, so it is not the explanation; ordinary non-determinism is.

## The actual fix

Two changes, both in
`sdks/flutter/vpay_checkout_flutter/lib/src/platform/`:

1. **`method_channel_checkout_platform.dart`**: the constructor's call to
   `VpayCheckoutFlutterApi.setUp(this)` is now wrapped in
   `_trySetUpFlutterApi()`, which catches exactly the `FlutterError` a
   missing `ServicesBinding` throws (any other exception still propagates)
   and retries from `show`, `dismiss` and `windowEvents` — every one of
   which only ever runs once a merchant's own `main()` has, when a binding
   always exists. This alone stops the throw, but still leaves
   `VpayCheckoutPlatform.instance` stuck at
   `UnimplementedVpayCheckoutPlatform` on the unlucky side of the race,
   because `registerWith()` never retries the _assignment_ itself — only
   the internal wiring.

2. **`checkout_platform.dart`** (load-bearing): `VpayCheckoutPlatform
.instance` is now a getter, not a plain static field. The first read
   that finds `UnimplementedVpayCheckoutPlatform` on Android, iOS or macOS
   resolves `MethodChannelVpayCheckoutPlatform` itself, lazily — at the
   point `VpayCheckout.start()` first reads it, which is necessarily after
   the app's own `main()` has run. `dartPluginClass`'s own
   `_PluginRegistrant.register()` call, when it wins the race, is now
   purely an optimisation: whichever path runs first, the result is the
   same concrete instance. The platform check is a `dart:io`-vs-web
   conditional import (`native_mobile_host_stub.dart` returns `false`
   unconditionally; `native_mobile_host_io.dart` checks
   `Platform.isAndroid || Platform.isIOS || Platform.isMacOS`), not
   `defaultTargetPlatform`/`kIsWeb` — that was tried first and rejected,
   because Flutter's own `defaultTargetPlatform` implementation
   (`package:flutter/src/foundation/_platform_io.dart`) forces the result to
   `TargetPlatform.android` whenever the `FLUTTER_TEST` environment variable
   is set (which `flutter test` always sets), which would have silently
   turned every `flutter test` run "Android," defeating
   `test/vpay_checkout_test.dart`'s own `UnimplementedVpayCheckoutPlatform`
   test case. `dart:io`'s `Platform.isAndroid` is not affected by that
   override — it reads the real host OS, and `flutter test` runs on
   Linux/macOS/Windows, never inside an Android emulator's own Dart VM.

## The hand-driven walk, after the actual fix

Same clean-build cycle, same device, same real session minted through
`examples/shop`:

- `adb shell dumpsys activity activities` after tapping "Start checkout"
  shows `topResumedActivity=ActivityRecord{... dev.vpay.vpay_checkout_flutter_example/dev.vpay.checkout_flutter.VpayCheckoutActivity ...}`.
- A screenshot of the real vpay hosted checkout page rendering inside that
  Activity (MTN Mobile Money / Orange Money rails, real `cs_…` reference)
  was saved during this run.
- Repeated afterwards across several cold force-stop/relaunch cycles with
  freshly minted sessions; every run whose fields were confirmed correctly
  filled (checked by screenshot before tapping) opened
  `VpayCheckoutActivity`. Two relaunches in this second round showed a
  _different_, expected failure instead — a pre-flight validation error
  naming an empty or already-superseded session URL, caused by this
  session's own automated-typing script clobbering a field — never
  `windowEvents has no platform host yet`. That is the pre-flight/validation
  path, not the platform-registration path this page is about.

## Gates

| Gate                                                                                                                  | Exit code | What it printed                                                                   |
| --------------------------------------------------------------------------------------------------------------------- | --------- | --------------------------------------------------------------------------------- |
| `flutter test` (package)                                                                                              | `0`       | **83 passed, 0 skipped** (was 82; one new regression test)                        |
| `dart analyze --fatal-infos` (package)                                                                                | `0`       | No issues found                                                                   |
| `dart format --output=none --set-exit-if-changed .` (package)                                                         | `0`       | Formatted 33 files (0 changed)                                                    |
| `test/method_channel_checkout_platform_registration_test.dart` alone, constructor reverted to eager unguarded `setUp` | `1`       | `Binding has not yet been initialized` — the exact real-app failure, reproduced   |
| `test/method_channel_checkout_platform_registration_test.dart` alone, against the actual fix                          | `0`       | All tests passed!                                                                 |
| `just test-flutter-emulator` (`VPAY_EMULATOR_SERIAL=emulator-5554`)                                                   | `0`       | window / dismiss / external-browser suites all green — "all three suites green"   |
| `just test-flutter-web`                                                                                               | `0`       | 5 passed                                                                          |
| `just test-flutter-e2e`                                                                                               | `0`       | 3 passed                                                                          |
| `cargo run -p xtask -- verify-sdk-parity`                                                                             | `0`       | 565 proving tests, 35 dated gaps, 32 methods across 35 rows (baseline, unchanged) |
| `cargo run -p xtask -- verify-links`                                                                                  | `0`       | 1624 links resolve                                                                |
| `just fmt-check-web` (repo-wide, prettier --check .)                                                                  | `0`       | All matched files use Prettier code style                                         |

`just ci` was not run, by instruction.

## What was reverted before landing

The temporary `print` in `registerWith()` used to confirm whether
`_PluginRegistrant.register()` runs at all, and the temporary revert of the
constructor back to the eager unguarded `setUp` call used to re-prove the
regression test still bites. `git status` was checked clean of scratch
changes before finishing; only the two-part fix, the two new
`native_mobile_host_*.dart` files, the pre-existing test, the suite edits
and this documentation remain.
