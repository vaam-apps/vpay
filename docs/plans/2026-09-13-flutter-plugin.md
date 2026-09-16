# Flutter plugin — a native payer window over the hosted checkout page

- **Date:** 2026-09-13
- **Requested by the maintainer**, verbatim: _"Let's develop a custom vpay
  flutter plugin. We'll do it by using a new Activity (android),
  UIViewController (iOS/…) and similar for web. We'll do the payment activity
  in the mobile window and call the hosted page."_
- **Status: design only, decisions taken.** D-M1 to D-M5 were answered by the
  maintainer on 2026-09-13 and are folded in below; the table at the foot says
  who took what. Nothing below is built. No Dart file exists in this
  repository, no Flutter toolchain is installed in CI or in the `vpay-ci` VM,
  and no handset has ever opened a vpay checkout.
- **Base:** `master` at `7a607a68`.

## What already exists, verified rather than assumed

Read before designing, all of it on `7a607a68`:

- **A hosted page that already works top-level with no peer.** The payer page
  has three shapes — framed (`/e/{id}`), popup (`/c/{id}` with an opener), and
  **top-level `/c/{id}`, whose peer is `none`, which renders normally and whose
  "Back to {merchant}" button does `location.assign(success_url | cancel_url)`
  (`docs/flows/hosted-checkout/page-memory-and-protocols.md`, the popup table).
  A `WKWebView`/Android `WebView` is that third shape. **The page needs no
  change to run in one.**
- **A session object that carries its own forwarding URLs.**
  `GET /v1/browser/checkout/sessions/{id}?key&client_secret` answers the
  session with `success_url`, `cancel_url`, `url`, `status`, `expires_at`,
  `ui_mode`, and `payment_intent` **expanded and carrying the intent's own
  `client_secret`** (`backends/crates/vpay-api/src/browser/checkout_sessions.rs`).
