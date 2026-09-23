# Tauri v2 checkout plugin — implementation brief

- **Date:** 2026-09-22
- **Requested by the maintainer**, verbatim: _"Please create a SDK for
  andoid+ios+web for tauri v2"_ [sic], then: _"use opus sub agents to fan out the
  biggest work and you, play only the orchestrator role"_.
- **What this is:** the frozen contract between the lanes that build the
  plugin in parallel. It is the Tauri counterpart of
  [`2026-09-13-flutter-plugin-brief.md`](2026-09-13-flutter-plugin-brief.md),
  and it inherits every decision of
  [`2026-09-13-flutter-plugin.md`](2026-09-13-flutter-plugin.md) (D1–D9) and
  [ADR-0021](../adr/0021-flutter-checkout-plugin.md) unchanged, because the
  thing being built is the same payer surface on a different host framework.
- **Base:** `master` at `7134ecb`.

## What is being built, in one paragraph

A Tauri v2 **plugin** — `tauri-plugin-vpay-checkout` — that opens vpay's
hosted checkout page in the **payer's own browser** (a partial Custom Tab on
Android, a detented `SFSafariViewController` on iOS, the default browser on
desktop) and answers a typed result once the payment intent actually settles,
by polling `GET /v1/browser/payment_intents/{id}` — never by reading a URL.
Its JavaScript API also runs **outside Tauri**, in a plain browser, where it
falls back to a `window.open` popup with the same origin-and-source-pinned
`vpay:complete` protocol `@vaam-apps/vpay-stripe-js` already speaks. That is
what "android + ios + web" means here: one guest-JS package, one state
machine, three hosts.

## Where it lives

```text
sdks/tauri/tauri-plugin-vpay-checkout/
  Cargo.toml            # its OWN [workspace] — NOT a member of the root workspace
  build.rs              # tauri_build::plugin: COMMANDS = ["show", "dismiss"], android + ios paths
  src/lib.rs            # tauri::plugin::Builder, the two commands, init()
  src/models.rs         # ShowCheckoutRequest, CheckoutStopUrl, CheckoutWindowEvent, CheckoutWindowOutcome
  src/error.rs          # thiserror Error, serde Serialize → string message
  src/desktop.rs        # default browser via the `open` crate; no dismissal signal
  src/mobile.rs         # PluginHandle::run_mobile_plugin("show"/"dismiss")
  src/commands.rs       # #[tauri::command] show / dismiss
  permissions/default.toml
  android/              # Kotlin: VpayCheckoutPlugin (@TauriPlugin), VpayCheckoutActivity, VpayCheckoutAppLinkActivity
  ios/                  # SwiftPM: Package.swift, Sources/VpayCheckoutPlugin/*.swift
  guest-js/             # TypeScript: the state machine, both hosts, tests
  package.json          # @vaam-apps/vpay-tauri-checkout, builds guest-js → dist/
  tsconfig.json, tsconfig.build.json, vitest.config.ts, eslint.config.js
  README.md
examples/tauri-checkout/   # a real Tauri app that consumes the plugin by path (Lane D)
```

`pnpm-workspace.yaml` gains `"sdks/tauri/*"`. The root `Cargo.toml` is **not**
touched: the plugin's manifest carries an empty `[workspace]` table so cargo
treats it as its own root, exactly so that `tauri`'s dependency graph (wry,
tao, webkit2gtk on Linux) never enters `Cargo.lock`, `cargo deny`,
`verify-no-mocks`'s `cargo metadata` or `just ci`'s `cargo nextest --workspace`.

## Names, fixed

