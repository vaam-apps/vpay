# Tauri v2 checkout plugin (`tauri-plugin-vpay-checkout`)

New page, 2026-09-22. Not a merchant SDK — a payer surface, like
`@vaam-apps/vpay-stripe-js` and
[mobile-flutter-plugin.md](mobile-flutter-plugin.md)'s subject — so it is not
folded into [merchant-sdks.md](merchant-sdks.md). See
[docs/flows/tauri-checkout.md](../flows/tauri-checkout.md) for the process,
[ADR-0023](../adr/0023-tauri-checkout-plugin.md) for the decisions (T1–T7,
on top of ADR-0021's D1–D9 unchanged), and
[docs/plans/2026-09-22-tauri-plugin.md](../plans/2026-09-22-tauri-plugin.md)
for the reasoning behind each.

Requested by the maintainer on 2026-09-22, verbatim: _"Please create a SDK
for andoid+ios+web for tauri v2"_ [sic], and built in parallel lanes against
[docs/plans/2026-09-22-tauri-plugin-brief.md](../plans/2026-09-22-tauri-plugin-brief.md).

## What is real today

Every "exit 0" and every count below is measured; the Verification section
says by whom and links the page that carries the command lines.

| Piece                                                                                 | State                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The Rust crate (`src/`, `build.rs`, `permissions/`)                                   | ✅ builds, `cargo clippy --all-targets -- -D warnings` clean, **19 unit tests + 1 doctest, 0 ignored**, `cargo doc` with 0 warnings, `cargo fmt` clean. `cargo check` succeeds for `aarch64-linux-android` and `aarch64-apple-ios`. Re-measured by the docs lane through `just test-tauri-rust`/`clippy-tauri-rust`/`check-tauri-mobile`, all exit 0.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Its own Cargo workspace (T2)                                                          | ✅ the manifest carries an empty `[workspace]`, so `tauri`'s graph (wry, tao, objc2, webkit2gtk) enters neither the root `Cargo.lock` nor `cargo deny`, `verify-no-mocks`'s `cargo metadata`, or `cargo nextest run --workspace`. The crate restates the root's `[lints.clippy]` deny list itself, because `lints.workspace = true` needs the parent this table severs.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| The desktop host (`src/desktop.rs`)                                                   | ✅ as a state machine: **11 `#[test]`s** cover one-window-at-a-time, refusing an unparseable URL/a non-http scheme/an `http` URL without the named opt-in before the browser is touched, exactly one `dismissed` event, and a failed launch clearing the in-flight slot. It also links into a real desktop binary now: Lane D's `pnpm exec tauri build --debug --no-bundle` exits 0 on macOS aarch64 and the 29,479,920-byte result launched and ran 6 s with no stderr and no panic. What is **not** real: `open::that_detached` has **never executed** — the tests drive a crate-private opener seam, and no `start()` from that binary ever reached a window.                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| The guest-JS package (`@vaam-apps/vpay-tauri-checkout`)                               | ✅ typecheck, lint at `--max-warnings 0`, build and **71 vitest cases across 8 files, 0 skipped**, all exit 0. It is the whole state machine (T1): parse, pre-flight, stop-URL derivation, the poll ladder, the terminal rule, and both JavaScript hosts.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Its place in the pnpm workspace                                                       | ✅ `pnpm-workspace.yaml` names `sdks/tauri/*`, so `just lint-web` (`pnpm -r typecheck`, `pnpm -r lint`), `just test-web` (`pnpm -r test`) and `just fmt-check-web` (`prettier --check .`) all reach it with **no recipe change**, and CI's `web` job filter already names `**/*.ts`. The TypeScript half is genuinely CI-gated; nothing else here is.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| The Android host (`android/`)                                                         | ✅ **compiles, and has taken a real payment.** Lane D: `pnpm exec tauri android build --debug --target aarch64 --apk` on `examples/tauri-checkout` exits 0; `dexdump` finds `Ldev/vpay/tauri/checkout/…` classes in the 134,491,016-byte APK; `aapt2 dump xmltree`, read out of that **built APK**, shows `VpayCheckoutActivity` `exported="false"` and `VpayCheckoutAppLinkActivity` `exported="true"`. Lane B's flagged `jvmTarget` risk did not materialise (plugin at JVM 17, `:tauri-android` and the app at 1.8, no fallback); one deprecation warning at `VpayCheckoutActivity.kt:349` (`getParcelableArrayListExtra`). **Lane D2, later the same day:** on an API 36 arm64 emulator against a running vpay, `show` crossed the IPC boundary, the Custom Tab opened on the real hosted page, an MTN push reached `Payment received`, and closing the tab resolved **`succeeded`** for `pi_93htevhkrn5vn23ys9e2se70`. No `AndroidRuntime`, no crash, and no vpay log line (D6 — the Kotlin has none). **One gap found doing it:** Chrome presented the tab **full-height**, declining the partial sheet the plugin requests. |
| The iOS host (`ios/`)                                                                 | ✅ **compiles, links, and has taken a real payment.** `swift build --sdk iphonesimulator` and `xcodebuild` for a generic Simulator destination both exit 0 (Lane C); `tauri ios build --debug --target aarch64-sim` reports BUILD SUCCEEDED and `nm` finds `_init_plugin_vpay_checkout` and the mangled Swift symbols in the app binary (Lane D). **Lane D2:** on an iPhone 17 simulator (iOS 26.3.1) against a running vpay, the `SFSafariViewController` opened on the real hosted page, an MTN push reached `Payment received`, and closing the sheet resolved **`succeeded`** for `pi_xpprqgpved3gh58p0yqsvs7v`; a second run swiped the sheet away unpaid and resolved **`pending`**, with a merchant-token read confirming `requires_payment_method`. **Not exercisable here:** dismiss-from-the-app — the `.large()` detent covers the app's own UI, and dragging the grabber dismisses outright.                                                                                                                                                                                                                           |
| `stopUrlReached` on iOS                                                               | ⛔ **unreachable**, not merely unverified (T6). tauri-v2.11.6 gives a Swift plugin no app-lifecycle hook, and Tauri's own `deep-link` plugin ships no `ios/` at all. Every iOS checkout ends `dismissed`; `matchesStopUrl`/`handleUniversalLink` exist and nothing calls them. D1/D4 make that correct — and Lane D2's two successful checkouts are the proof of the pudding rather than an exception: both ended with a **manual close plus the poll**, and both were right.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| The example app                                                                       | ✅ [`examples/tauri-checkout`](../../examples/tauri-checkout/) builds on all three targets — desktop (`tauri build --debug --no-bundle`, a 29,479,920-byte binary that launched and ran 6 s clean), iOS simulator and Android APK, every command exit 0 — and it is what drove both real checkouts above. **Zero edits to the plugin were needed by any build or any run**, which is the strongest single thing this row says: every deviation the writing lanes recorded survived a real Gradle, a real Xcode, a real cargo link and a real payment unchanged. It also confirms `bundle.iOS.minimumSystemVersion: "15.0"` reaches the compiler (generated `pbxproj` carries `IPHONEOS_DEPLOYMENT_TARGET = 15.0`, the Podfile `platform :ios, '15.0'`), and that the five tracked permission files are regenerated **byte-identical** by every app build. Its README records a trap worth knowing: an **unquoted** `VITE_VPAY_SESSION_URL` is silently truncated at the `#`, because vite's dotenv parser reads it as a comment — and that `#` fragment is the session secret.                                                     |
| `just test-tauri-rust` / `clippy-tauri-rust` / `test-tauri-js` / `check-tauri-mobile` | ✅ exist, each preceded by a `_tauri-preflight` that refuses with a named reason when the plugin directory or its manifest is missing. **None of the three Rust ones is in `just ci` or `just verify`** (T7) — confirmed by reading both recipe lists; neither names any of them, and the justfile's own gate tally is unchanged at fifteen.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `release-please-config.json`                                                          | ✅ two new `extra-files` entries — a `generic` one for `Cargo.toml` (whose `version` line carries `# x-release-please-version`) and a `json` one at `$.version` for `package.json`. Neither is a bare string, for the reason #203/#204 wrote down. `cargo xtask verify-versions` now counts **24** references, all `0.4.0` (was 22).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `docs/sdks/parity.md`                                                                 | ✅ a fourth table, one column, **88 new proving tests and 7 dated ⛔ rows** — `verify-sdk-parity` goes from 662/37 to **750 proving tests / 44 dated gaps**.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ADR-0023                                                                              | ⚠️ [docs/adr/0023-tauri-checkout-plugin.md](../adr/0023-tauri-checkout-plugin.md) — **Proposed, needs maintainer acceptance.** T1–T7 were taken by the implementing agent; the maintainer asked for the surface and the fan-out and nothing else here has been put to them.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |

## What is still not real

- **No gate compiles the Rust, the Kotlin or the Swift** (T7). The CI image
  has no `libwebkit2gtk-4.1-dev`, no Android SDK/NDK and no Xcode; adding any
  of them is a runner-image change. Every Rust count this repository quotes
  for this crate is a human running a `just` recipe by hand. The TypeScript
  half is the exception and is genuinely gated.
- ~~**The Kotlin was compiled by nobody in the lanes that wrote it.**~~
  **Closed the same day by Lane D**, and the sentence is struck rather than
  deleted because the reason it was true is still the shape of this plugin:
  nothing inside `sdks/tauri/` can compile the Kotlin, and no gate or `just`
  recipe does it either — only a consuming app's Gradle project can, and
  `examples/tauri-checkout` is now that app. See the Android row above for
  what the built APK actually contains.
- ~~**No browser sheet has ever opened, on any platform.**~~ **Closed
  2026-09-22 by Lane D2, for iOS and Android.** The
  `SFSafariViewController` and the Custom Tab both opened on vpay's **real
  hosted checkout page**; `show` and `dismiss` crossed the IPC boundary into
  the Swift and the Kotlin at runtime; MTN pushes were driven through each to
  `Payment received`; and both resolved **`succeeded`**. A third run
  dismissed an unpaid sheet and resolved **`pending`** — D4, on a real
  intent, never `canceled`. **Still true for desktop**, which no run has ever
  touched. The struck sentence is kept because it was the honest state for
  most of the day and because the part of it about desktop still holds.
- **The Android Custom Tab presented full-height, not partial.** Chrome
  declined the 90 % sheet the plugin requests via
  `setInitialActivityHeightPx`, offering "Minimize tab to return to it later"
  instead. The request does reach the platform — the task record carries the
  bounds — so this is the browser's choice, not a plugin defect; but every
  description of the Android surface as a "partial Custom Tab" is describing
  a request, and now says so. Not reproduced on a physical device or another
  browser.
- **`stopUrlReached` has never fired on any platform.** Unreachable on iOS
  (T6); not implemented on desktop, where an OS-registered URL scheme belongs
  to the application rather than to this plugin; wired on Android and needing
  a merchant-verified host serving `assetlinks.json`, which this repository
  can neither deploy nor prove against — the same wall
  [mobile-flutter-plugin.md](mobile-flutter-plugin.md)'s equivalent hit on
  2026-09-16.
- **Desktop has no dismissal signal at all** (T4), by decision rather than by
  oversight: handing a URL to the default browser leaves no handle on the
  window. `dismiss()` is the only trigger, and the host deliberately invents
  no focus-based signal — the Flutter macOS host shipped exactly that fake
  and removed it on 2026-09-16.
- **`open::that_detached` has never executed.**
- **A host that resolves `show` and then never reports anything hangs
  `start`.** There is no stream-done signal on that seam. Named by the lane
  that built it, not fixed. A host whose `show` _rejects_, or which closes
  without reporting after rejecting, resolves `unresolved` correctly.
- ~~**Nothing has run against a running vpay**, and therefore nothing against
  a rail of any kind.~~ **The vpay half closed 2026-09-22 by Lane D2**, which
  succeeded where Lane D's attempt died on a full disk: the stack came up
  with **zero image builds**, because `ghcr.io/vaam-apps/vpay-server:edge`
  and `vpay-checkout:edge` pull anonymously. Three real sessions were minted
  by `curl` doing exactly what `examples/shop/src/server/orders.ts` does —
  real `private_key_jwt` `client_credentials`, real
  `POST /v1/payment_intents`, real form-encoded
  `POST /v1/checkout/sessions` — and two were paid to `succeeded`, each
  cross-checked with a **merchant-token** `GET /v1/payment_intents/{id}`.
  **Three caveats, all load-bearing:** the stack was **`edge` images from
  GHCR, not a build of this tree**; `vpay-shop` never ran, so **no merchant
  webhook was verified** and the merchant-side evidence is a
  token-authenticated read rather than the shop order row the Flutter e2e
  suite asserts on; and the plugin's own suites still open no socket.
- **No real rail, and this half did not move.** The rail behind Lane D2's
  stack is a `wiremock/wiremock` container (`../status.md`'s banner) — its
  journal shows real `POST /collection/token/`,
  `POST /collection/v1_0/requesttopay` and
  `GET /collection/v1_0/requesttopay/{ref}` calls **to WireMock**. No MTN
  endpoint has been called and no money has moved. Orange Money and the
  failure MSISDNs were not exercised at all.
- ~~**No device, emulator or simulator run of any kind.**~~ **Narrowed
  twice the same day:** first a launch (Lane D), then a **full checkout**
  driven on both a headless Android emulator (`vpay_tauri`, API 36 arm64)
  and an iPhone 17 iOS Simulator (iOS 26.3.1) (Lane D2). **Still not real:
  no physical device**, no desktop checkout of any kind, no release build,
  no Windows or Linux desktop build, and `tauri ios dev`/`tauri android dev`
  were not used — artefacts were installed directly. The iOS UI was driven
  by AppleScript keystrokes through Simulator.app, because the native
  simulator tool refused to run on this host (it looks for
  `/var/db/xcode_select_link`, which is absent), so those results are what
  was on screen rather than what a harness read back.
- **No window has ever closed itself.** Every checkout this plugin has ever
  completed ended because a human closed the sheet and D4's poll then
  decided. That is correct by design and it is also the whole of the
  evidence: the self-closing path needs `stopUrlReached`, which has never
  fired.
- **No App Store or Play review.** ADR-0021's D9 reading of the published
  rules carries over unchanged.
- **Nothing is published.** The crate is `publish = false` with a stated
  reason; the npm package is publish-ready and published by nothing.
- **`verify-npm-scope` has not yet seen the new manifest**, because it walks
  `git ls-files` and the tree was untracked when this page was written. It
  will the moment the tree is staged.

## Verification

- [verification/2026-09-22-tauri-plugin.md](verification/2026-09-22-tauri-plugin.md)
  — every command above with its exit code, split into what each building
  lane measured and what the docs lane re-ran in its own pass; the
  `cargo check --target aarch64-apple-ios` exit 101 that swift-rs's iOS 13.0
  default caused and the `if #available(iOS 15.0, *)` guard that fixed it;
  every deviation from the brief each lane recorded; and — § "Lane D" —
  the example app's three builds with their sizes and their two instructive
  first failures (`generate_context!` reading `bundle.icon` with
  `bundle.active` false, and Android refusing a `0.0.0` version), what
  `dexdump`/`aapt2`/`nm` read out of the built artefacts, the emulator and
  simulator launches, the `just demo-up` disk exhaustion that left the
  plugin unexercised, and the Homebrew packages `tauri ios init` installed
  without asking.
- [verification/2026-09-22-tauri-plugin.md § "Lane D2"](verification/2026-09-22-tauri-plugin.md)
  — the stack-driven runs, later the same day: how the stack came up with
  zero image builds, the three session/intent id pairs, the verbatim screen
  text from vpay's real hosted page and from the app, the merchant-token
  cross-check table, Chrome's first-run trap and the full-height Custom Tab
  finding, and a "what Lane D2 did not verify" list that is longer than the
  results.
- [`examples/tauri-checkout/README.md`](../../examples/tauri-checkout/README.md)
  § Status — Lane D's own primary record, including the parts about the
  example app rather than the plugin (its CSP, its `0.0.1` version, and why
  `src-tauri/gen/` is not tracked).
