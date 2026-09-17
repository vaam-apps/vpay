# vpay_checkout_flutter

Opens vpay's hosted checkout page in the payer's **browser** — a bottom-sheet
Custom Tab on Android, a detented `SFSafariViewController` on iOS — and
answers a typed `VpayCheckoutResult` once the payment intent actually
settles.

Design: [`docs/plans/2026-09-13-flutter-plugin.md`](../../../docs/plans/2026-09-13-flutter-plugin.md).
Decisions: [ADR-0021](../../../docs/adr/0021-flutter-checkout-plugin.md).

## Getting started

### 1. Add the package

```yaml
dependencies:
  vpay_checkout_flutter:
    path: ../path/to/sdks/flutter/vpay_checkout_flutter # not published yet
```

No platform setup is needed. There is nothing to add to your
`AndroidManifest.xml` or `Info.plist` to open a checkout.

### 2. Your server creates the session

This package never creates one. `POST /v1/checkout/sessions` needs a merchant
credential, and a credential on a payer's device is a credential that has
left your control. Your own server does it and returns the session `url`:

```jsonc
// your server's response to your app
{ "url": "https://checkout.vpay.example/c/cs_abc123?key=pk_test_…#cs_abc123_secret_…" }
```

That URL's fragment is the session's `client_secret`. Treat it like one.

### 3. Open the sheet

```dart
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

final VpayCheckoutResult result = await showVpayCheckoutSheet(
  context,
  sessionUrl: sessionUrl,
  baseUrl: 'https://api.vpay.example',
  publishableKey: 'pk_test_…',
  merchantName: 'Njangi Store',
);
```

A native bottom sheet rises over your app — your UI stays visible behind
it. It returns when the outcome is known, not when the payer closes the
sheet: it confirms, then polls `GET /v1/browser/payment_intents/{id}` until
the intent reaches a terminal state or the budget runs out.

Rails that need the payer to authenticate on the rail's own site (Orange
Money) hand off to a browser sheet and come back to this one. You do not
choose that — the server's rail spec does.

`showVpayCheckoutSheetRoute` is the same thing as a full-screen route, for
apps where a sheet does not fit.

### 4. Handle every outcome

`VpayCheckoutResult` is a sealed class, so the compiler makes you handle all
five. That is deliberate: in payments, the case you forget is the one that
costs money.

```dart
switch (result) {
  case VpayCheckoutSucceeded(:final paymentIntentId):
    // The intent reached `succeeded`. Show a receipt — but see the warning
    // below: fulfil from your webhook, not from here.
    showReceipt(paymentIntentId);

  case VpayCheckoutFailed(:final code, :final providerMessage):
    // The rail declined. `providerMessage` is the rail's own words
    // ("insufficient funds") — show it *beside* your message, not instead.
    showError(code, providerMessage);

  case VpayCheckoutCanceled():
    // The payer walked away and the intent is terminally canceled.
    returnToCart();

  case VpayCheckoutPending():
    // Still moving when the poll budget ran out — a slow rail, not a
    // failure. Never show this as an error; your webhook will settle it.
    showPending();

  case VpayCheckoutUnresolved(:final error):
    // We could not find out: the window never opened, the network failed,
    // the session was expired. No money moved on *our* side of the call.
    showUnresolved(error);
}
```

### 5. Fulfil from the webhook, never from this result

**`VpayCheckoutSucceeded` is a UI fact, not a settlement.** It means the
payer's device observed `succeeded`. Ship the goods when your server
receives `payment_intent.succeeded` and verifies its signature. A payer's
device is not an authority on whether you were paid.

### Testing against the demo stack

`compose.demo.yml` serves plain HTTP, which `BrowserClient` refuses by
default. Opt in by name — never inferred from a debug build:

```dart
VpayCheckout(
  baseUrl: 'http://localhost:8080',
  publishableKey: 'pk_test_…',
  allowInsecureBaseUrl: true, // demo stack only
);
```

`example/` is a runnable app. Its three fields can be pre-filled at build
time so a cold launch is one tap:

```bash
flutter run \
  --dart-define=VPAY_BASE_URL=http://localhost:8080 \
  --dart-define=VPAY_PUBLISHABLE_KEY=pk_test_… \
  --dart-define='VPAY_SESSION_URL=http://localhost:3080/c/cs_…#cs_…_secret_…'
```

## Status (2026-09-16)

**The checkout is the payer's browser on every platform.** The in-app
`WebView` was removed on 2026-09-16: it rendered vpay's payment form inside
the merchant app's own process, where `evaluateJavascript`, the cookie store
and a navigation delegate are all reachable — a compromised merchant app
could read the payer's PAN and OTP and the payer could not tell. A browser
runs in its own process and shows a real URL bar.

Driven by hand against a real `just demo-up` stack, a real session and a real
MTN push, on both platforms — the merchant's own database read `paid` via its
HMAC-verified webhook, and the SDK answered `VpayCheckoutSucceeded`:

| Platform    | Surface                                               | Verified                                        |
| ----------- | ----------------------------------------------------- | ----------------------------------------------- |
| **iOS**     | `SFSafariViewController`, `.large()` detent           | Driven end to end on iPhone 17 Pro / iOS 26.5   |
| **Android** | Partial (bottom sheet) Custom Tab, `androidx.browser` | Driven end to end on an emulator                |
| **macOS**   | `NSWorkspace.open` — the payer's default browser      | Compiles; not driven                            |
| **Web**     | `window.open` popup                                   | `flutter build web`; unit-tested in real Chrome |

There is no mode to choose. `VpayCheckoutMode` and the wire-level
`CheckoutWindowMode` were deleted with the WebView — one surface, nothing to
select between.

### What is still not true

- **Deep-link return is unverified on every platform.** Android App Links and
  iOS/macOS Universal Links both need an HTTPS origin serving
  `assetlinks.json` / `apple-app-site-association`, which this repository
  cannot deploy. The code is wired; no deep link has ever been driven through
  it. Every checkout today ends as a dismissal, and the poll is what makes
  that correct.
- **The sheet does not close itself.** No host can see a navigation any more,
  so the payer taps Done. Observed, not theorised: on iOS the payer reaches
  `success_url` and the sheet stays open.
- **macOS has no dismissal signal at all.** `NSWorkspace.open` gives no
  callback. Unless a Universal Link arrives or you call `dismiss()`, nothing
  tells the app the browser closed.
- **Chrome's first-run screen can intercept the checkout.** On a device where
  Chrome has not finished onboarding, tapping Pay shows "Make Chrome your
  own" and a Google sign-in prompt before the page loads. Not fixable from
  this side.
- **No real rail.** WireMock behind everything, as everywhere else here.
- **No CI gate runs any Flutter recipe** (D-M3). A human runs them.

## Customising it

The sheet inherits your app. There are **no hardcoded colours in it** —
every colour comes from `Theme.of(context)`, so if your app sets a
`ColorScheme`, the sheet already uses it. Nothing to configure.

What you can pass today:

| Parameter        | Effect                                                                              |
| ---------------- | ----------------------------------------------------------------------------------- |
| `merchantName`   | named in the summary and the outcome copy                                           |
| `locale`         | `VpayLocale.fr` / `.en`; **French is the default**                                  |
| `allowedMethods` | narrows the rails on offer — it can only ever narrow what the server already allows |
| `borderRadius`   | the sheet's top corners; defaults to `kVpayCheckoutSheetCornerRadius` (28)          |

28 rather than Material's default because this sheet is often _replaced on
screen_ by a system browser sheet when the payer picks a redirect rail, and
a squarer sheet handing over to a much rounder system one reads as a
glitch. Override it if your app's surfaces speak a different language.

Buttons have a 52pt minimum tap target — above both Material's 48dp and
Apple's 44pt, because this is a small surface used once, usually
one-handed, by someone anxious about money.

### What you cannot customise yet, and why

- **The inner radii** (summary card, fields, drag handle) are still
  hardcoded. Only the sheet's own corner is configurable. A scoped
  `VpayCheckoutTheme` carrying all of them is designed but not built —
  today, restyling those means restyling your app's `ThemeData`.
- **The country on a phone field.** The rail spec declares exactly one
  `region` per field (`"CM"` for MTN Cameroon) and the server enforces it,
  so there is nothing for a country picker to pick. A picker would either
  offer one country or let a payer enter a number the server then refuses.
  Making it real means the spec carrying `regions` as a list first — a
  backend change, not a widget.
- **Your own form fields.** Deliberately not supported. The field set is
  declared by the server so the sheet can render a rail it has never heard
  of; a field injected by the app breaks the contract that makes that work,
  and the sheet would be collecting data vpay never declared and cannot
  validate. Put merchant-specific input in **your own UI before opening the
  sheet**, or in the payment intent's `metadata`, which travels with the
  payment server-side. This matters more as card rails arrive: a
  merchant-supplied field inside a payment sheet is the shape that turns
  into a PCI question.

## What this is not

`POST /v1/checkout/sessions` stays on the merchant's own server, exactly as
it does for a web integration. This package never holds a merchant token, a
private key, or an `Authorization` header — only a publishable key and one
session's own short-lived credentials, read out of the `url` the merchant's
server already returned.

## App Store and Play policy — read this before shipping

Raised by the maintainer: _"won't apple flag us? Even if we're using a
browser, technically we're still in app right?"_ (design doc D9).

**The rule keys on what is sold, not on where the payment UI lives.** A
browser sheet and an in-app web view are treated identically by both stores;
neither is a way around anything.

- **Selling a physical good, or a service consumed outside the app?** You are
  required to use a method other than in-app purchase — Apple's App Store
  Review Guidelines, **3.1.3(e)**, verbatim:

  > If your app enables people to purchase physical goods or services that
  > will be consumed outside of the app, you must use purchase methods other
  > than in-app purchase to collect those payments, such as Apple Pay or
  > traditional credit card entry.

  This plugin is exactly such a method. Google Play is the same shape from
  the other side: Play Billing is for digital items only.