| Thing                                                  | Name                                                                                                                              |
| ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------- |
| Rust crate                                             | `tauri-plugin-vpay-checkout`, `version = "0.4.0" # x-release-please-version`, `publish = false` (with a reason)                   |
| Tauri plugin identifier                                | `vpay-checkout` → commands `plugin:vpay-checkout\|show`, `plugin:vpay-checkout\|dismiss`                                          |
| Kotlin package / class                                 | `dev.vpay.tauri.checkout` / `VpayCheckoutPlugin`, `VpayCheckoutActivity`, `VpayCheckoutAppLinkActivity`                           |
| Swift package / class                                  | `tauri-plugin-vpay-checkout` / `VpayCheckoutPlugin`, C entry `init_plugin_vpay_checkout`                                          |
| npm package                                            | `@vaam-apps/vpay-tauri-checkout`, `"version": "0.4.0"`, `main`/`types` under `dist/`, `files: ["dist","README.md"]`               |
| Guest-JS entry class                                   | `VpayCheckout` with `start(sessionUrl)` and `dismiss()`                                                                           |
| Pinned versions (crates.io / npm, resolved 2026-09-22) | `tauri = "2.11.6"`, `tauri-build = "2.6.3"`, `tauri-plugin = "2.6.3"`, `@tauri-apps/api = "2.11.1"`, `@tauri-apps/cli = "2.11.5"` |

Rust `serde` on this crate's wire is `#[serde(rename_all = "camelCase")]`:
it is the plugin's **JS** wire, not vpay's `/v1`, and `verify-serde` scans
`backends/crates` only. Do not spell a `…Resource` type in Rust or TypeScript
anywhere in `sdks/tauri/` — `verify-sdk-parity` enumerates `impl …Resource`
blocks and `class …Resource` as merchant capabilities and would demand rows.

## The wire contract between guest-JS and every host

**`show`** — arguments (camelCase, exactly these keys):

```jsonc
{
  "url": "https://checkout.vpay.example/c/cs_…?key=pk_…#cs_…_secret_…", // the session's own hosted url; the host loads exactly this
  "stopUrls": [{ "scheme": "https", "host": "shop.example", "port": 443, "path": "/return" }],
  "allowInsecureUrl": false, // D6's named opt-in, forwarded so a host never re-derives it from the scheme
  "onEvent": "<Tauri Channel>" // tauri::ipc::Channel<CheckoutWindowEvent>; Kotlin app.tauri.plugin.Channel; Swift Tauri.Channel
}
```

Resolves with no value once the window is **showing**. Rejects with a
string message on: a window already open (`already_open`), no Activity /
presenter (`no_activity` / `no_presenter`), an unparseable URL
(`invalid_url`). Guest-JS maps **every** rejection to the fixed
`platform_window_failed` error and never interpolates the thrown value —
it can quote the URL, and the URL's fragment is the session secret (D6).

**`dismiss`** — no arguments, resolves with no value. Closes the window if
one is open; a no-op otherwise.

**The one event**, sent on `onEvent` **exactly once per `show`**:

```jsonc
{ "outcome": "dismissed" | "stopUrlReached", "reachedUrl": "https://…" | null }
```

- `dismissed` — the payer closed the browser sheet (Done, back, swipe) with
  no matching deep link having arrived. Since the browser cutover this is the
  **ordinary** end of a successful payment too, which is why D4 polls.
- `stopUrlReached` — an **incoming deep link** (Android App Link, iOS
  Universal Link) matched one of `stopUrls` on scheme+host+port+path, query
  and fragment ignored (D2). `reachedUrl` is diagnostics only; guest-JS reads
  nothing off it (D1). Unverified end to end on every platform for the same
  reason the Flutter plugin's is: this repository serves no
  `assetlinks.json` / `apple-app-site-association`.

Never a `succeeded` / `canceled` / `failed` outcome. The host decides
nothing about money (D1).

**Desktop (macOS/Windows/Linux) host:** opens `url` in the default browser
through the `open` crate and has **no dismissal signal** — exactly the
Flutter macOS host's situation. It sends `dismissed` only when `dismiss()`
is called while a show is in flight, and it says so in its doc comment
rather than inventing a focus-based signal.

## The guest-JS package

