# Mobile checkout (`vpay_checkout_flutter`)

**What this is.** A Flutter plugin that opens vpay's hosted checkout page — the
same page [hosted-checkout.md](hosted-checkout.md) describes — in the payer's
own browser on Android, iOS, macOS and web, and answers the merchant's app
with a typed result once the payment is actually decided. **Revised
2026-09-16**: there is no in-app WebView any more, on any platform — a
partial (bottom-sheet) Custom Tab on Android, `SFSafariViewController` on
iOS, the default browser via `NSWorkspace` on macOS, the same `window.open`
popup on web it always used. See the Status section's final entry for why
and what it costs. Requested by the
maintainer on 2026-09-13, verbatim: _"Let's develop a custom vpay flutter
plugin. We'll do it by using a new Activity (android), UIViewController
(iOS/…) and similar for web. We'll do the payment activity in the mobile
window and call the hosted page."_ Designed in
[`../plans/2026-09-13-flutter-plugin.md`](../plans/2026-09-13-flutter-plugin.md)
(decisions D1–D9, D-M1–D-M6) and staged for implementation in
[`../plans/2026-09-13-flutter-plugin-brief.md`](../plans/2026-09-13-flutter-plugin-brief.md)
(Lane B: this document, the parity gate, the recipes, the version pin — Lane
A: the Dart core — Lane C: the platform hosts).

This document is the process; the design doc above is where every decision's
reasoning and alternatives live and is not restated here except as a quote.
[browser-checkout.md](browser-checkout.md) is the wire surface underneath both:
the plugin speaks the same two publishable-key-and-`client_secret` routes a
browser does, over the same `/v1/browser` nest.

## The invariant

**A Flutter app is a payer's device, not a merchant's server.** It holds a
publishable key and one session's credentials and nothing else — no merchant
token, no private key, no `Authorization` header, ever.
`POST /v1/checkout/sessions` stays on the merchant's own server exactly as it
does for the web integration; the plugin is handed only the `url` that call
returned. This is why the plugin belongs in the parity matrix beside
`sdks/stripe-js` — a payer surface — rather than beside `sdks/rust` and
`sdks/nodejs`, the merchant SDKs that make that server call.

**The outcome is read from the API, never from a URL.** A navigation to
`success_url` closes the window; it does not decide anything. Only a poll of
the payment intent (`GET /v1/browser/payment_intents/{id}`) decides, because
the merchant controls `success_url` and a plugin that reports success off a
redirect it did not authenticate is exactly the failure mode this repository's
own rules exist to prevent — "filling an unimplemented function with something
plausible that returns a hard-coded success" in spirit, wearing a different
mechanism.

## What happens, in order

1. **Pre-flight.** The plugin calls
   `GET /v1/browser/checkout/sessions/{cs_id}?key&client_secret` once, before
   any window opens. This buys the intent id and its own `client_secret` (the
   polling credential — read while the session is `open`, which is the only
   time that read is served, and good for the intent's whole life thereafter),
   the session's own `success_url`/`cancel_url` (so the stop list is derived
   from the session, never configured by the app), the `ui_mode` (an
   `embedded` session is **refused here**, before any window opens, with a
   typed error — D2), and a fail-fast on a bad or expired link.
2. **Substitute and normalise the stop URLs.** `{CHECKOUT_SESSION_ID}` is
   replaced in both URLs the same way vpay itself replaces it when forwarding.
   A navigation matches a stop URL on **scheme, host, port and path only** —
   query and fragment are ignored — so a merchant's own tracking parameters on
   `success_url` do not break the match.
