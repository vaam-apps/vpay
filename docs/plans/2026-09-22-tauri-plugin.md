# Tauri v2 checkout plugin — one state machine, three hosts

- **Date:** 2026-09-22
- **Requested by the maintainer**, verbatim: _"Please create a SDK for
  andoid+ios+web for tauri v2"_ [sic], then: _"use opus sub agents to fan out
  the biggest work and you, play only the orchestrator role"_.
- ~~**Status: built, unproven in every place that matters.** The code exists
  and its own unit suites pass; nothing here has spoken to a running vpay, a
  real rail, a device or a simulator.~~ **Narrowed the same day by Lane D2,
  and the header is corrected 2026-09-23 so that a reader who stops here is
  not misled** — § "What will not be true when this ships" below has carried
  the strikethroughs since; this bullet had not. **Status: built, driven for
  real on two simulators, and still unproven where it matters most.** The
  plugin has spoken to a **running vpay** (`ghcr.io/vaam-apps/vpay-*:edge`,
  not a build of this tree), on an **iOS Simulator and an Android
  emulator**, and took two payments through vpay's own hosted page to
  `succeeded`. What stands, unchanged: **no real rail** — every rail this
  repository has ever driven is a WireMock container, so no money has moved;
  **no physical device**; **no desktop run of any kind**, so
  `open::that_detached` has still never executed; and **the plugin's own
  suites open no socket** — every count in `sdks/tauri/` is a unit suite
  against a scripted `fetch`. What _is_ measured is on
  [`../status/verification/2026-09-22-tauri-plugin.md`](../status/verification/2026-09-22-tauri-plugin.md),
  and what is still missing is on
  [`../status/mobile-tauri-plugin.md`](../status/mobile-tauri-plugin.md).
- **Inherits, unchanged:** [ADR-0021](../adr/0021-flutter-checkout-plugin.md)
  and [`2026-09-13-flutter-plugin.md`](2026-09-13-flutter-plugin.md)'s D1–D9.
  This document adds **T1–T7**, the decisions that are about Tauri and about
  nothing else. Where a rule below says "D4" or "D6" it means that document's
  rule, not a restatement of it.
- **Decisions:** [ADR-0023](../adr/0023-tauri-checkout-plugin.md).
- **Contract the lanes built against:**
  [`2026-09-22-tauri-plugin-brief.md`](2026-09-22-tauri-plugin-brief.md).
- **Base:** `master` at `7134ecb`, branch
  `claude/tauri-v2-sdk-multiplatform-ca900b`.

## What already existed, read rather than assumed

- **The Flutter plugin has already answered every question that is not about
  Tauri.** D1 (the outcome is polled, never read off a URL), D2 (one
  pre-flight read, stop URLs derived from the session), D4 (a dismissal polls
  before it reports), D5 as revised on 2026-09-16 (the payer's own browser,
  never a WebView), D6 (the credential never reaches a log line) and D9 (the
  App Store / Play bounds on adoption) are decided, argued and — for Flutter
  — partly proven. Re-deciding any of them here would have produced a second
  answer to a settled question on a second surface, which is exactly the
  divergence [ADR-0015](../adr/0015-sdk-parity.md) exists to prevent.
- **A browser client that already speaks the two reads.**
  `@vaam-apps/vpay-stripe-js` ([`sdks/stripe-js`](../../sdks/stripe-js/))
  has `loadStripe(pk, { baseUrl, fetch })`, `retrieveCheckoutSession` and
  `retrievePaymentIntent`. Porting `browser_client.dart` into TypeScript
  would have been a third implementation of the same two GETs.
- **`/v1/browser` is CORS `Any` with credentials off**, and nowhere else is
  (`backends/crates/vpay-api/src/lib.rs`), so a Tauri webview reaches those
  routes with no origin registration, whatever custom-protocol origin the
  platform gives it.
- **The hosted URL is `{base}/c/{cs_id}?key={pk}#{cs_secret}`** — publishable
  key in the query, session secret in the **fragment**. That fragment is why
  no error message in either half of this plugin may interpolate a URL (D6).

## The shape