Port of `sdks/flutter/vpay_checkout_flutter/lib/src/{vpay_checkout,checkout_controller,result,errors,redaction}.dart`
to TypeScript, under `guest-js/`. It **reuses `@vaam-apps/vpay-stripe-js`**
(`workspace:*`) for the two reads — `loadStripe(pk, { baseUrl, fetch })`, then
`retrieveCheckoutSession` for the pre-flight and `retrievePaymentIntent` for
the poll — rather than porting `browser_client.dart`. Two facts about that
package to build on: its `isCheckoutSession` checks only `object` and `id`,
so a real server answer decodes; and its `CheckoutSession.client_secret` is
typed `string` but the server **never sends it back** (the Flutter e2e suite
found this on 2026-09-14) — read the intent's credential from
`session.payment_intent.client_secret`, and never touch
`session.client_secret`. `loadStripe` does not refuse `http://`; this package
does (D6), before calling it, unless `allowInsecureBaseUrl: true` is passed.

Public surface (`guest-js/index.ts`):

```ts
export interface VpayCheckoutOptions {
  baseUrl: string;                 // the API origin
  publishableKey: string;
  allowInsecureBaseUrl?: boolean;  // D6: demo stack only, never inferred
  fetch?: typeof fetch;            // injected, for tests
  host?: CheckoutHost;             // defaults to defaultHost(): tauriHost() inside Tauri, webHost() otherwise
  clock?: VpayClock;               // injected, for tests: now() and delay(ms)
}
export class VpayCheckout {
  constructor(options: VpayCheckoutOptions);
  start(sessionUrl: string): Promise<VpayCheckoutResult>;   // parse, pre-flight, show, watch, resolve — never rejects
  dismiss(): Promise<void>;
}
export type VpayCheckoutResult =
  | { kind: "succeeded"; sessionId: string; paymentIntentId: string }
  | { kind: "failed"; sessionId: string; paymentIntentId: string; code: string | null; providerMessage: string | null }
  | { kind: "canceled"; sessionId: string; paymentIntentId: string }
  | { kind: "pending"; sessionId: string; paymentIntentId: string }
  | { kind: "unresolved"; sessionId: string; paymentIntentId: string; error: VpayError };
export interface VpayError { type: string; code?: string; message?: string; param?: string } // StripeError's shape, re-exported
export interface CheckoutStopUrl { scheme: string; host: string; port: number; path: string }
export interface ShowCheckoutRequest { url: string; stopUrls: CheckoutStopUrl[]; allowInsecureUrl: boolean }
export type CheckoutWindowOutcome = "dismissed" | "stopUrlReached";
export interface CheckoutWindowEvent { outcome: CheckoutWindowOutcome; reachedUrl: string | null }
export interface CheckoutHost {
  show(request: ShowCheckoutRequest, onEvent: (event: CheckoutWindowEvent) => void): Promise<void>;
  dismiss(): Promise<void>;
}
export function tauriHost(deps?: { invoke?: typeof invoke; Channel?: typeof Channel }): CheckoutHost;
export function webHost(win?: Window): CheckoutHost;
export function defaultHost(): CheckoutHost;   // isTauri() ? tauriHost() : webHost()
export const VPAY_CLIENT_ERROR_CODES: { pollingTimeout, unexpectedResponse, embeddedSessionNotSupported, platformWindowFailed, invalidRequest };
export function redacted(value: string | null | undefined): string;  // "[N chars redacted]" / "null"
```

Rules the port must keep, each as a test whose name a parity cell can cite:

1. **D1** — `resolveAfterStopUrlReached` takes no URL. The outcome comes
   from the poll only. An intent whose `id` differs from the pre-flight's is
   `unresolved`, never `succeeded`.
2. **D2** — one pre-flight read; `{CHECKOUT_SESSION_ID}` substituted before a
   configured URL becomes a stop rule; match on scheme+host+port+path with
   default ports normalised; `ui_mode: "embedded"` refused **before** any
   window opens with `embedded_session_not_supported`.
3. **D4** — a dismissal runs the **same** poll loop on a 5 s / 1 s budget
   (stop URL: 180 s / 2 s) and answers `pending` when the budget elapses,
   never `canceled` unless the intent says so.
4. **Terminal rule** — `succeeded`, `canceled`, or `requires_payment_method`
   **with** `last_payment_error`. Bare `requires_payment_method` keeps polling.
