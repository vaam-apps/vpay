# Tauri checkout (`tauri-plugin-vpay-checkout`)

**What this is.** A Tauri v2 plugin that opens vpay's hosted checkout page —
the same page [hosted-checkout.md](hosted-checkout.md) describes — in the
**payer's own browser** (a partial Custom Tab on Android, a detented
`SFSafariViewController` on iOS, the default browser on desktop), and answers
the merchant's app with a typed result once the payment intent has actually
settled. Its JavaScript half also runs **outside** Tauri, in a plain browser,
where it falls back to a `window.open` popup. That is what "android + ios +
web" means here: one guest-JS package, one state machine, three hosts.

Requested by the maintainer on 2026-09-22, verbatim: _"Please create a SDK
for andoid+ios+web for tauri v2"_ [sic]. Designed in
[`../plans/2026-09-22-tauri-plugin.md`](../plans/2026-09-22-tauri-plugin.md)
(T1–T7), staged in
[`../plans/2026-09-22-tauri-plugin-brief.md`](../plans/2026-09-22-tauri-plugin-brief.md),
decided in [ADR-0023](../adr/0023-tauri-checkout-plugin.md).

**It inherits [ADR-0021](../adr/0021-flutter-checkout-plugin.md)'s D1–D9
unchanged**, so this page does not re-argue them.
[mobile-checkout.md](mobile-checkout.md) is the same process on Flutter and
is the fuller document; where the two agree, that page is the one with the
reasoning. [browser-checkout.md](browser-checkout.md) is the wire surface
under both: the same two publishable-key-and-`client_secret` reads on the
same `/v1/browser` nest.

## The invariant

**A Tauri app is a payer's device, not a merchant's server.** It holds a
publishable key and one session's credentials and nothing else — no merchant
token, no private key, no `Authorization` header, ever.
`POST /v1/checkout/sessions` stays on the merchant's own server exactly as it
does for the web integration; the app is handed only the `url` that call
returned. This is why the plugin gets its own table in
[`../sdks/parity.md`](../sdks/parity.md) beside `sdks/stripe-js` and
`sdks/flutter/vpay_checkout_flutter` — payer surfaces — rather than beside
the merchant SDKs that make that server call.

**The outcome is read from the API, never from a URL.** A navigation to
`success_url` closes nothing and decides nothing. Only a poll of
`GET /v1/browser/payment_intents/{id}` decides, because the merchant controls
`success_url` and a plugin that reported success off a redirect it did not
authenticate would be a hard-coded success wearing a different mechanism.

**No window is ever a WebView.** No `WebView`, `WKWebView` or Tauri
`WebviewWindow` appears in the payment path. Rendering vpay's payment form
inside the merchant app's own process would put `evaluateJavascript`, the
cookie store and a navigation delegate within that app's reach.

## What happens, in order

1. **The merchant's server creates the session.**
   `POST /v1/checkout/sessions` with `ui_mode: "hosted"`, a `success_url` and
   a `cancel_url`, on the secret key. The app never makes this call.
2. **The app is handed `session.url`** — `{base}/c/{cs_id}?key={pk}#{cs_secret}`
   — and calls `new VpayCheckout({ baseUrl, publishableKey }).start(url)`.
3. **Parse.** `start` splits the URL into base, `cs_id`, publishable key and
   the session secret in the **fragment**. A URL with no fragment, or an
   empty one, is refused here — with no network call, and without the URL
   appearing in the error.
4. **Pre-flight, once, before any window opens.**
   `GET /v1/browser/checkout/sessions/{cs_id}?key&client_secret` buys the
   intent id, the intent's own `client_secret` (the polling credential —
   read while the session is `open`, and good for the intent's whole life
   afterwards), `ui_mode`, and `success_url`/`cancel_url`. The stop rules are
   **derived** from those two URLs, with `{CHECKOUT_SESSION_ID}` substituted
   first; the merchant configures no URLs (D2). An `ui_mode: "embedded"`
   session is refused **here**, before any window opens.