- **Unlocking something _within_ the app** — a subscription, in-app credits,
  game levels, premium content, a digital gift card? Apple's **3.1.1**
  requires In-App Purchase for that, and a vpay checkout for any of it is a
  rejection.

- **The trap: using a browser does not move a digital-goods app out of
  3.1.1.** Apple's **3.1.1(a)** goes further: outside the United States
  storefront, an app "may not include buttons, external links, or other calls
  to action that direct customers to purchasing mechanisms other than in-app
  purchase" without a region-limited StoreKit entitlement. For a
  digital-goods merchant, opening a browser is not neutral — it can itself be
  the violation.

This is a reading of the published guidelines on 2026-09-13, not legal advice
and not a review outcome — Apple's reviewers decide case by case, and both
stores' rules move. See
[`docs/flows/mobile-checkout.md`](../../../docs/flows/mobile-checkout.md) for
the fuller quotes and sources read.

## Credentials and redaction (D6)

- `toString()` is overridden on every type holding a session URL, a session
  secret or an intent secret — `[N chars redacted]`, never the value.
- No `print`, `debugPrint` or `log` call anywhere in `lib/` — asserted by
  `test/no_logging_test.dart`, which reads the package's own source,
  qualified calls (`developer.log(…)`) included.
- The **pigeon-generated** channel types redact too, and this is fragile in a
  way worth knowing about. `ShowCheckoutRequest` holds the session URL, whose
  fragment _is_ the `client_secret`, and pigeon's generated
  `toString`/`description` renders it verbatim. All four generated copies
  (Dart, Kotlin, and both Swift) are **hand-edited after generation**, and
  `dart run pigeon` silently reverts them — it did on 2026-09-16.
  `test/messages_redaction_test.dart` catches the Dart copy. **Nothing
  catches the other three.** If you regenerate, re-apply all four by hand.
- `BrowserClient` refuses a non-`https` base URL unless the named
  `allowInsecureBaseUrl` opt-in is passed.

## Development

```bash
just install-flutter   # flutter pub get
just analyze-flutter   # dart analyze --fatal-infos
just test-flutter      # flutter test (unit tests only, no device, no stack)
just test-flutter-e2e  # test_e2e/ against a REAL, RUNNING vpay
just test-flutter-web  # the web platform host, in a real Chrome
```

None is in `just ci` yet (D-M3) — see `docs/sdks/parity.md`'s dated ⛔ row.

**`just test-flutter` is entirely `MockClient` — `just test-flutter-e2e` is
not.** The latter drives this package's own `BrowserClient`/
`CheckoutController` with a real `package:http` client against whatever `just
demo-up` has running: a real session read, a real confirm, a real poll to a
real terminal outcome, the uniform 404 on a bad credential, and a real
`checkout_session_expired` 409. Its fixture is minted through
`examples/shop`'s real server with a real `private_key_jwt`. It refuses
loudly, never skips, when no stack answers.

If you are on a Mac, note that `just demo-up` required a fix to run at all
(BSD vs GNU `sed`) — if your stack silently starts nothing, check you have
that fix.

## Platform hosts (design doc D5, revised 2026-09-16)

- **Android** — `VpayCheckoutActivity` launches a partial Custom Tab
  (`setInitialActivityHeightPx`, `androidx.browser` 1.8.0). The Activity
  stays `android:exported="false"`: it receives the credential-bearing URL,
  and exporting it would be an intent-redirection hole. It is also what
  closes the tab — a Custom Tab lives in your app's own task, so relaunching
  an activity beneath it with `FLAG_ACTIVITY_CLEAR_TOP` finishes it.
  `VpayCheckoutAppLinkActivity` is a second, deliberately narrow exported
  component whose only job is to forward an incoming App Link; it holds no
  state and never sees the session secret. **It ships no `<intent-filter>`** —
  a hostless `https` filter would make every app using this plugin a
  candidate handler for every link on the device. You declare one for your
  own verified host.
- **iOS** — `SFSafariViewController` at a `.large()` detent, presented by
  `VpayCheckoutFlutterPlugin`. Deployment floor **iOS 15** (Swift Concurrency
  needs 13+, and Flutter 3.48 pins projects to 15 regardless).
- **macOS** — `NSWorkspace.shared.open(url)`, the payer's real default
  browser. Floor **macOS 12**. See the dismissal caveat above.
- **Web** — `WebVpayCheckoutPlatform` opens the session `url` with
  `window.open`, the same surface `sdks/stripe-js/src/popup.ts` treats as a
  first-class peer. The only two signals are a `{type: 'vpay:complete', …}`
  `postMessage` from the popup's own page — origin **and** `event.source`
  pinned — and the popup's `closed` property, polled.

One note for anyone reading the native code: `show`/`dismiss` on the Apple
hosts are `@MainActor`, and that is load-bearing. Pigeon's generated
dispatcher wraps the call in `Task { @MainActor in … }`, but the protocol
requirement is _nonisolated_ `async`, so Swift hops straight back off the
main actor — without the annotation the first real tap dies on
`NSInternalInconsistencyException: Call must be made on main thread`.