5. **D6** — no `console.*` in shipping source (a test reads `guest-js/*.ts`
   the way `sdks/stripe-js` and the Flutter `no_logging_test.dart` do); no
   error message ever interpolates a URL, a secret or a thrown value; every
   host rejection becomes the fixed `platform_window_failed`.
6. **The event cannot be lost** — the `onEvent` callback is wired **before**
   `show` is awaited; a host that never reports resolves `unresolved`
   (`platform_window_failed`) rather than hanging when its promise rejects
   or its window closes without an event.
7. **Web host** — `window.open(url, "vpay-checkout", "popup=yes,location=yes,…")`;
   accepts a `{type:"vpay:complete"}` message only when `event.origin` is the
   page's own origin **and** `event.source` is the exact popup it opened;
   polls `closed` every 500 ms; a refused popup rejects `show`, which guest-JS
   maps to `platform_window_failed`. `stopUrls` and `allowInsecureUrl` are
   documented as inapplicable there, for the reasons `web_checkout_platform.dart`
   gives.
8. **Tauri host** — `invoke("plugin:vpay-checkout|show", { url, stopUrls,
allowInsecureUrl, onEvent: channel })` with `@tauri-apps/api/core`'s
   `Channel`; `channel.onmessage` forwards exactly one event and ignores any
   second.

Tooling mirrors `sdks/stripe-js`: TS strict from `tsconfig.base.json`, target
ES2020 + DOM, `eslint.config.js` is the three-line `vpayEslintConfig` call,
vitest on Node with injected `fetch`/`invoke`/`Window` (no jsdom unless a test
is about a real DOM event), `"scripts": { build, prepack, typecheck, test, lint }`
so `pnpm -r typecheck|lint|test` — and therefore `just ci`'s `web` half — run
it without any recipe change. `verify-npm-scope` will read this manifest:
`publishConfig.access: "public"`, `license`, `repository`/`homepage` naming
`github.com/vaam-apps/vpay`, `files` naming `dist`, a `prepack` that builds.

## The Android host

A port of `sdks/flutter/vpay_checkout_flutter/android/` onto Tauri's Android
plugin API (`app.tauri.annotation.TauriPlugin`, `app.tauri.plugin.Plugin`,
`Invoke`, `Channel`, `startActivityForResult` + `@ActivityCallback`). Keep
every security decision of the Flutter files, and their comments' reasoning:

- `VpayCheckoutActivity` is `android:exported="false"`, launches a **partial**
  Custom Tab (`androidx.browser:browser:1.8.0`, `setInitialActivityHeightPx`
  at 90 % of the display, `ACTIVITY_HEIGHT_ADJUSTABLE`, 16 dp corners, close
  button at START), removes the URL extra the moment it has read it, reports
  exactly one result (`reported` guard), treats a **second** `onResume` as the
  tab closing, persists `customTabLaunched` across recreation, and finishes
  as `dismissed` when no browser can open the URL.
- `VpayCheckoutAppLinkActivity` is the one exported component, ships **no**
  `<intent-filter>`, holds no state, forwards `ACTION_VIEW` data to the open
  Activity and finishes. The manifest comment carries the merchant's
  `tools:node="merge"` snippet and the API 21-30 / 31+ reasoning.
- `minSdk` 21 (D-M4). `<queries>` for the Custom Tabs service action.
- The plugin class maps `show` → build intent, `startActivityForResult`;
  the `@ActivityCallback` reads the result extras and sends the one event on
  the invoke's `Channel`; `dismiss` → `VpayCheckoutActivity.dismissIfOpen()`.
  Reject `show` with `already_open` when a window is open.

Nothing here decides an outcome (D1). No `WebView` anywhere.

## The iOS host