- **An intent read that outlives the session.**
  `GET /v1/browser/payment_intents/{id}?key&client_secret` authenticates on the
  publishable key and the intent's secret alone — `browser::retrieve` calls
  `authenticate` and nothing else. It is **not** gated on the session's
  `status` or its 24-hour horizon; only `confirm` is
  (`docs/flows/hosted-checkout.md`, "A session that is not `open` refuses the
  confirm"). This is the fact the whole design rests on.
- **CORS is `Any` with credentials off** on the `/v1/browser` nest and nowhere
  else (`vpay-api/src/lib.rs`), so a native HTTP client and a WebView both
  reach it without an origin registration.
- **The hosted `url` is `{base}/c/{cs_id}?key={pk}#{cs_secret}`**
  (`v1/checkout_sessions.rs:1073`) — key in the query, session secret in the
  fragment (D6).
- **No Dart anywhere.** One incidental string in `docs/plans/exp9-notes/opus.md`
  (`cratestack-client-dart`, a build log) is the only match in the tree.

## The invariant this plugin must not break

**A Flutter app is a payer's device, not a merchant's server.** It holds a
publishable key and one session's credentials, and nothing else — no merchant
token, no private key, no `Authorization` header, ever. `POST
/v1/checkout/sessions` stays on the merchant's server, exactly as it does for
the web integration. The plugin is handed the `url` that server returned.

That is the same split `docs/flows/browser-checkout.md` states for the browser,
and it is the reason this plugin belongs beside `sdks/stripe-js` in the parity
matrix rather than beside `sdks/rust` and `sdks/nodejs`.

## The shape

```
 Dart (all the decisions, no platform code, unit-testable on a laptop)
 ┌──────────────────────────────────────────────────────────────────┐
 │ VpayCheckout.start(sessionUrl)                                    │
 │   1. parse       url → base, cs_id, pk, session secret (fragment) │
 │   2. pre-flight  GET /v1/browser/checkout/sessions/{cs_id}        │
 │                  → ui_mode, success_url, cancel_url, expires_at,  │
 │                    payment_intent (expanded) + its client_secret  │
 │   3. open        ──────────────► platform window (below)          │
 │   4. watch       ◄────────────── "reached <url>" | "payer left"   │
 │   5. resolve     poll GET /v1/browser/payment_intents/{pi} until  │
 │                  terminal, on a budget                            │
 │   6. answer      VpayCheckoutResult                               │
 └──────────────────────────────────────────────────────────────────┘
        │ Pigeon-generated, typed channel: show(url, stopUrls) / dismiss()
        ▼
 ┌──────────────┬──────────────┬──────────────┬──────────────────────┐
 │ Android      │ iOS          │ macOS        │ Web                  │
 │ Activity     │ UIViewCtrl   │ NSViewCtrl   │ window.open popup    │
 │ + WebView    │ + WKWebView  │ + WKWebView  │ (no plugin window)   │
 └──────────────┴──────────────┴──────────────┴──────────────────────┘
   …and the same four behind `VpayCheckoutMode.externalBrowser` (D8):
   Custom Tabs / SFSafariViewController instead of an in-app WebView.
   Same Dart state machine, a weaker "reached a stop URL" signal.
```

The platform half is deliberately stupid: **show this URL; tell me when the
payer reaches one of these URLs, or leaves.** It decides nothing, formats no
money, knows no status vocabulary and holds no credential longer than the URL
it was handed. Every rule lives in Dart, where a test can reach it without a
device — the same choice `frontends/apps/checkout` made when it put its state
machine in a pure reducer with React as wiring.

## D1 — the outcome comes from the API, never from a URL

**The plugin resolves the result by polling the payment intent. A navigation to
`success_url` closes the window; it does not decide anything.**

Interception tells you the _page forwarded_. Only the intent tells you _money
moved_. The two normally agree — `forwardKindFor` picks `success_url` on a paid
session — but a plugin that reads success off a URL is the first item on this
repository's own list of ways to damage it: something that returns a plausible
hard-coded success. The merchant also controls those URLs, and a merchant whose
`success_url` is reached by their own site's redirect chain would have the
plugin lying.

So: the window's job is the payer's experience, and `/v1/browser` is the source
of truth. One consequence worth stating plainly in the plugin's own README,
because every mobile SDK gets this wrong: **`VpayCheckoutResult.succeeded` is a
UI fact, not a settlement. A merchant's server must still act on
`payment_intent.succeeded` from the webhook.**

## D2 — the pre-flight, and why the plugin needs no URL configuration

Before any window opens, the plugin calls
`GET /v1/browser/checkout/sessions/{cs_id}?key&client_secret` once. It buys
four things that are each otherwise a configuration burden or a bug:

1. **The intent id and the intent's `client_secret`** — the polling credential.
   Fetched while the session is `open`, which is the only time that read is
   served; the intent read then works for the rest of the intent's life.
2. **`success_url` and `cancel_url`, from the session itself** — so the stop
   list is derived, not configured. The merchant does not have to tell the app
   what its own backend put in the session, and the two cannot drift.
3. **`ui_mode`** — an `embedded` session is refused here, with a typed error,
   rather than rendering a page that refuses itself because it is not framed.
4. **A fail-fast on a bad link** — a wrong key, an expired session or a typo is
   one typed `VpayError` instead of a blank WebView the payer stares at.

`{CHECKOUT_SESSION_ID}` is substituted in both URLs before they become stop
rules, because vpay substitutes it when it forwards. Matching is on **scheme,
host, port and path** — query and fragment ignored — so a merchant whose
`success_url` carries their own parameters still matches.

Cost: one extra round trip before the window. Taken deliberately.

## D3 — no JavaScript bridge, and no change to the checkout page

`addJavascriptInterface` / `WKScriptMessageHandler` is **not** used, and the
page gets no `native` peer to go beside `parent` and `opener`.

The WebView loads third-party content by design — Orange's own page is a
top-level navigation inside it. A JS bridge is reachable from whatever is
loaded at the time unless it is very carefully origin-gated, and the gate would
have to be right on every platform. The stop-URL mechanism is a navigation
observation, not an API the page exposes, so there is nothing for a rail's page
(or an injected script) to call.

This is also what keeps the plugin buildable today: **zero server change, zero
page change, zero new route.** If a future need genuinely requires the page to
speak to a native host, it should arrive as a fourth peer with its own origin
rule, as a separate decision.

## D4 — dismissal is never reported as cancellation until the API says so

The payer swipes the sheet down, or presses Back. That is not an answer: they
may have approved the MTN push on their handset ten seconds earlier.

So dismissal runs one **short** poll (a few seconds) and reports what the intent
actually says. The result type therefore has a `pending` arm, and it is not a
failure:

```dart
sealed class VpayCheckoutResult {
  final String sessionId;
  final String paymentIntentId;
}
class VpayCheckoutSucceeded  extends VpayCheckoutResult { … }
class VpayCheckoutFailed     extends VpayCheckoutResult { FailureCode? code;
                                                          String? providerMessage; }
class VpayCheckoutCanceled   extends VpayCheckoutResult { … }
class VpayCheckoutPending    extends VpayCheckoutResult { … } // still processing
class VpayCheckoutUnresolved extends VpayCheckoutResult { VpayError error; }
```

`VpayCheckoutPending` exists so the plugin never has to choose between lying and
throwing. `providerMessage` is the rail's own words, carried as data and
labelled as the provider's — the same treatment the page gives it.

## D5 — what the native window is, per platform

**Android.** `VpayCheckoutActivity`, a plain `Activity` with a full-bleed
`WebView`, launched with `startActivityForResult` and answering through an
`ActivityResultContract`. Non-negotiable settings:

- `android:exported="false"` in the plugin's manifest. An exported Activity that
  takes a URL extra is an intent-redirection hole, and the extra here carries a
  payer credential.
- `javaScriptEnabled = true` (the page is React) and `domStorageEnabled = true`
  (page memory).
- `allowFileAccess = false`, `allowFileAccessFromFileURLs = false`,
  `allowUniversalAccessFromFileURLs = false`, no `addJavascriptInterface`.
- `onReceivedSslError` **not overridden** — an invalid certificate cancels the
  load. A payment SDK that proceeds through a certificate error is the single
  worst thing this plugin could ship.
- Back press = dismissal (D4), not a WebView history pop, once the payer is past
  the entry screen — a history pop would put them on a stale form.

**iOS / macOS.** `VpayCheckoutViewController` (`UIViewController`, `WKWebView`),
presented modally over the Flutter root view controller; macOS the same with an
`NSViewController` in a sheet. `WKNavigationDelegate.decidePolicyFor` is the
stop check; the modal's dismissal (interactive swipe included) is D4's
dismissal. `WKWebViewConfiguration` uses the **default persistent** data store
(D-M5, decided 2026-09-13).

**Web.** No plugin window: `window.open(url)` and the popup path that already
exists, which vpay already treats as a first-class peer. The Flutter web app's
origin must be in the merchant's `checkout_origins` for `vpay:complete` to
arrive; if it is not, the popup still works and the plugin falls back to
polling plus a `popup.closed` watch. The Dart state machine is the same one —
`vpay:complete` and "the popup closed" are just this platform's spelling of
"reached a stop URL" and "the payer left".

**Windows / Linux desktop.** Not supported in the first version. A dated ⛔ row,
not silence.

**Minimum platform versions** (D-M4, decided 2026-09-13): **Android `minSdk` 21,
iOS deployment target 12.0.** The widest reach, chosen because the low-end
Android handset is the payer this repository is actually for. The cost is
bought knowingly and is stated in "What will not be true when this ships": older
system WebViews have quirks nobody in this repository will test.

## D6 — the credential never reaches a log line

Every SDK in this repository redacts this value and each one had to be made to:
`PaymentIntentRow` and `PaymentIntentWithSecret` carry hand-written `Debug`
impls, and `@vaam-apps/vpay-stripe-js` has a test that there is no `console` call
in its shipping source at all.

Dart's equivalents, each of which is a parity row rather than a promise:

- `toString()` is overridden on every type that holds a session URL, a session
  secret or an intent secret, rendering `[N chars redacted]`. Dart's default
  `toString` is the class name, which is safe — but a `copyWith`-style data
  class with a generated `toString` is not, and generated code is how this
  regresses.
- No `print`, no `debugPrint`, no `log` anywhere in `lib/` — asserted by a test
  over the package's own source, the way stripe-js asserts its `console` rule.
- The Android extra and the iOS property holding the URL are cleared when the
  window finishes.
- The plugin refuses a non-`https` session URL unless the app explicitly opted
  into an insecure base (for the `compose.demo.yml` stack), and that opt-in is
  a named, documented parameter — not a debug-build inference.

## D7 — package layout

Published as **`vpay_checkout_flutter`** (D-M1, decided 2026-09-13). pub.dev has
no scopes, so the `@vaam-apps/*` convention cannot be reproduced; the name says
what the package is — the payer checkout surface — and not what it is not, which
is a merchant SDK. **Check availability on pub.dev before the first commit**: the
name is permanent once published.

```
sdks/flutter/
  vpay_checkout_flutter/
    lib/
      vpay_checkout_flutter.dart     — the public surface
      src/browser_client.dart        — the Dart port of stripe-js's read half
      src/checkout_controller.dart   — the state machine (pure, no plugin calls)
      src/result.dart  src/errors.dart  src/redaction.dart
      src/platform/…                 — pigeon-generated channel + web impl
    android/  ios/  macos/
    pigeons/checkout.dart
    test/                            — Dart unit tests, no device
    example/                         — a runnable app, pointed at the demo stack
```

One package, not a federated split with a `_platform_interface`: there is no
third-party implementor to accommodate, and a federated layout would make the
one thing that matters here — that all the logic is in Dart — four packages
away from the tests that prove it. Pigeon rather than a raw `MethodChannel`
because the channel carries a credential and an untyped `Map<String, dynamic>`
across three languages is where that gets mistyped.

## D8 — the external-browser mode, and the constraint that shapes it

D-M2, decided 2026-09-13: **an external-browser mode ships beside the in-app
WebView in v1**, for rails that refuse an embedded WebView. Orange is the reason
(see the risks below).

**The constraint, verified rather than assumed.** `checked_forward_url`
(`vpay-api/src/v1/checkout_sessions.rs:740`) accepts `http://` and `https://`
and **nothing else** — https alone under `deployment.livemode`. So
`success_url` and `cancel_url` **cannot be a custom scheme**, and the usual
mobile pattern (`myapp://return` as the callback) is not available without a
merchant-hosted bounce page. That single fact decides the rest of this section.

**What follows from it, and why it is good news.** Because D1 already resolves
the outcome by polling, the external mode **degrades correctly with no merchant
deployment work at all**: the payer finishes on the merchant's own `success_url`
page inside the system browser, closes it, and the plugin polls on resume and
reports the truth. Return detection is therefore a **UX upgrade, never a
correctness requirement** — which is what makes this mode cheap enough to ship
in v1 rather than a second integration contract.

Three tiers, and a merchant may stop at the first:

| Tier                                 | What the merchant does                            | What the payer sees                                                         |
| ------------------------------------ | ------------------------------------------------- | --------------------------------------------------------------------------- |
| **0 — always available**             | nothing                                           | finishes on the merchant's page, taps Done/Back; the plugin polls on resume |
| **1 — Android App Links**            | serve `assetlinks.json` on the `success_url` host | Android hands the return straight to the app; the tab closes itself         |
| **1 — iOS 17.4+ Associated Domains** | `apple-app-site-association` on the same host     | the same, through `ASWebAuthenticationSession`'s https callback             |

**No custom URL scheme, and no bounce page — recommended against.** A bounce
page at `https://shop/return` redirecting to `myapp://vpay/return` would make
tier 1 work everywhere including iOS 12, and it is the wrong trade: custom
schemes are first-come-first-served on Android, so any installed app may claim
`myapp://` and receive the return. App Links and Associated Domains are
domain-verified and cannot be claimed. This is the same hijack class the in-app
WebView avoids by construction, and it should not be reintroduced to save a
tap. Recorded here as a decision so it is not quietly made later.

**iOS: `SFSafariViewController`, not `ASWebAuthenticationSession`, below 17.4.**
With no custom scheme to call back on, `ASWebAuthenticationSession` buys nothing
below iOS 17.4 — its `callbackURLScheme` would never fire — and it costs a
system consent alert reading _"… wants to use … to sign in"_, which is the
wrong sentence on a payment and is the kind of prompt a payer abandons. So the
external mode is `SFSafariViewController` with its dismissal delegate on the
iOS 12–17.3 range, and `ASWebAuthenticationSession` with an https callback only
where tier 1 is configured and the OS supports it. **Android** is Custom Tabs
(`androidx.browser`) throughout.

**`VpayCheckoutMode.inApp` stays the default.** It is what was asked for, it
needs no merchant deployment work for a clean return, and it is the mode with
no scheme-hijack surface at all.

## D9 — what this plugin may and may not be used for, on Apple and Google

Raised by the maintainer on 2026-09-13: _"won't apple flag us? Even if we're
using a browser, technically we're still in app right?"_ Checked against the
live guidelines rather than answered from memory — the numbering had already
moved (it is **3.1.3(e)**, not the 3.1.5(a) it was for years).

**The rule keys on what is sold, not on where the payment UI lives.** In-app
`WKWebView` (D5) and external browser (D8) are treated identically. Neither is
a way around anything, and the mode choice is a UX and rail-compatibility
decision only.

**vpay's own merchants are on the permitted side, and not by a carve-out.**
Apple 3.1.3(e), verbatim:

> If your app enables people to purchase physical goods or services that will
> be consumed outside of the app, you must use purchase methods other than
> in-app purchase to collect those payments, such as Apple Pay or traditional
> credit card entry.

_Must use methods other than IAP._ A shop selling physical goods, or a service
consumed in the real world, paid with MTN MoMo or Orange Money, is required to
do exactly what this plugin does — and would be rejected for using IAP. Google
Play is the same shape from the other direction: Play Billing is only for
digital items, and physical goods and physical services are not supported by it.

**Where a merchant is flagged, and no architecture here saves them.** Apple
3.1.1 requires IAP to unlock features or functionality _within_ the app —
subscriptions, in-app credits, game levels, premium content, full-version
unlocks, and digital gift cards or vouchers redeemable for digital goods. A
vpay checkout for any of those is a rejection.

**The trap worth documenting, because it looks like an escape hatch.** D8's
external-browser mode does **not** move a digital-goods app out of 3.1.1.
Apple 3.1.1(a) goes further: outside the United States storefront, apps and
their metadata "may not include buttons, external links, or other calls to
action that direct customers to purchasing mechanisms other than in-app
purchase" without a StoreKit External Purchase Link Entitlement, which is
region-limited. So for a digital-goods merchant, opening Safari is not neutral
— it can be the violation. **The plugin's README must say this in the same
breath as it documents `externalBrowser`**, or the mode reads as a workaround
and someone will use it as one.

**What this costs the plugin: documentation, not architecture.** No code
changes. What is owed:

1. A section in `vpay_checkout_flutter`'s README — a short decision tree
   ("what are you selling?") landing on _use this plugin_ or _use IAP / Play
   Billing_, with 3.1.3(e) and 3.1.1 quoted rather than paraphrased.
2. The same in `docs/flows/mobile-checkout.md`, as a constraint on adoption.
3. The pub.dev description must not suggest unlocking in-app content.

**Bounds on the above, stated because this is the kind of claim that rots.**
It is a reading of the published guidelines on 2026-09-13, not legal advice,
and not a review outcome — Apple's reviewers decide case by case. Both stores'
rules have moved repeatedly through the Epic injunctions in the United States
and the DMA in the EU, and Google entered a revised settlement in March 2026
with alternative-billing reporting obligations starting 1 October 2026. A
merchant outside "physical goods, consumed outside the app" should check the
current text themselves. Sources read: Apple's App Store Review Guidelines
(§3.1.1, §3.1.1(a), §3.1.3(a)–(g)) and Google Play Console Help's payments
policy.

## What this means for the repository's own gates

This is the part that is easy to under-state, so it is stated first: **a parity
table nothing reads is prose, and this repository has already measured what
that costs** (`verify-sdk-parity` was doc-only until 2026-09-06, and deleting a
whole capability row was measured to pass).

1. **`docs/sdks/parity.md` gains a third table** — the Flutter plugin is a payer
   surface, like `@vaam-apps/vpay-stripe-js`, and shares no capability row with the
   merchant SDKs.
2. **`cargo xtask verify-sdk-parity` must learn Dart.** It reads Rust
   `#[test]`/`#[tokio::test]` and TypeScript `it("…")`/`test("…")` today. It
   needs Dart `test('…')` — including the `group`/`testWidgets` spellings the
   package actually uses — or the new table is unchecked. **This is in scope for
   the implementation, not a follow-up.**
3. **`just` recipes**: `install-flutter`, `analyze-flutter` (`dart analyze
--fatal-infos`), `test-flutter`.
4. **They do not join `just ci` yet** (D-M3, decided 2026-09-13). A Flutter SDK
   in the CI image and in the `vpay-ci` VM is a prerequisite, and the plugin
   should not wait on it. **The honest form of "later" is a dated ⛔ row in
   `docs/sdks/parity.md` saying the plugin's tests are run by a human and naming
   the date** — not a silent absence. `just ci`'s gate table in
   `docs/status.md` says what it runs; it must not imply this is one of them.
   Closing that gap is its own piece of work with its own owner.
5. **A pinned Flutter version**, in a file, the way `rust-toolchain.toml` pins
   Rust and CI reads the pin from the file — so that the day the gate lands, it
   lands on a version, not on whatever the image happened to have.
6. **Docs**: `docs/flows/mobile-checkout.md` (the flow), a row in
   `docs/status/merchant-sdks.md` or a page of its own, and an **ADR** — this
   adds a payer surface and a third toolchain, which is ADR-0020 shaped.

## What will not be true when this ships

Written now, at design time, so it cannot be forgotten at summary time:

- **No real rail.** Every payment a Flutter app completes will settle against
  WireMock, like every payment in this repository's history.
- **Orange's page inside a WebView is unproven, and is the biggest technical
  risk here.** Redirect rails routinely refuse embedded webviews, and vpay has
  only ever talked to a WireMock mapping serving two links. D8 is the answer if
  that turns out to be true, and D8 has never been run against a real Orange
  page either.
- **No CI gate** — see the gates section, item 4.
- **No physical device**, no Play Store or App Store review, no CI that builds
  an APK or an `.ipa`.
- **The Android 21 / iOS 12 floor is a claim nobody will test.** The support
  matrix is chosen for reach; the oldest system WebViews in it will be exercised
  by merchants, not by this repository.
- **No desktop Linux or Windows.**
- **No app built on this plugin has been through App Store or Play review.** D9
  reads the published rules; a reviewer's verdict is a different thing and this
  repository will not have one.
- **Page memory** (IndexedDB, 90 days) behaves differently per platform WebView
  and has been watched by nobody — a gap it inherits, since
  "nothing has watched a real browser's IndexedDB" is already true on the web.
- **Tier 1 return detection (App Links / Associated Domains) is merchant
  deployment work this repository cannot verify**, and no test here can prove a
  merchant configured it. Tier 0 is what the plugin's own tests will cover.

## Decisions taken, and by whom

| #    | Decision                                                                | Taken                                             |
| ---- | ----------------------------------------------------------------------- | ------------------------------------------------- |
| D1   | The outcome is polled from `/v1/browser`, never read off a URL          | design                                            |
| D2   | One pre-flight session read; stop URLs derived, not configured          | design                                            |
| D3   | No JavaScript bridge; no native peer added to the checkout page         | design                                            |
| D4   | Dismissal polls before it reports; `pending` is a first-class result    | design                                            |
| D-M1 | Published as `vpay_checkout_flutter`                                    | maintainer, 2026-09-13                            |
| D-M2 | An external-browser mode ships in v1 (D8)                               | maintainer, 2026-09-13                            |
| D-M3 | Recipes now, `just ci` gate later, with a dated ⛔                      | maintainer, 2026-09-13                            |
| D-M4 | Android `minSdk` 21, iOS 12.0                                           | maintainer, 2026-09-13                            |
| D-M5 | Persistent WebView storage, so page memory works                        | maintainer, 2026-09-13                            |
| D-M6 | The checkout page gets **no** native peer                               | design (D3), open to reversal                     |
| D9   | Store-policy bounds on adoption; a README obligation, not a code change | design, from the maintainer's question 2026-09-13 |

One decision remains genuinely open and is **not** taken here: whether
`docs/flows/mobile-checkout.md` supersedes or sits beside
`docs/flows/hosted-checkout.md`'s popup table, which already describes three
peers and would now describe a fourth shape that is not a peer at all. That is
a documentation-structure call on a page six other documents point at.