5. **Show.** The host is handed
   `{ url, stopUrls, allowInsecureUrl, onEvent }`. The `onEvent` callback is
   wired **before** `show` is awaited, so an outcome reported from inside
   `show` cannot be missed.
6. **The one event.** Each host reports exactly one
   `{ outcome: "dismissed" | "stopUrlReached", reachedUrl }` per `show`. A
   second is ignored. There is no `succeeded`, `canceled` or `failed`
   outcome in the vocabulary at all.
7. **Poll.** `GET /v1/browser/payment_intents/{id}?key&client_secret` on a
   budget — 180 s at 2 s after a stop URL, 5 s at 1 s after a dismissal (D4)
   — until the intent is terminal. Terminal means `succeeded`, `canceled`,
   or `requires_payment_method` **with** a `last_payment_error`; a bare
   `requires_payment_method` keeps polling.
8. **Answer.** `start` resolves one of five kinds and **never rejects**:
   `succeeded`, `failed`, `canceled`, `pending`, `unresolved`. `pending` and
   `unresolved` are not failures and must not be rendered as one.

A `succeeded` here is a UI fact about what the payer's browser and one API
read showed. The merchant's server fulfils on `payment_intent.succeeded` from
its own signature-verified webhook, not on this.

## What can go wrong

| Situation                                                                    | What the payer's app sees                                                                                                                                                                                                                          |
| ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The payer closes the browser sheet having paid                               | `dismissed` → D4's short poll → **`succeeded`**. This is the _ordinary_ end of a successful payment on every host.                                                                                                                                 |
| The payer closes it mid-payment                                              | `dismissed` → the poll's 5 s budget elapses → **`pending`**. Never `canceled` unless the intent itself says `canceled`.                                                                                                                            |
| A deep link matches a stop URL while the intent is still processing          | `stopUrlReached` → the 180 s poll → **`pending`** if it never settles. Never `succeeded` off the URL.                                                                                                                                              |
| The poll comes back with an intent whose `id` is not the pre-flight's        | **`unresolved`**, never `succeeded`. A different intent is not evidence about this one.                                                                                                                                                            |
| The pre-flight 404s (bad link, wrong key, wrong secret, expired)             | **`unresolved`** with the typed error, before any window opens. The uniform 404 is the same for all four causes by design.                                                                                                                         |
| The session is `ui_mode: "embedded"`                                         | **`unresolved`** with `embedded_session_not_supported`, before any window opens.                                                                                                                                                                   |
| The host refuses to open a window (no capability, popup blocked, no browser) | **`unresolved`** with the fixed `platform_window_failed`. The thrown value is never interpolated — a host's message can quote the URL, and the URL's fragment is the session secret.                                                               |
| The window closes without reporting anything                                 | **`unresolved`** (`platform_window_failed`) rather than a hang — as long as the host's `show` rejects or reports. A host that resolves `show` and then never reports at all still hangs `start`; there is no stream-done signal. Named, not fixed. |
| Desktop: the payer closes the browser tab                                    | **Nothing happens.** There is no dismissal signal on desktop at all (T4). The app must call `dismiss()`, which sends `dismissed` and starts the poll.                                                                                              |
| iOS: a deep link arrives                                                     | **Nothing happens.** `stopUrlReached` is unreachable on iOS with tauri-v2.11.6 (T6); the checkout ends `dismissed` when the payer taps Done or swipes away, and D4 makes that correct.                                                             |
| The `baseUrl` is `http://` and no opt-in was named                           | The **constructor** throws, before any network call. `allowInsecureBaseUrl: true` is the named opt-in (D6), and it is forwarded to the host as `allowInsecureUrl` rather than re-derived from the scheme.                                          |

## What holds throughout