A port of `sdks/flutter/vpay_checkout_flutter/ios/` onto Tauri's Swift plugin
API (`Tauri` package: `Plugin`, `Invoke`, `Channel`, `@objc` commands,
`@_cdecl("init_plugin_vpay_checkout")`). `SFSafariViewController`, `.pageSheet`
with `[.large()]` detent and a visible grabber, `SFSafariViewControllerDelegate`
for Done **and** `UIAdaptivePresentationControllerDelegate` for the swipe-away
(the 2026-09-17 finding), one event per show, `dismiss` closes the sheet,
`already_open` / `no_presenter` / `invalid_url` rejections. Universal-Link
intake: implement it only through a hook Tauri's iOS `Plugin` base class
actually exposes, and state in the file header what was verified against
the upstream source and what was not. iOS floor **15.0** (Swift Concurrency,
and what Tauri 2 itself requires). Everything UIKit runs on the main actor.

## The example app (Lane D)

`examples/tauri-checkout/`: the smallest real Tauri v2 app (Vite + vanilla
TS front end, `src-tauri/` with its own `[workspace]`, path dependency on the
plugin, `@tauri-apps/cli` as a devDependency) whose one screen takes a base
URL, a publishable key and a session URL, calls `new VpayCheckout(...).start(url)`
and renders the `kind`. Its purpose is **evidence**: `cargo build` on desktop,
`cargo tauri android build --debug` (Android SDK at
`/opt/homebrew/share/android-commandlinetools`, NDK 28.2.13676358, JDK 21) and
`cargo tauri ios build --debug` (Xcode 26.2, CocoaPods present) each exiting 0
is the only proof that the Kotlin and Swift compile; a run on the iOS
Simulator or an emulator is better still and is recorded as exactly what it
was. `src-tauri/gen/` and other generated trees are gitignored and
prettier-ignored.

## What no lane may do

- Return a plausible success anywhere: an unbuilt path is a typed error or a
  dated ⛔ in `docs/sdks/parity.md`, never a `TODO` that resolves `succeeded`.
- Read an outcome off a URL, a message payload, or an event (D1).
- Put a `WebView`/`WKWebView`/Tauri `WebviewWindow` in the payment path (D5 rev. 2026-09-16).
- Log, `toString`, or interpolate a session URL, a session secret or an intent
  secret (D6).
- Add the plugin crate to the root Cargo workspace, or `tauri` to `Cargo.lock`.
- Name a test in a ✅ cell that does not exist under `sdks/tauri/tauri-plugin-vpay-checkout`
  as a Rust `#[test]` or a vitest `it("…")`/`test("…")`, or that is skipped.
- Claim a compile, a run or a device the lane did not actually perform.

## Lanes

| Lane | Owns                                                                                                                   | Depends on |
| ---- | ---------------------------------------------------------------------------------------------------------------------- | ---------- |
| A1   | The Rust crate: manifest, `build.rs`, `src/`, `permissions/`, desktop host, Rust unit tests, `cargo clippy` clean      | —          |
| A2   | `guest-js/`, `package.json` and TS tooling, vitest suite, README                                                       | —          |
| B    | `android/`                                                                                                             | —          |
| C    | `ios/`                                                                                                                 | —          |
| D    | `examples/tauri-checkout/`, the three builds, the evidence page draft                                                  | A1 A2 B C  |
| E    | Docs: design + ADR-0023 + flow page + parity table + status pages + justfile + release-please + CI filters + skills PR | A–D        |

---

## What the lanes actually did — appended 2026-09-22, the contract above unchanged

This brief is the frozen contract and is **not** rewritten. What follows is
the one-paragraph index a reader needs so that a deviation is found here
rather than discovered in the code.

The plugin was built. Every deviation from the contract above — the four
flat `show` arguments instead of one `request` object, the extra
`insecure_url` / `invalid_arguments` refusal tokens, `Error::PluginInvoke`
being deliberately transparent, the Android host's two-guard `already_open`
and missing `no_activity` branch, and the iOS host's **absent** Universal-Link
intake — is recorded with its reason in
[`2026-09-22-tauri-plugin.md`](2026-09-22-tauri-plugin.md) (the design
record, T1–T7), [`../adr/0023-tauri-checkout-plugin.md`](../adr/0023-tauri-checkout-plugin.md)
(the decisions, Proposed) and
[`../status/verification/2026-09-22-tauri-plugin.md`](../status/verification/2026-09-22-tauri-plugin.md)
(every command, every exit code, and who measured it).