```text
 guest-JS — @vaam-apps/vpay-tauri-checkout (every decision; runs in and out of Tauri)
 ┌───────────────────────────────────────────────────────────────────────┐
 │ VpayCheckout.start(sessionUrl)                                        │
 │   1. parse       url → base, cs_id, pk, session secret (fragment)     │
 │   2. pre-flight  GET /v1/browser/checkout/sessions/{cs_id}            │
 │                  → ui_mode, success_url, cancel_url, expires_at,      │
 │                    payment_intent (expanded) + its client_secret      │
 │   3. show        ─────────────► CheckoutHost.show(request, onEvent)   │
 │   4. watch       ◄───────────── exactly one CheckoutWindowEvent       │
 │   5. resolve     poll GET /v1/browser/payment_intents/{pi} on a budget│
 │   6. answer      VpayCheckoutResult — never throws                    │
 └───────────────────────────────────────────────────────────────────────┘
        │ tauriHost(): invoke("plugin:vpay-checkout|show", { url, stopUrls,
        │              allowInsecureUrl, onEvent: Channel })
        ▼
 ┌─────────────────────────────────────────────────┬─────────────────────┐
 │ tauri-plugin-vpay-checkout (Rust) — the window  │ webHost(): no Tauri │
 ├───────────────┬───────────────┬─────────────────┤                     │
 │ Android       │ iOS           │ Desktop         │ window.open popup,  │
 │ partial       │ SFSafariView- │ default browser │ origin- AND source- │
 │ Custom Tab    │ Controller    │ (`open` crate)  │ pinned vpay:complete│
 │ dismiss: yes  │ dismiss: yes  │ dismiss: NONE   │ dismiss: `closed`   │
 └───────────────┴───────────────┴─────────────────┴─────────────────────┘
   Every host's whole vocabulary: { outcome: "dismissed" | "stopUrlReached",
                                    reachedUrl: string | null }, once per show.
```

The hosts are deliberately stupid — **show this URL; tell me the payer left,
or that a deep link matched.** None of them formats money, knows a status
word, or holds the URL a moment longer than the window is open.

## T1 — the state machine lives in guest-JS, not in Rust

**Decision.** `VpayCheckout`, `CheckoutController`, the stop-URL rules, the
poll ladder and the terminal rule are TypeScript under `guest-js/`. The Rust
crate, the Kotlin host and the Swift host between them hold no rule about
money at all.

**Reason.** The maintainer asked for **web** as well as Android and iOS, and
on the web there is no Rust: a plain browser loading this package has a
`window`, a `fetch` and nothing else. Putting the state machine in Rust would
have meant either shipping a wasm build of it to the web (a second artefact,
a second set of failure modes, for a state machine that is a few hundred
lines of `if`) or writing it twice. It is written once, and the same 71
vitest cases cover the Tauri path and the plain-browser path.

**Consequence.** D1 and D4 — the two rules a payer's money depends on — are
proven by tests that need no device, no emulator and no toolchain beyond
Node. That is the trade this decision buys, and it is the same one
`2026-09-13-flutter-plugin.md`'s "the platform half is deliberately stupid"
made for Dart.

## T2 — the crate is its own Cargo workspace

**Decision.** `sdks/tauri/tauri-plugin-vpay-checkout/Cargo.toml` carries an
empty `[workspace]` table. The repository root's `Cargo.toml` is untouched
and the crate is a member of nothing.

**Reason.** `tauri` pulls `wry`, `tao`, `objc2` and — on Linux —
`webkit2gtk`. Making this crate a workspace member would put all of that into
the root `Cargo.lock`, and therefore into `cargo deny check`, into
`verify-no-mocks`'s `cargo metadata` sweep, and into `just ci`'s
`cargo nextest run --workspace`. On the CI image, which has no
`libwebkit2gtk-4.1-dev`, `cargo nextest run --workspace` would then stop
compiling — every test in the repository red, because a payer-surface plugin
was added.

**Consequence, stated plainly rather than discovered later.** **No root gate
compiles this crate.** `just clippy`, `just test-rust`, `just deny`,
`verify-no-mocks` and `verify-serde` (which scans `backends/crates` only) all
walk past it. The crate restates the root workspace's `[lints.clippy]` block
in its own manifest for that reason — `lints.workspace = true` needs a parent
workspace, and the `[workspace]` table is precisely what severs that link —
and the new `just clippy-tauri-rust` / `just test-tauri-rust` recipes are the
only things that read it. See T7.