- **D1 — the outcome comes from the API.** No code path in the Rust crate,
  the Kotlin host, the Swift host or the two JavaScript hosts reads an
  outcome off a URL, a message payload or an event.
  `resolveAfterStopUrlReached` takes no URL argument at all; the signature is
  the proof.
- **D4 — a dismissal polls before it reports.** The same loop as a reached
  stop URL, on a shorter budget. `pending` is a first-class result.
- **D6 — the credential never reaches a log line.** No `console.*` in
  shipping guest-JS source (a test reads the files). Every error message is a
  fixed string; the only values any of them carry are a public `cs_…` id and
  an HTTP status. The Rust `ShowCheckoutRequest`'s `Debug` is hand-written so
  neither `{:?}` nor `{:#?}` can render the URL.
- **Exactly one event per `show`.** Enforced at both ends: each host reports
  once and drops a second, and the guest-JS controller drives exactly one
  poll however many arrive.

## Per platform

| Host                          | Window                                                      | Dismissal signal                                | `stopUrlReached`                                                       |
| ----------------------------- | ----------------------------------------------------------- | ----------------------------------------------- | ---------------------------------------------------------------------- |
| Android                       | partial Custom Tab, 90 % height, adjustable, close at START | yes — the tab closing                           | wired (App Link → exported forwarding Activity), unverified end to end |
| iOS                           | `SFSafariViewController`, `.pageSheet` + `.large()` detent  | yes — Done **and** swipe-away                   | **unreachable** — see T6                                               |
| Desktop (macOS/Windows/Linux) | the payer's **default browser**, via `open::that_detached`  | **none** — `dismiss()` only                     | not implemented at all                                                 |
| Plain browser (no Tauri)      | `window.open` popup, address bar visible                    | yes — the popup's `closed`, polled every 500 ms | n/a — a cross-origin popup's location is unreadable from the opener    |

`stopUrls` and `allowInsecureUrl` are documented as **inapplicable** in the
plain-browser host rather than silently ignored.

The Android host declares **no `<intent-filter>` of its own**: a hostless
`https` filter would claim every https URL on the device (harmful on API
21-30, inert on 31+), so the merchant declares one for their own verified
host against `dev.vpay.tauri.checkout.VpayCheckoutAppLinkActivity` with
`tools:node="merge"`, and serves an `assetlinks.json` there. The manifest
carries that snippet verbatim.

## App Store and Play