**The one deviation that changes what this contract's wire means in
practice:** `stopUrlReached` **cannot be produced on iOS** at
tauri-v2.11.6 — the Swift `Plugin` base class exposes no app-lifecycle hook
and Tauri's own `deep-link` plugin ships no `ios/` directory, handling iOS in
Rust via `RunEvent::Opened`. The section "The wire contract between guest-JS
and every host" above reads as though three hosts implement both outcomes;
two do. Every iOS checkout ends `dismissed`, which D1/D4 make correct, and it
is a dated ⛔ in [`../sdks/parity.md`](../sdks/parity.md).

**~~Two~~ Three sentences in the contract above are wrong as written,
corrected here — two on 2026-09-22, the third on 2026-09-23 — rather than
edited in place.**

1. **"Pinned versions" (the Names table) lists `tauri-build = "2.6.3"` among
   this plugin's pins. The plugin does not depend on `tauri-build` at all.**
   `tauri-build` is the **consuming application's** build dependency —
   `examples/tauri-checkout/src-tauri` has it, this crate does not. What the
   plugin carries is `tauri-plugin = { version = "2.6.3", features =
["build"] }` as its one `[build-dependencies]` entry, which is what
   generates `permissions/autogenerated/`, the permission schema and the
   `mobile`/`desktop` cfg aliases. Recorded by Lane A1; the `2.6.3` figure
   itself is right, it is attached to the wrong crate. The runtime pin is
   `tauri = "2.11"`, a caret rather than the table's exact `2.11.6`, for the
   reason the manifest's own comment gives: an app pins one `tauri` for the
   whole binary, and an exact pin here would make the plugin unusable in any
   app one patch ahead.
2. **"The iOS host" says the 15.0 floor is "Swift Concurrency, and what
   Tauri 2 itself requires". The second half does not hold**, and it matters
   because it implies the toolchain enforces the floor. `ios/Package.swift`
   declares `platforms: [.iOS(.v15)]`, and **nothing in the plugin's own
   build reads that manifest**: `cargo check --target aarch64-apple-ios` goes
   through `tauri_utils::build::link_apple_library` →
   `swift_rs::SwiftLinker::with_ios(…)`, which reads
   `IPHONEOS_DEPLOYMENT_TARGET` and **defaults to `"13.0"`** (confirmed from
   `cargo check -vv`, Lane C follow-up). So the sources must still compile at
   `-target arm64-apple-ios13.0`, and the one iOS-15-only API —
   `sheetPresentationController`'s detents — is reached only inside an
   `if #available(iOS 15.0, *)` guard. The 15.0 floor is a `Package.swift`
   declaration plus that guard, enforced by nothing else in the build; before
   the guard existed, a bare `cargo check --target aarch64-apple-ios` exited
   **101**. `Package.swift`'s own "The two version floors" comment carries
   this at length.
3. **The Names table above spells `0.4.0` twice — the crate's
   `version = "0.4.0" # x-release-please-version` and the npm package's
   `"version": "0.4.0"`. Both manifests are at `0.4.1` today, and the table
   is deliberately left alone.** Added 2026-09-23. The contract's intent was
   "whatever version master is on, carried by release-please the same way
   every other first-party manifest is" — the load-bearing parts of those
   two rows are the `# x-release-please-version` annotation and the
   `$.version` JSON path, not the literal. What aged is the literal. The
   branch this brief drove forked at `7134ecb`, one commit before
   release-please's `0.4.0` → `0.4.1` bump (#236) landed, so both manifests
   were born at the version master had just left; vpay #240 (`078fa3d`)
   moved them to `0.4.1` and `cargo xtask verify-versions` now prints
   `24 version references all say 0.4.1`. A frozen contract naming a version
   literal will always age this way, and the correction belongs here rather
   than in the table for exactly that reason.

Lane D's example app and the real Android/iOS builds were still in flight
when this note was written; the verification page's final section is where
their result belongs.