3. **Open the payer's own browser** on the platform host — a partial
   (bottom-sheet) Custom Tab on Android, `SFSafariViewController` on iOS,
   the default browser via `NSWorkspace` on macOS, or a `window.open` popup
   on web (Lane C) — loading `session.url` from `POST /v1/checkout/sessions`.
   **Since D5's 2026-09-16 revision there is no in-app WebView on any
   platform** (see the Status section's final entry), so the host cannot
   watch a navigation any more; its only job is to show the page and report
   two things back over a typed channel: "an incoming deep link matched one
   of these stop URLs" (unverified on every platform as of this revision) or
   "the payer left."
4. **Resolve.** On a reached stop URL, or on dismissal, the plugin polls the
   payment intent — on a budget, and (D4) **before** reporting anything for a
   dismissal, never immediately as "canceled." A dismissed sheet fifteen
   seconds after the payer approved an MTN push is not a cancellation; it is a
   payer who has not been asked yet.
5. **Answer.** One of `VpayCheckoutSucceeded`, `VpayCheckoutFailed`,
   `VpayCheckoutCanceled`, `VpayCheckoutPending` (still processing — not a
   failure, and not lied about as one) or `VpayCheckoutUnresolved` (the plugin
   itself could not decide — a typed `VpayError`, never a silent guess).

## What can go wrong

- **The rail refuses an embedded WebView.** **Moot as of D5's 2026-09-16
  revision: there is no WebView left, on any platform, to refuse an embed
  into.** This was the single biggest technical risk named in the original
  design — Orange's page had never been driven inside a WebView, and vpay
  had only ever talked to a WireMock stub serving two links. It is retired
  by construction, not proven safe: the page now always runs in the payer's
  own browser (a partial Custom Tab on Android, `SFSafariViewController` on
  iOS, the default browser on macOS, the existing popup on web), the same
  surface `VpayCheckoutMode.externalBrowser` (D8) used to offer as one of
  two options before `VpayCheckoutMode`/`CheckoutWindowMode` were deleted
  outright. Android's Custom Tab and iOS's `SFSafariViewController` are the
  same code D8 built 2026-09-14 (`Application.ActivityLifecycleCallbacks`
  for the return signal — no `startActivityForResult`, no scheme to call
  back on, because the outcome is always polled, D1) — repurposed as the
  only path. **What this costs**: no host on any platform can watch a
  navigation any more, so a stop URL only ever arrives as an incoming deep
  link (Android App Link / iOS or macOS Universal Link), and that signal is
  **unverified on every platform** as of 2026-09-16 — it needs a
  merchant-hosted `assetlinks.json`/`apple-app-site-association` deployment
  this repository cannot provide or prove against. Every checkout today
  therefore ends as `dismissed`, never `stopUrlReached`, and D1/D4's poll is
  what makes that correct regardless. Android and the example app **compile**
  (`BUILD SUCCESSFUL`) but this cutover has **not** been run on an emulator;
  it has been run — not merely compiled — on a real iOS Simulator, for the
  first time (see the Status section's final entry). See the dated rows in
  [`../sdks/parity.md`](../sdks/parity.md).
- **The payer dismisses the window mid-flow.** Handled by D4's short poll
  before reporting anything (above); `VpayCheckoutPending` exists precisely so
  the plugin never has to choose between lying and throwing.
- **A merchant configures an `embedded` session.** Refused at the pre-flight,
  before any window opens, with a typed error — not a WebView that renders and
  then refuses itself the way the page does when it is framed wrong.
- **A publishable key, session id or `client_secret` is wrong or expired.**
  Mapped to the same uniform 404 `browser::authenticate` already renders for
  the six causes it does not distinguish between — the confidentiality
  property this plugin inherits depends on the Dart client not trying to tell
  them apart either.
- **A custom URL scheme return, or a bounce page to one.** Recorded here as a
  decision, not an oversight: `checked_forward_url`
  (`vpay-api/src/v1/checkout_sessions.rs:740`) accepts only `http://`/`https://`
  for `success_url`/`cancel_url`, so a bounce page redirecting to `myapp://…`
  is the only way to get one, and a custom scheme is first-come-first-served on
  Android — any installed app may claim it and receive the return. Android App
  Links and iOS Associated Domains (both domain-verified, tier 1 in the
  design) are the upgrade path; tier 0 — nothing configured, the plugin polls
  on resume — is what this plugin's own tests can prove, and tier 1 is
  merchant deployment work no test here can verify.

## What holds throughout

- No merchant credential and no long-lived bearer token is ever held by the
  app — a publishable key and one session's own short-lived secrets, same as
  a browser.
- No JavaScript bridge and no native peer is added to the checkout page
  (D3): `addJavascriptInterface`/`WKScriptMessageHandler` were never used
  even when the page ran inside this plugin's own WebView, and since D5's
  2026-09-16 revision the page runs in the payer's own browser process
  instead, where this plugin's code cannot reach it at all — a stronger
  version of the same property, not a restatement of it (see the Status
  section's final entry for the security reasoning).
- A session credential never reaches a log line, a generated `toString`, or
  an Android extra/iOS property that outlives the window (D6) — a parity row
  in its own right, not a promise; see
  [`../sdks/parity.md`](../sdks/parity.md) once Lane A lands it.
- `onReceivedSslError` was never overridden and `decidePolicyFor` never
  bypassed certificate validation while this plugin still rendered the page
  itself — an SDK that proceeds through a certificate error on a payment
  page would have been the single worst thing this plugin could ship (D5).
  **Moot as of 2026-09-16**: the plugin no longer renders the page or
  handles its TLS at all — the payer's own browser does, with its own
  certificate handling this plugin cannot override even if it wanted to.

## App Store and Play policy — a constraint on adoption, not a code change

Raised by the maintainer on 2026-09-13: _"won't apple flag us? Even if we're
using a browser, technically we're still in app right?"_ — checked against the
live guidelines rather than answered from memory (design doc D9; the numbering
had already moved — it is 3.1.3(e), not the 3.1.5(a) it was for years).

**The rule keys on what is sold, not on where the payment UI lives.** The
former in-app `WebView` and the browser surface that replaced it (D5, revised
2026-09-16) are treated identically by both stores; neither is a way around
anything.

**vpay's own merchants are on the permitted side, and not by a carve-out.**
Apple's App Store Review Guidelines, **3.1.3(e)**, verbatim:

> If your app enables people to purchase physical goods or services that will
> be consumed outside of the app, you must use purchase methods other than
> in-app purchase to collect those payments, such as Apple Pay or traditional
> credit card entry.

A shop selling physical goods, or a service consumed in the real world, paid
with MTN MoMo or Orange Money through this plugin, is required to do exactly
what this plugin does — and would be **rejected** for using Apple's own In-App
Purchase instead. Google Play is the same shape from the other direction: Play
Billing is for digital items only, and physical goods and physical services
are not supported by it.

**Where a merchant is flagged, and no architecture here saves them.** Apple's
**3.1.1** requires In-App Purchase to unlock features or functionality
_within_ the app — subscriptions, in-app credits, game levels, premium
content, full-version unlocks, digital gift cards or vouchers redeemable for
digital goods. A vpay checkout for any of those is a rejection, whichever
surface renders it.

**The trap: opening the payer's browser does not move a digital-goods app
out of 3.1.1.** Apple's **3.1.1(a)** goes further: outside the United States
storefront, an app and its metadata "may not include buttons, external links,
or other calls to action that direct customers to purchasing mechanisms other
than in-app purchase" without a StoreKit External Purchase Link Entitlement,
which is region-limited. For a digital-goods merchant, this plugin opening a
browser at all — the only thing it does since D5's 2026-09-16 revision — is
not neutral: it can itself be the violation. The plugin's own README must
document this constraint in the same breath it documents what the plugin
does, so it does not read as a workaround.

**Bounds on the above.** This is a reading of the published guidelines on
2026-09-13, not legal advice and not a review outcome — Apple's reviewers
decide case by case, and both stores' rules have moved repeatedly (the Epic
injunctions in the United States, the DMA in the EU, and Google's revised
settlement entering alternative-billing reporting obligations from 1 October
2026). A merchant outside "physical goods, consumed outside the app" should
check the current text themselves. Sources read: Apple's App Store Review
Guidelines §3.1.1, §3.1.1(a), §3.1.3(a)–(g), and Google Play Console Help's
payments policy.

## An open question this document does not decide

`docs/flows/hosted-checkout/page-memory-and-protocols.md` already documents
three peers a checkout-page window can have — framed (`window.parent`), popup
(`window.opener`), and top-level (`none`) — in a table this design doc calls
"the popup table." A native `WebView`/`WKWebView` is a **fourth** shape, and
whether this document supersedes that table, sits beside it, or the table
grows a fourth column, is **left to the maintainer**: it is a
documentation-structure call on a page six other documents point at, not a
Lane B decision, and the design doc says so explicitly rather than picking a
default. This document does not touch
`hosted-checkout/page-memory-and-protocols.md`.

## Status

**2026-09-14, after the review of Lanes A, B and C, and narrowed the same
day by D8.** All three lanes have landed on one branch and been reviewed
adversarially; this section replaces the Lane-B-only text that stood here,
which said "no Dart file exists in this repository yet" and was true when
it was written and false the moment Lane A merged. D8 then widened the
pigeon seam so `VpayCheckoutMode.externalBrowser` is real rather than
`UnimplementedError` — see the ⛔→✅ move below and the design doc's own D8
section for the reasoning.

What exists and is verified on this host:

- **The Dart core** — `sdks/flutter/vpay_checkout_flutter/lib/`: the browser
  client, the pure state machine, the result and error types, redaction, the
  pigeon seam (now carrying `CheckoutWindowMode`). `flutter test` is green,
  82 passed / 0 skipped (was 80 before D8 added two); the counts and skips
  are on the dated verification page below, which is also where every
  mutation this was proven by is recorded.
- **The Android host, both modes.** `VpayCheckoutActivity`
  (`android:exported="false"`, `onReceivedSslError` not overridden, no
  `addJavascriptInterface`, mode `inApp`) and
  `VpayCheckoutExternalBrowserSession` (`androidx.browser` Custom Tabs, mode
  `externalBrowser`), both behind `VpayCheckoutFlutterPlugin`. Proven by
  compiling: `flutter build apk --debug` **and** `--release` on `example/`
  (the debug-only JS test hook stays debug-only: DEX string count 4 in
  debug, 0 in release, unchanged by D8), and the merged manifest carries the
  Activity with `exported="false"`. **`externalBrowser` proven by running,
  too** — a dedicated headless AVD (`vpay_d8_avd`, created and deleted for
  this) showed a real `CustomTabsIntent` open Chrome
  (`com.android.chrome` as the resumed activity) and a real hardware back
  press return control to the host `Activity`, reporting a real dismissal
  over the real channel. `inApp`'s own existing emulator suites (window +
  dismiss) were re-run on the same device to confirm no regression.
- **The web host, both modes.** `WebVpayCheckoutPlatform`, `window.open`
  plus the `vpay:complete` message and a `closed` poll — `inApp` and
  `externalBrowser` collapse to the identical popup, since there is no
  in-app WebView on Flutter web to distinguish them. Proven by compiling:
  `flutter build web` on `example/`. **No browser has driven it.**
- **The gate** — `cargo xtask verify-sdk-parity` reads Dart, drops skipped
  tests, and since 2026-09-14 drops a skipped group's tests, commented-out
  declarations and titles quoted inside strings.
- **The Dart core against a real, running vpay (2026-09-14).**
  `just test-flutter-e2e` (`test_e2e/real_stack_e2e_test.dart`) mints a real
  Checkout Session through `examples/shop`'s real server — a real
  `private_key_jwt` exchange — and drives `BrowserClient`/`CheckoutController`
  with a real `http.Client` against it: a real pre-flight, a real confirm, a
  real poll to a real terminal outcome, the uniform 404 on a bad credential,
  and a real `checkout_session_expired` refusal on a session that is no
  longer `open`. It found a real bug no `MockClient` fixture had caught — the
  session model required a field the real server never sends back — fixed
  the same day. It is **not** in `just ci` (D-M3, see below), and it refuses
  loudly, never skips, when no stack answers.

What does **not** exist, and is a dated ⛔ in
[`../sdks/parity.md`](../sdks/parity.md) rather than a silence:

- **D8's "tier 1"** (Android App Links / iOS 17.4+ Associated Domains, for
  an external window that closes itself instead of the payer switching back
  manually) — needs a merchant-hosted `assetlinks.json`/
  `apple-app-site-association` deployment this repository cannot provide.
  `ASWebAuthenticationSession` is, deliberately, not a dependency of the iOS
  host for the same reason `androidx.browser` was not one before D8: a
  dependency with no code path that could ever run is its own kind of false
  claim.
