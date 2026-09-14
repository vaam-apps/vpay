# vpay_checkout_flutter

A payer-facing checkout surface for vpay: opens vpay's hosted checkout page
in a native window and answers a typed [`VpayCheckoutResult`] once a payment
intent actually settles. Design:
[`docs/plans/2026-09-13-flutter-plugin.md`](../../../docs/plans/2026-09-13-flutter-plugin.md).
Decisions: [ADR-0021](../../../docs/adr/0021-flutter-checkout-plugin.md).

**Status (2026-09-14, narrowed the same day by D8).** Android and web hosts
exist and are wired up (`android/`, and the web implementation in
`lib/src/platform/web_checkout_platform.dart`) for **both**
`VpayCheckoutMode.inApp` and `.externalBrowser` — Android compiles for real
(`flutter build apk --debug` **and** `--release` on `example/`; the
debug-only JS test hook stays debug-only through this change, `0`
occurrences in the release DEX, `4` in debug) and web compiles for real
(`flutter build web` on `example/`). Android's `externalBrowser` (Custom
Tabs) was also **run for real** on a headless emulator — see "Platform
hosts", below. **iOS and macOS Swift also exist (`ios/`, `macos/`), both
modes, but are compiled by nobody** — this repository has no macOS/iOS
toolchain (Linux host), so that Swift is reviewed by reading only, never
built, never run.

**The Dart core has been driven against a real, running vpay (2026-09-14),
never a real rail.** `just test-flutter-e2e` mints a real Checkout Session
through `examples/shop`'s real server and drives this package's own
`BrowserClient`/`CheckoutController` — a real pre-flight, a real confirm, a
real poll to a real terminal outcome — against it. The rail behind that
stack is WireMock, exactly as it is everywhere else in this repository; see
"Development", below, and `docs/sdks/parity.md`'s two rows on this.

**`VpayCheckoutMode.externalBrowser` is wired end to end (D8, 2026-09-14).**
`pigeons/checkout.dart`'s `ShowCheckoutRequest` now carries a `mode` field,
and every platform host acts on it — no custom URL scheme anywhere, on any
platform (D8: `checked_forward_url` accepts only `http(s)`, and a scheme is
first-come-first-served on Android, so any installed app could claim one).
Return detection is D8's own "tier 0": the payer finishes on the merchant's
own page inside the external window, the window closes or the payer
switches back, and the plugin reports a dismissal — `checkout_controller
.dart`'s D4 already polls the real payment intent before answering a
dismissal, so this is correctness-complete (D1), not a downgrade.

- **Android — Custom Tabs (`androidx.browser`), compiled for real AND run
  for real on a headless emulator.** `VpayCheckoutFlutterPlugin` launches a
  `CustomTabsIntent`; `VpayCheckoutExternalBrowserSession` reports a
  dismissal the moment the host `Activity` itself resumes
  (`Application.ActivityLifecycleCallbacks` — a Custom Tab is another app's
  window, with no `startActivityForResult` and no scheme to call back on).
  `example/integration_test/checkout_external_browser_test.dart` drove this
  on a real AVD: a real Custom Tab opened (Chrome, confirmed present on the
  image, `com.android.chrome` as the resumed activity), a real hardware
  back press returned to the host `Activity`, and the real dismissal
  arrived over the real platform channel. See
  [`docs/sdks/parity.md`](../../../docs/sdks/parity.md) for the dated row.
- **Web** opens the same popup it always does — a `window.open` popup
  already _is_ an external-browser context; there is no in-app WebView on
  Flutter web to distinguish `inApp` from. See
  `WebVpayCheckoutPlatform`'s own doc comment.
- **iOS** wraps `SFSafariViewController`, never `ASWebAuthenticationSession`
  — this README's own "App Store and Play policy" section and design doc D8
  explain why. **macOS** opens the payer's default browser via
  `NSWorkspace` (the maintainer's own call — no `SFSafariViewController` and
  no Custom-Tabs equivalent exist on macOS). **Both are compiled by
  nobody** — see the Status paragraph above.
- **Not implemented: D8's "tier 1"** (Android App Links / iOS 17.4+
  Associated Domains, for a tab that closes itself instead of relying on
  the payer switching back manually). It needs a merchant-hosted deployment
  this repository cannot provide; see
  [`docs/sdks/parity.md`](../../../docs/sdks/parity.md)'s dated ⛔ row.

## What this is not

`POST /v1/checkout/sessions` stays on the merchant's own server, exactly as
it does for a web integration. This package never holds a merchant token, a
private key, or an `Authorization` header — only a publishable key and one
session's own short-lived credentials, read out of the `url` the merchant's
server already returned.

**`VpayCheckoutResult.succeeded` is a UI fact, not a settlement.** A
merchant's server must still act on `payment_intent.succeeded` from the
webhook — this package never reads an outcome off a URL a merchant's own
site controls (design doc D1); it polls
`GET /v1/browser/payment_intents/{id}` and answers what that says.

## App Store and Play policy — read this before shipping `externalBrowser`

Raised by the maintainer: _"won't apple flag us? Even if we're using a
browser, technically we're still in app right?"_ (design doc D9).

**The rule keys on what is sold, not on where the payment UI lives.** The
in-app `WebView` (the default `VpayCheckoutMode.inApp`) and
`VpayCheckoutMode.externalBrowser` are treated identically by both stores;
neither is a way around anything.

- **Selling a physical good, or a service consumed outside the app?** You
  are required to use a method other than in-app purchase — Apple's App
  Store Review Guidelines, **3.1.3(e)**, verbatim:

  > If your app enables people to purchase physical goods or services that
  > will be consumed outside of the app, you must use purchase methods other
  > than in-app purchase to collect those payments, such as Apple Pay or
  > traditional credit card entry.

  This plugin is exactly such a method. Google Play is the same shape from
  the other side: Play Billing is for digital items only.