Opening the payment page in the payer's own browser rather than an in-app
WebView is deliberate, and the store-policy reasoning — what Apple 3.1.1,
3.1.1(a) and 3.1.3(e) and Google's equivalents actually say, quoted rather
than paraphrased — is
[mobile-checkout.md § "App Store and Play policy — a constraint on adoption, not a code change"](mobile-checkout.md#app-store-and-play-policy--a-constraint-on-adoption-not-a-code-change).
It applies to this plugin unchanged and is **not** re-quoted here: one copy
of a quoted external rule is what can be kept correct. Read it before
changing which window this plugin opens.

## Status

**2026-09-22 — written, unit-tested, and then driven end to end for real
on two simulators.** This section was rewritten twice on the day it was
written; the strikethroughs below are what it said before each narrowing.
The short version: **two real payments reached `succeeded` through vpay's
own hosted page**, on an iOS Simulator and an Android emulator, against a
running `vpay-server` whose rail is still WireMock. The plugin
exists at
[`sdks/tauri/tauri-plugin-vpay-checkout`](../../sdks/tauri/tauri-plugin-vpay-checkout/):
a Rust crate in its own Cargo workspace, the `@vaam-apps/vpay-tauri-checkout`
guest-JS package, a Kotlin Android host and a Swift iOS host.

**What is verified**, each measurement attributed on
[`../status/verification/2026-09-22-tauri-plugin.md`](../status/verification/2026-09-22-tauri-plugin.md):

- The Rust crate builds, is clippy-clean at `-D warnings`, and its **19**
  unit tests plus **1** doctest pass; `cargo check` succeeds for
  `aarch64-linux-android` and for `aarch64-apple-ios`.
- The guest-JS package typechecks, lints at `--max-warnings 0`, builds, and
  its **71** vitest cases across **8** files pass with **0** skipped.
- The Swift compiles: `swift build` against the iOS 15 simulator SDK and
  `xcodebuild` for a generic iOS Simulator destination both succeed.

**What is not verified, and each of these is a dated ⛔ in
[`../sdks/parity.md`](../sdks/parity.md):**

- **None of the Rust, Kotlin or Swift is in `just ci` or `just verify`**
  (T7). The four `just *-tauri-*` recipes are run by a human. The TypeScript
  half _is_ gated, through `pnpm -r` and the `web` job's existing filter.
- ~~**The Android host was compiled by none of the lanes that wrote it**~~ —
  **closed the same day** by
  [`examples/tauri-checkout`](../../examples/tauri-checkout/): the Kotlin
  compiles, its classes are in a built APK, and both Activities are in that
  APK's own merged manifest with the expected `exported` values. What stands:
  **no gate and no recipe here builds it**, and only a consuming app's Gradle
  project ever can.
- **`stopUrlReached` has never fired on any platform.** On iOS it cannot
  (T6); on desktop it is not implemented; on Android it is wired and needs an
  `assetlinks.json` this repository cannot serve.
- **Desktop has no dismissal signal** and `open::that_detached` has never
  executed — the eleven desktop tests drive a crate-private opener seam.
- ~~**Nothing has run against a running vpay**~~ — **closed 2026-09-22,
  later the same day (Lane D2):** `examples/tauri-checkout` drove the plugin
  against a real, running `vpay-server` from an iOS Simulator and an Android
  emulator, minting three real sessions and paying two through to
  `succeeded`, each cross-checked with a merchant-token
  `GET /v1/payment_intents/{id}`. Three caveats: the stack was
  `ghcr.io/vaam-apps/vpay-*:edge` images, **not a build of this tree**; no
  shop ran, so **no merchant webhook was verified**; and the plugin's own
  suites still open no socket. (Lane D's earlier attempt was the one that
  had to point at an unreachable host, because `just demo-up` exhausted this
  host's disk and wedged Docker; Lane D2 avoided that by pulling published
  images instead of building any.)
- **Nothing against a real rail, and that half did not move.** Every rail in
  every stack this repository has ever driven is WireMock
  (`../status.md`'s banner), Lane D2's included. No MTN endpoint was called
  and no money moved; Orange Money and the failure MSISDNs were not
  exercised at all.
- ~~**No device, emulator or simulator run of any kind.**~~ **Narrowed
  twice the same day**: a launch (Lane D), then a **full checkout** on both
  a headless Android emulator and an iPhone 17 iOS Simulator (Lane D2). No
  **physical device**, no desktop checkout, no release build, no Windows or
  Linux desktop build.
- ~~**No browser sheet has ever opened on any platform**~~ — **closed for
  iOS and Android.** Both the `SFSafariViewController` and the Custom Tab
  opened on vpay's real hosted page, `show`/`dismiss` crossed the IPC
  boundary into the Swift and the Kotlin, and the order described further up
  this page ran end to end for real. **Desktop is the one host this still
  describes:** `open::that_detached` has never executed.
- **The Android Custom Tab presented full-height, not partial.** Chrome
  declined the 90 % sheet the plugin requests via
  `setInitialActivityHeightPx`; the request does reach the platform, and
  honouring it is the browser's choice. Every "partial Custom Tab" on this
  page is describing a request.
- **No window has ever closed itself.** Both successful checkouts ended
  because a human closed the sheet and the poll then decided — correct by
  D1/D4, and the reason an unfired `stopUrlReached` costs latency rather
  than correctness.

Fuller accounting:
[`../status/mobile-tauri-plugin.md`](../status/mobile-tauri-plugin.md).
