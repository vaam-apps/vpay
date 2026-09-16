# Mobile checkout (`vpay_checkout_flutter`)

**What this is.** A Flutter plugin that opens vpay's hosted checkout page — the
same page [hosted-checkout.md](hosted-checkout.md) describes — in a native
window on Android, iOS, macOS and web, and answers the merchant's app with a
typed result once the payment is actually decided. Requested by the
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
3. **Open the native window** on the platform host (Android `Activity` +
   `WebView`, iOS/macOS `UIViewController`/`NSViewController` + `WKWebView`, or
   a `window.open` popup on web — Lane C), loading `session.url` from
   `POST /v1/checkout/sessions`. The window's only job is to show the page and
   report two things back over a typed channel: "reached one of these stop
   URLs" or "the payer left."
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

- **The rail refuses an embedded WebView.** Orange's page has never been
  driven inside one — vpay has only ever talked to a WireMock stub serving two
  links, and "Orange's page inside a WebView is unproven" is named as the
  single biggest technical risk in the design. `VpayCheckoutMode.externalBrowser`
  (D8) is the mitigation, and as of 2026-09-14 it is **wired, not merely
  designed**: `pigeons/checkout.dart`'s `ShowCheckoutRequest` carries a
  `mode` field, threaded end to end. Android launches Custom Tabs
  (`androidx.browser`) and reports the payer's return over
  `Application.ActivityLifecycleCallbacks` — no `startActivityForResult`, no
  scheme to call back on, because the outcome is always polled (D1). The
  external mode degrades correctly with **no merchant deployment work at
  all**, exactly as designed — the payer finishes in the system browser,
  closes it or switches back, and the plugin polls on resume. Android's path
  is **compiled AND run for real** on a headless emulator (a real Custom Tab
  opened, a real back press returned to the host `Activity`, a real
  dismissal arrived over the real channel); iOS wraps
  `SFSafariViewController`, macOS opens the default browser via
  `NSWorkspace`, both **compiled by nobody**. **Not built**: D8's "tier 1"
  (Android App Links / iOS 17.4+ Associated Domains, for a window that
  closes itself), which needs a merchant-hosted deployment this repository
  cannot provide. See the dated rows in
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
  (D3): `addJavascriptInterface`/`WKScriptMessageHandler` are not used, so
  there is nothing on the page for a rail's own content — loaded top-level
  inside the same WebView — to call.
- A session credential never reaches a log line, a generated `toString`, or
  an Android extra/iOS property that outlives the window (D6) — a parity row
  in its own right, not a promise; see
  [`../sdks/parity.md`](../sdks/parity.md) once Lane A lands it.
- `onReceivedSslError` is never overridden and `decidePolicyFor` never bypasses
  certificate validation — an SDK that proceeds through a certificate error on
  a payment page is the single worst thing this plugin could ship (D5).

## App Store and Play policy — a constraint on adoption, not a code change

Raised by the maintainer on 2026-09-13: _"won't apple flag us? Even if we're
using a browser, technically we're still in app right?"_ — checked against the
live guidelines rather than answered from memory (design doc D9; the numbering
had already moved — it is 3.1.3(e), not the 3.1.5(a) it was for years).

**The rule keys on what is sold, not on where the payment UI lives.** The
in-app `WebView` and the external-browser mode (D8) are treated identically by
both stores; neither is a way around anything.

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
digital goods. A vpay checkout for any of those is a rejection, in-app WebView
or external browser alike.

**The trap: `externalBrowser` does not move a digital-goods app out of
3.1.1.** Apple's **3.1.1(a)** goes further: outside the United States
storefront, an app and its metadata "may not include buttons, external links,
or other calls to action that direct customers to purchasing mechanisms other
than in-app purchase" without a StoreKit External Purchase Link Entitlement,
which is region-limited. For a digital-goods merchant, opening Safari via
`VpayCheckoutMode.externalBrowser` is not neutral — it can itself be the
violation. The plugin's own README documents `externalBrowser` and this
constraint in the same breath, so the mode does not read as a workaround.

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