## T3 — the two reads reuse `@vaam-apps/vpay-stripe-js`

**Decision.** `guest-js` depends on `@vaam-apps/vpay-stripe-js` at
`workspace:*` and calls `loadStripe(pk, { baseUrl, fetch })`, then
`retrieveCheckoutSession` for the pre-flight and `retrievePaymentIntent` for
the poll. It does not port `browser_client.dart`.

**Reason.** One wire, one implementation. A divergence between two TypeScript
clients speaking the same two routes in the same repository would be
invisible until a payer hit it.

**The trap, inherited from the Flutter e2e suite's 2026-09-14 finding.**
`CheckoutSession.client_secret` is typed `string` in that package and the
server **never sends it back**. The polling credential is
`session.payment_intent.client_secret` — the expanded intent's own secret —
and `controller.ts` reads that one. `the polling credential is the intent's
client_secret, never the session's` is the test that pins it. Touching
`session.client_secret` would compile, typecheck, and fail every real
pre-flight.

**The second thing that package does not do:** `loadStripe` accepts an
`http://` base URL without complaint. D6 says plaintext is a demo-stack
affordance asked for by name, so `VpayCheckout`'s **constructor** refuses a
non-`https` `baseUrl` unless `allowInsecureBaseUrl: true` is passed — before
`loadStripe` is ever called.

## T4 — desktop opens the default browser and has no dismissal signal

**Decision.** On macOS, Windows and Linux the host calls
`open::that_detached(url)` and keeps the channel. It reports `dismissed`
**only** when the application calls `dismiss()` while a show is in flight. It
invents no focus-based, window-activation-based or timer-based signal.

**Reason.** This is the Flutter macOS host's situation exactly, and that host
already made the mistake once: it used to report `dismissed` when the app
merely regained focus, which is not evidence that the browser closed. That
fake signal was removed on 2026-09-16. Re-inventing it on a second surface
would have been a guess dressed as an event.

**Why it is a latency gap and not a money gap.** D4 polls
`GET /v1/browser/payment_intents/{id}` on a budget and answers `pending` when
the budget elapses; D1 means the window never decided anything anyway. A
window that reports nothing costs a merchant a slower answer, not a wrong
one. `desktop.rs`'s own module header says this at length, and
`a_valid_https_show_launches_the_browser_on_exactly_the_url_it_was_given`
and `dismiss_sends_exactly_one_dismissed_event` are what hold the behaviour
still.

