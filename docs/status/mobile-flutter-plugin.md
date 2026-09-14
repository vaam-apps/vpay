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
| iOS and macOS hosts                     | ⛔ the Swift exists and is **compiled by nobody** — Linux host, no `xcodebuild`, reviewed by reading only.                                                                                                                                                                                            |
| `VpayCheckoutMode.externalBrowser` (D8) | ⛔ designed, not built. Until 2026-09-14 the parameter was accepted and silently ignored — the caller got the in-app WebView. It now throws `UnimplementedError`.                                                                                                                                     |
| The parity table                        | ✅ `docs/sdks/parity.md`'s third table, 550 proving tests across the file, every Flutter ✅ cell naming a Dart test that actually ran.                                                                                                                                                                |

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

## What is still not real

- **No `just ci` gate** (D-M3). `install-flutter`/`analyze-flutter`/
  `test-flutter`/`test-flutter-e2e` exist; none is in `just ci` or
  `just verify`, and `docs/status.md`'s gate table does not claim otherwise.
  Every count this repository quotes for this package is a human running it
  by hand.
- **No iOS or macOS compile.** Not "not yet run" — there is no toolchain on
  this host and there cannot be.
- **No device, no emulator, no browser.** Android is proven by compiling and
  web by compiling; neither has been opened.
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