- **Unlocking something _within_ the app** — a subscription, in-app credits,
  game levels, premium content, a digital gift card? Apple's **3.1.1**
  requires In-App Purchase for that, and a vpay checkout for any of it is a
  rejection — in-app `WebView` or `externalBrowser` alike.

- **The trap: `externalBrowser` does not move a digital-goods app out of
  3.1.1.** Apple's **3.1.1(a)** goes further: outside the United States
  storefront, an app "may not include buttons, external links, or other
  calls to action that direct customers to purchasing mechanisms other than
  in-app purchase" without a region-limited StoreKit entitlement. For a
  digital-goods merchant, opening Safari via `externalBrowser` is not
  neutral — it can itself be the violation.

This is a reading of the published guidelines on 2026-09-13, not legal
advice and not a review outcome — Apple's reviewers decide case by case, and
both stores' rules move. See
[`docs/flows/mobile-checkout.md`](../../../docs/flows/mobile-checkout.md)
for the fuller quotes and sources read.

## Credentials and redaction (D6)

- `toString()` is overridden on every type holding a session URL, a session
  secret or an intent secret (`PaymentIntent`, `CheckoutSession`) —
  `[N chars redacted]`, never the value.
- No `print`, `debugPrint` or `log` call anywhere in `lib/` — asserted by
  `test/no_logging_test.dart`, which reads the package's own source,
  qualified calls (`developer.log(…)`) included.
- The **pigeon-generated** channel types redact too. `ShowCheckoutRequest`
  holds the session URL, whose fragment _is_ the session's `client_secret`,
  and pigeon's generated `toString`/`description` rendered it verbatim in
  Dart, Kotlin and Swift alike until 2026-09-14. All three are hand-edited;
  `test/messages_redaction_test.dart` is what keeps the Dart one edited
  across a `dart run pigeon` that would overwrite it.
- `BrowserClient` refuses a non-`https` base URL unless the named
  `allowInsecureBaseUrl` opt-in is passed — for `compose.demo.yml` only,
  never inferred from a debug build.

## Development

```bash
just install-flutter   # flutter pub get
just analyze-flutter   # dart analyze --fatal-infos
just test-flutter      # flutter test (unit tests only, no device, no stack)
just test-flutter-e2e  # test_e2e/ against a REAL, RUNNING vpay (see below)
```

None of the four is in `just ci` yet (D-M3) — see `docs/sdks/parity.md`'s
dated ⛔ row.

**`just test-flutter` is entirely `MockClient` — `just test-flutter-e2e` is
not.** `test_e2e/real_stack_e2e_test.dart` drives this package's own
`BrowserClient`/`CheckoutController` with `package:http`'s real `Client`,
against whatever `just demo-up` has running: a real session read, a real
pre-flight, a real confirm, a real poll to a real terminal outcome, the
uniform 404 on a bad credential, and a real `checkout_session_expired` 409 on
a session that is no longer `open`. The fixture it reads is minted by the
recipe itself through `examples/shop`'s real server — a real
`private_key_jwt` exchange, never a credential this package holds (see "What
this is not", above). It refuses loudly, never skips, when no stack answers.
**Still not a real rail** — see `docs/sdks/parity.md`'s two Flutter rows on
this, which are now two rows and not one for exactly this reason.

## Platform hosts (design doc D5, D8)

- **Android** — `VpayCheckoutActivity` (plain `Activity` + `WebView`,
  `android:exported="false"`, mode `inApp`) and
  `VpayCheckoutExternalBrowserSession` (`androidx.browser` Custom Tabs, mode
  `externalBrowser`), both behind `VpayCheckoutFlutterPlugin`. Compiled for
  real: `flutter build apk --debug`/`--release` in `example/`. **Run for
  real**, both modes: `just test-flutter-emulator`'s existing suites (in-app
  window + dismiss) and `example/integration_test
/checkout_external_browser_test.dart` (external browser), all on a real
  headless AVD.
- **Web** — `WebVpayCheckoutPlatform` opens the session's `url` in a popup
  (`window.open`), the same surface `sdks/stripe-js/src/popup.ts` treats as
  a first-class peer, for **both** `inApp` and `externalBrowser` — there is
  no in-app WebView on Flutter web to distinguish the two, so a popup
  already is this platform's external-browser context. There is no
  navigation to watch across a popup's cross-origin boundary, so the only
  two signals are a `{type: 'vpay:complete', …}` `postMessage` from the
  popup's own page (the merchant's own return page has to send it — see the
  module doc comment) and the popup's `closed` property, polled. Compiled
  for real: `flutter build web` in `example/`.
- **iOS** — `VpayCheckoutViewController`/`VpayCheckoutFlutterPlugin` (mode
  `inApp`) and `VpayCheckoutExternalBrowserSession` wrapping
  `SFSafariViewController` (mode `externalBrowser`, never
  `ASWebAuthenticationSession` — see the App Store/Play section above).
- **macOS** — the same shape, its `externalBrowser` opening the payer's
  default browser via `NSWorkspace` and watching for this app's own
  reactivation instead of a Custom Tab or `SFSafariViewController`, neither
  of which exist on macOS.
- **iOS and macOS are compiled by nobody**: this repository has no
  macOS/iOS toolchain. Reviewed by reading only.
- **D8's "tier 1"** (Android App Links / iOS 17.4+ Associated Domains, so
  the external window can close itself instead of the payer switching back
  manually) **is not implemented on any platform** — it needs a
  merchant-hosted deployment (`assetlinks.json` /
  `apple-app-site-association`) this repository cannot provide. See
  `docs/sdks/parity.md`'s dated ⛔ row.