Deep links, and therefore `stopUrlReached`, are **not implemented on desktop
at all**: an OS-registered URL scheme belongs to the _application_
(`tauri-plugin-deep-link`'s territory), and this plugin cannot register one
on a merchant's behalf.

## T5 — outside Tauri, the same package is the popup host

**Decision.** `defaultHost()` is `isTauri() ? tauriHost() : webHost()`.
`webHost()` opens a `window.open(url, "vpay-checkout", …)` popup, listens for
`{ type: "vpay:complete" }`, and polls `popup.closed` every 500 ms.

**Reason.** "android + ios + web" was the request, and the web third of it is
not a Tauri question at all — it is the protocol
`@vaam-apps/vpay-stripe-js`'s own popup surface already speaks. Making it a
_host_ rather than a separate package means the merchant's code is the same
three lines in a Tauri app and in a web build of the same front end.

**The security property, and it is the whole of this host.** A
`vpay:complete` message is accepted only when `event.origin` is the page's
**own** origin **and** `event.source` is the exact `Window` this host opened.
Origin alone is not enough — the Flutter web suite found that on 2026-09-15,
where any same-origin window could spoof the completion — so both pins are
asserted:
`a message from a different origin is ignored — the whole security boundary of this host`
and `a message from another window on the same origin is ignored`. And the
message never carries the outcome:
`the outcome is never read off the message payload, whatever status or session it carries (D1)`.

`stopUrls` and `allowInsecureUrl` are **inapplicable** here and the host says
so rather than pretending: a cross-origin popup's location cannot be read
from the opener, and the browser's own address bar already shows the payer
the scheme.

## T6 — iOS cannot report `stopUrlReached`, so every iOS checkout ends `dismissed`

**Decision.** The iOS host implements the Universal-Link matching rule (D2)
and the entry point a hook would call, and **nothing calls them**. They are
kept, loudly labelled, rather than deleted or faked.

**Reason, measured at tag `tauri-v2.11.6` by the lane that wrote the Swift,
2026-09-22.** Three things were checked before concluding it:

1. `crates/tauri/mobile/ios-api/Sources/Tauri/Plugin/Plugin.swift`'s `Plugin`
   base class exposes `load(webview:)`, `checkPermissions`,
   `requestPermissions`, `trigger`/`registerListener`/`removeListener` — and
   no app-lifecycle hook of any kind.
2. A grep for `userActivity`, `NSUserActivity`, `openURL`,
   `UIApplicationDelegate` and `applicationDelegate` over every file in
   `crates/tauri/mobile/ios-api/Sources/Tauri/` returns nothing.
3. `tauri-apps/plugins-workspace@v2`'s **`deep-link`** plugin — the one
   plugin whose entire job is Universal Links — **ships no `ios/`
   directory**. It handles iOS on the Rust side, matching
   `tauri::RunEvent::Opened { urls }`. That is the only iOS deep-link seam
   Tauri has, and a Swift plugin cannot reach it.

The Flutter host got this for free from
`registrar.addApplicationDelegate(self)`, which Tauri has no equivalent of.

**Why it is correct rather than broken.** D1 and D4 make `dismissed` the
ordinary end of a _successful_ payment too: a payer who paid and then tapped
Done resolves `succeeded`, because the poll — not the window — decides. It is
a latency and diagnostics gap, and it is recorded as a dated ⛔ in
[`../sdks/parity.md`](../sdks/parity.md) rather than left implicit in a wire
contract that reads as though three hosts implement it.

The honest alternative to keeping dead matching code would have been to
delete the D2 rule from the only host that will eventually need it, or to
invent a hook. Neither is better.

**A second iOS fact, because it reads like a toolchain guarantee and is
not.** The iOS floor is **15.0** — Swift Concurrency, and
`sheetPresentationController`'s detent API, which the `.large()` detent needs
— but that floor is `ios/Package.swift`'s `platforms: [.iOS(.v15)]`
**declaration plus an `if #available(iOS 15.0, *)` guard, and nothing in the
build enforces it.** `cargo check --target aarch64-apple-ios` never reads
that manifest: it goes through `tauri_utils::build::link_apple_library` →
`swift_rs::SwiftLinker::with_ios(…)`, which reads
`IPHONEOS_DEPLOYMENT_TARGET` and **defaults to `"13.0"`**. So the Swift
sources must go on compiling at `-target arm64-apple-ios13.0`, and the one
iOS-15-only call site is reached only inside that guard. Before the guard
existed a bare `cargo check --target aarch64-apple-ios` exited **101**; that
is the regression signal, and it is the only one. The brief's "The iOS host"
section says the floor is "what Tauri 2 itself requires", which does not
hold and is corrected in its own appended note.

## T7 — none of the Rust half is in `just ci`; the TypeScript half already is

**Decision.** `just test-tauri-rust`, `just clippy-tauri-rust` and
`just check-tauri-mobile` exist and are **not** wired into `just ci` or
`just verify`. `just test-tauri-js` exists as a scoped convenience; the
package's own tests run in CI regardless.

**Reason.** The Linux CI image has no `libwebkit2gtk-4.1-dev` (which the
`tauri` crate needs to build at all on Linux), no Android SDK/NDK and no
Xcode. Adding any of the three would be a change to the runner image, which
is the maintainer's call and not a thing a payer-surface plugin should block
on. This is D-M3's reasoning applied a second time, and it is recorded the
same way it was for Flutter: as dated ⛔ rows in
[`../sdks/parity.md`](../sdks/parity.md), which is the honest form of
"later".

**Which half is gated, precisely.** `pnpm-workspace.yaml` gained
`"sdks/tauri/*"`, so `@vaam-apps/vpay-tauri-checkout` is an ordinary member
of the pnpm workspace. `just ci` runs `lint-web` (`pnpm -r typecheck` and
`pnpm -r lint`) and `test-web` (`pnpm -r test`), both of which therefore
include this package with no recipe change; `fmt-check` runs
`prettier --check .` over it; and CI's `web` job filter already names
`**/*.ts`, `**/*.json` and `**/*.md`, so a change to this package fires that
job. **`.github/workflows/ci.yml` needed no edit for any of this.** What no
job anywhere compiles is `src/*.rs`, `android/**/*.kt` and
`ios/**/*.swift`.

**And the same question about `examples/tauri-checkout`, which this section
stopped one sentence short of — added 2026-09-23.** The plugin crate's
exclusion from `just ci` is written down in five places; the example's was
written down in none, and it is the **more dangerous** of the two, because
the example is the only thing in this repository that can compile the
Kotlin and the Swift at all. The honest split, measured on `dd1a48b`:

- **Gated.** `pnpm-workspace.yaml` globs `examples/*`, so the example is a
  pnpm workspace member exactly as the plugin package is. Its `typecheck`,
  `lint` and `test` scripts therefore run inside `just ci` through
  `lint-web`'s `pnpm -r typecheck` / `pnpm -r lint` and `test-web`'s
  `pnpm -r test` — and each of those chains `pnpm run deps` first, so the
  guest-JS package is **built** in CI too. "Nothing builds the example" is
  wrong and would be a worse sentence than none.
- **Ungated, and it is exactly two things.** `src-tauri/` — the Rust, and
  with it every route to `android/**/*.kt` and `ios/**/*.swift` — plus any
  script `pnpm -r` never calls, which today is **`dev`**. That pair is
  precisely what #241 had to repair, and the maintainer found both on real
  hardware rather than any gate finding them.
- **Root cause.** `examples/tauri-checkout/src-tauri/Cargo.toml` carries its
  **own `[workspace]` table**, so T2's isolation applies a second time: no
  root cargo command — `cargo nextest run --workspace`, `just clippy`,
  `cargo deny`, `verify-no-mocks`'s `cargo metadata` — ever resolves it.
  `examples/tauri-checkout` appears in **0** justfile recipes, **0** files
  under `.github/workflows/` and **0** places in `.xtask/src/`.

A gate for it (`cargo check --locked` in `src-tauri/`) is **deliberately not
added here**: it would move the machine-checked `count:verify-gates` figure,
and the justfile is explicit that putting Tauri Rust into CI is a
runner-image change and the maintainer's call. This is the flag, not the
decision.

## What will not be true when this ships

- ~~**No vpay stack.**~~ **Closed 2026-09-22, later the same day (Lane D2):**
  `examples/tauri-checkout` drove the plugin against a real, running
  `vpay-server` from an iOS Simulator and an Android emulator. Still true of
  the plugin's **own suites**: no test in `sdks/tauri/` opens a socket, and
  the two reads there go through `@vaam-apps/vpay-stripe-js` over a scripted
  `fetch`. And the stack was `ghcr.io/vaam-apps/vpay-*:edge`, **not a build
  of this tree**.
- ~~**No payment.**~~ **Closed the same day:** three real checkout sessions,
  two paid through the real hosted page to `succeeded` and confirmed by a
  merchant-token `GET /v1/payment_intents/{id}`; the third dismissed unpaid
  and correctly `pending`. **No merchant webhook was verified** — no shop
  ran — so the merchant-side evidence is a token-authenticated read rather
  than the shop order row the Flutter e2e suite asserts on.
- **No rail. Unchanged, and it is the one that matters.** Every rail in
  every stack this repository has ever driven is a `wiremock/wiremock`
  container (`docs/status.md`'s banner), Lane D2's included: the MTN journal
  shows real `requesttopay` calls **to WireMock**. No MTN endpoint has been
  called and no money has moved.
- **No deep link, anywhere.** `stopUrlReached` is unverified on every
  platform, for the same reason the Flutter plugin's is: this repository
  serves no `assetlinks.json` and no `apple-app-site-association`. On iOS it
  is worse than unverified — it is unreachable (T6). On desktop it is not
  implemented (T4). **Unchanged by the two real checkouts above, and in a
  sense demonstrated by them:** both ended with a human closing the window
  and D4's poll deciding, which is the only ending this plugin has.
- **No store review.** D9's reading of the published Apple and Google rules
  carries over unchanged; a reviewer's verdict is a different thing.
- **`open::that_detached` has never executed.** The desktop host's eleven
  unit tests drive a crate-private `BrowserOpener` seam; the one shipping
  implementation that actually launches a browser is covered by no test and
  has not been run in an app.
- ~~**The Kotlin was compiled by none of the lanes that wrote it**, and by no
  gate or `just` recipe here.~~ **Closed the same day, 2026-09-22, by
  `examples/tauri-checkout`**: `pnpm exec tauri android build --debug
--target aarch64 --apk` exits 0, `dexdump` finds the plugin's classes in the
  APK, and `aapt2 dump xmltree` reads both Activities out of the built APK's
  own manifest with the expected `exported` values. The half that stands: no
  **gate** and no `just` recipe builds it, and only a consuming app's Gradle
  project ever can. Lane B's flagged `jvmTarget` risk did not materialise.
- ~~**No device, no emulator, no simulator.**~~ **Narrowed the same day**:
  the example app launched and rendered on a headless Android emulator and
  on an iPhone 17 iOS Simulator. What stands: **no physical device**, no
  release build, and no Windows or Linux desktop build.
- ~~**No browser sheet has ever opened, on any platform** — the sentence that
  replaces the two above and is the one worth keeping. The Custom Tab, the
  `SFSafariViewController` and `open::that_detached` are compiled, linked
  and unexecuted; `show` and `dismiss` have never crossed the IPC boundary at
  runtime.~~ **Closed for iOS and Android 2026-09-22, later the same day
  (Lane D2)**: both sheets opened on vpay's real hosted page, `show` and
  `dismiss` crossed the IPC boundary into the Swift and the Kotlin, MTN
  pushes were driven through each to `Payment received`, and both resolved
  **`succeeded`**; a third run dismissed an unpaid sheet and resolved
  **`pending`**. **Desktop is the one host this still describes** —
  `open::that_detached` has never executed.

Every claim in the three bullets above is Lane D's or Lane D2's, measured
2026-09-22 and written up in
[`../status/verification/2026-09-22-tauri-plugin.md`](../status/verification/2026-09-22-tauri-plugin.md)
§§ "Lane D" and "Lane D2".

## Decisions taken, and by whom

| #   | Decision                                                                                               | Taken                                                       |
| --- | ------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------- |
| —   | Build a Tauri v2 checkout SDK for Android, iOS and web                                                 | maintainer, 2026-09-22 (verbatim request at the top)        |
| —   | Fan the work out across parallel sub-agents, orchestrator-only at the top                              | maintainer, 2026-09-22                                      |
| T1  | The state machine is guest-JS; the Rust/Kotlin/Swift hosts decide nothing                              | implementing agent — **needs maintainer acceptance**        |
| T2  | The crate is its own Cargo workspace, so no root gate compiles it                                      | implementing agent — **needs maintainer acceptance**        |
| T3  | The two reads reuse `@vaam-apps/vpay-stripe-js` rather than a ported client                            | implementing agent — **needs maintainer acceptance**        |
| T4  | Desktop opens the default browser and has no dismissal signal; `dismiss()` is the only trigger         | implementing agent — **needs maintainer acceptance**        |
| T5  | Outside Tauri the same package is the popup host, origin **and** source pinned                         | implementing agent — **needs maintainer acceptance**        |
| T6  | iOS cannot report `stopUrlReached` on tauri-v2.11.6; the matching code stays, unreachable and labelled | implementing agent — **needs maintainer acceptance**        |
| T7  | The Rust half is in no gate; the TypeScript half is already in `just ci` through `pnpm -r`             | implementing agent — **needs maintainer acceptance**        |
| —   | D1–D9 inherited unchanged from ADR-0021                                                                | maintainer/design, 2026-09-13 and 2026-09-16 — not reopened |

Four things this document deliberately does **not** decide, and
[ADR-0023](../adr/0023-tauri-checkout-plugin.md) § "Left to the maintainer"
carries them: whether to publish to crates.io and npm, whether to put Tauri
toolchains in the CI image, whether to keep the `open` dependency or depend
on `tauri-plugin-opener`, and whether a desktop host should exist at all.