- **iOS and macOS** — the Swift exists under `ios/` and `macos/`, now
  including `externalBrowser`'s `VpayCheckoutExternalBrowserSession`, and is
  **compiled by nobody**: this repository runs on Linux and has no
  `xcodebuild` (`swiftc`/`swift` confirmed absent too). It has been reviewed
  by reading, and that is all.
- **A CI gate** — `install-flutter`/`analyze-flutter`/`test-flutter`/
  `test-flutter-e2e` exist and none of them is in `just ci` (D-M3). Every
  count this repository quotes for this package is a human running the
  recipe by hand.
- **A real rail.** The stack `just test-flutter-e2e` drives is real; the
  rail behind it is still WireMock, exactly as it is everywhere else in this
  repository (`docs/status.md`'s banner). Nothing here, or anywhere in this
  repository, has been driven against MTN or Orange for real.
- **A device, an App Store or Play review, and the Android 21 / iOS 12
  floor**, which is a claim nobody has tested.

Evidence:
[`../status/verification/2026-09-14-flutter-review.md`](../status/verification/2026-09-14-flutter-review.md),
and the area page
[`../status/mobile-flutter-plugin.md`](../status/mobile-flutter-plugin.md).

**Corrected 2026-09-16: the Android/web window ✅ above was earned by a path
no real app takes.** `example/integration_test/*.dart` registered the
platform host by hand
(`support/ensure_platform_registered.dart`); `example/lib/main.dart`, the
only entrypoint a real merchant app uses, relied on `pubspec.yaml`'s
`dartPluginClass` mechanism alone, which threw and was silently swallowed
before the app's own `main()` ran, so a real installed APK never opened the
window. **A first fix attempt the same day — deferring
`VpayCheckoutFlutterApi.setUp` out of the constructor and into the first
call to `show` — was written up here as done but never actually landed in
the source file; a second pass the same day found the constructor still
eager and the real device still failing.** The fix that actually landed has
two parts: the constructor now catches the missing-binding error and
`show`/`dismiss`/`windowEvents` each retry it (the `dartPluginClass` race
turned out to be genuinely intermittent, not one-directional), and
`VpayCheckoutPlatform.instance` is now platform-aware on its own — the
first read that finds no host registered yet on Android/iOS/macOS resolves
the method-channel implementation lazily, at a point guaranteed to be after
the app's own `main()` has run, making `dartPluginClass` registration an
optimisation rather than a requirement. The hand-registration helper stays
deleted and the suites rely on the same automatic registration a real app
depends on. Evidence:
[`../status/verification/2026-09-16-flutter-real-app-registration.md`](../status/verification/2026-09-16-flutter-real-app-registration.md),
and the area page's own dated section.

**Revised 2026-09-16: the Android/iOS window is a modal bottom sheet, not a
full-screen window.** The maintainer's own words: the full-screen `Activity`
"feels like the user is quitting the app." This revises design D5 —
[`../plans/2026-09-13-flutter-plugin.md`](../plans/2026-09-13-flutter-plugin.md)'s
own "D5, revised 2026-09-16" section carries the reasoning and every
non-negotiable re-checked. In short: Android's `VpayCheckoutActivity` is
still the same Activity (`android:exported="false"` unchanged) with a
translucent theme and a Material `BottomSheetBehavior` sheet — a ~90% detent,
draggable to full height, per the maintainer's explicit decision — instead of
a full-bleed opaque window; drag-down, a scrim tap and back press all still
resolve through the one existing dismissal signal (D4), never a second
"cancel" path. iOS adopts `UISheetPresentationController`'s `.large()` detent
on 15+ (`.pageSheet`, unchanged, was already card-like on 13/14); iOS 12 — the
D-M4 floor — has no non-full-screen modal presentation API at all and
necessarily stays `.fullScreen` there. macOS is unaffected (already presented
as a sheet) and web is unaffected (the popup was never a full-screen
takeover). Proven by a hand-driven walk on the maintainer's own
`emulator-5554` across three cold launches, all three showing the sheet with
the merchant app visibly behind it, one driven through a real MTN MoMo push
to a paid outcome and back to the merchant screen, the other two exercising
drag-down and a scrim tap as dismissal triggers; `just test-flutter-emulator`
still exits 0, all three suites green. iOS/macOS remain compiled by nobody.
Evidence:
[`../status/verification/2026-09-16-flutter-bottom-sheet.md`](../status/verification/2026-09-16-flutter-bottom-sheet.md).

**Revised again 2026-09-16, later the same day — the WebView is gone, on
every platform, and every mode reference above describes a design this
plugin no longer has.** The plugin now opens the payer's own browser
unconditionally: a partial (bottom-sheet) Custom Tab on Android
(`CustomTabsIntent.setInitialActivityHeightPx`), `SFSafariViewController`
on iOS, the default browser via `NSWorkspace` on macOS, the same popup on
web. `VpayCheckoutMode` and the wire `CheckoutWindowMode` are **deleted
outright**, not deprecated — there is no `inApp`/`externalBrowser` choice
left anywhere above this paragraph to make. `VpayCheckoutViewController.swift`
(the old `WKWebView` controller) is deleted on both Apple platforms, and
`VpayCheckoutActivity.kt` no longer creates a `WebView` of any kind. A new,
narrow, exported `VpayCheckoutAppLinkActivity` exists solely to forward an
incoming Android App Link; the plugin ships no `<intent-filter>` of its own
(a hostless `https` filter would claim every `https` URL on the device),
so the merchant declares one for their own verified host via
`tools:node="merge"`.

**Why**: a WebView renders the payment form inside the merchant app's own
process, where `evaluateJavascript`, the cookie store and a navigation
delegate are all reachable to code the merchant controls — a compromised
merchant app could read the payer's PAN and OTP undetectably. A separate
browser process cannot be inspected that way, and the payer gets a real,
checkable URL bar. **The cost**: no host on any platform can observe a
navigation any more, so `CheckoutWindowOutcome.stopUrlReached` now means an
incoming deep link matched a stop URL, never an intercepted navigation, and
that signal is **unverified on every platform** as of this revision — it
needs a merchant-hosted `assetlinks.json`/`apple-app-site-association`
deployment this repository cannot provide. Every checkout today ends as
`dismissed`, and D1/D4's poll is what makes that correct regardless. macOS
loses even its previous weak signal: the old code reported `dismissed` when
the app merely regained focus, which was never real evidence the browser
had closed; that fake signal is removed, so macOS now has **no dismissal
signal at all** unless a Universal Link arrives or the merchant calls
`dismiss()` itself.

**Verified, and precisely how far.** Run — not merely compiled — for the
first time on a real iOS Simulator (iPhone 17 Pro, iOS 26.5): the
`SFSafariViewController` sheet renders with Safari's own address bar;
tapping through to `success_url` leaves the sheet open (no navigation
interception exists any more); tapping Done reports `dismissed`, D4 polls,
and the checkout resolves `VpayCheckoutSucceeded`. The Dart suite stays 80
passed / 0 skipped, `dart analyze --fatal-infos` clean. Android and
`example/` **compile** (`BUILD SUCCESSFUL`) but were **not** run on an
emulator against this architecture — the Lane E/D8 emulator evidence
earlier in this section predates the cutover and does not carry forward.
The debug-only JS-injection harness (`evaluateJavascriptForTests`,
`VpayCheckoutActivityTestHarness.kt`) and `checkout_window_test.dart` — the
one suite that drove a full MTN MoMo push through the real hosted page
rendered inside a real WebView — are both deleted along with the WebView
they depended on; no suite in this repository drives a full MTN push
through the payer's real hosted checkout page end to end on Android any
more. macOS was not exercised in this pass. Evidence:
[`../status/verification/2026-09-16-flutter-browser-cutover.md`](../status/verification/2026-09-16-flutter-browser-cutover.md).

**Added 2026-09-16, later still — issue #189 lane 1: the server-driven rail
spec and the screen/return machines, pure-Dart core only.** Issue #189 is
the next step past the browser cutover above: rendering the checkout as
native Flutter widgets driven by the #186 rail spec, so the browser this
section just finished describing stops being the checkout and becomes only
the `redirect`-rail handler. This lane builds none of the widgets —
`lib/src/models.dart` parses `CheckoutSession.rails`; `lib/src/sheet/`
carries a 13-screen payment reducer (`machine.ts`'s port), a separate
credential-free return reducer (`return.ts`'s port, D6), the shared
poll-terminal rule, the structural (never-per-code) rail-support decision,
and Dart ports of `money.ts`, `msisdn.ts` and `failures.ts`'s
`providerReason`. `flutter test` is 200 passed / 0 skipped (was 80 before
this lane); three mutations (float money division, a terminal bare
`requires_payment_method`, an inserted rail-code branch) were each applied
to the real source and confirmed to fail the tests built to catch them, then
reverted. **Not done:** the sheet widget, i18n, "remember this number", the
test-mode banner, focus management, the `redirect` hand-off, and wiring the
new jittered-poll primitive into a controller. Evidence:
[`../status/verification/2026-09-16-flutter-rail-spec-screen-machine.md`](../status/verification/2026-09-16-flutter-rail-spec-screen-machine.md).
