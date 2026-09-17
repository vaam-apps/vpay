# ADR-0021: A Flutter checkout plugin — a payer surface, polling never a URL

- **Status:** Accepted
- **Date:** 2026-09-13
- **Deciders:** maintainer (D-M1–D-M6, 2026-09-13); design (D1–D9,
  [`docs/plans/2026-09-13-flutter-plugin.md`](../plans/2026-09-13-flutter-plugin.md))
- **Number checked at branch time, not assumed:** `origin/master`
  (`7a607a68`) carries ADRs through `0019`, with **two** files both numbered
  `0018` (`0018-cross-tenant-admin-reads.md` and
  `0018-privacy-controls-and-evidence.md`) — `git ls-tree origin/master
docs/adr/` returns 20 entries for 19 numbers. Open PR
  [#172](https://github.com/vaam-apps/vpay/pull/172) already renumbers the
  second of those two to `0020-privacy-controls-and-evidence.md`
  (`gh pr diff 172 --name-only`), so `0020` is reserved even though it is not
  yet on `origin/master`. This ADR takes **0021**, the next number free of
  both the tree and every open PR's diff (`gh pr list --state open` returned
  exactly two open PRs on 2026-09-13; neither #172 nor #173 touches
  `docs/adr/0021-*`).

## Context

The maintainer asked, verbatim, on 2026-09-13: _"Let's develop a custom vpay
flutter plugin. We'll do it by using a new Activity (android), UIViewController
(iOS/…) and similar for web. We'll do the payment activity in the mobile
window and call the hosted page."_

Two facts already in this repository make the shape of the answer largely a
foregone conclusion rather than a fresh design:

1. **The hosted checkout page already runs top-level with no peer.**
   `/c/{id}`'s "top-level, `peer: none`" shape
   ([`hosted-checkout/page-memory-and-protocols.md`](../flows/hosted-checkout/page-memory-and-protocols.md))
   needs no change to run inside a `WebView`/`WKWebView` — it is a third shape
   already, a popup, and a native window is a fourth kind of the same thing.
2. **The intent read outlives the session.**
   `GET /v1/browser/payment_intents/{id}` authenticates on the publishable key
   and the intent's own `client_secret` alone, gated on nothing but a
   `PaymentIntentWithSecret`'s validity — not the session's `status` or its
   24-hour horizon (only `confirm` is gated on that). This is what makes
   polling for an outcome after the window closes possible at all, and the
   whole design in the plan document rests on it.

## Decision

Ship `vpay_checkout_flutter` (D-M1) as a Dart-first plugin: every decision that
determines the outcome lives in pure, unit-testable Dart with no platform
call, no HTTP client swapped for a fake and no timer of its own; the
per-platform host (Android `Activity`, iOS/macOS `UIViewController`/
`NSViewController`, web `window.open`) is deliberately "stupid" — it shows a
URL and reports "reached one of these stop URLs" or "the payer left," and
decides nothing.

1. **D1 — the outcome is polled, never read off a URL.** A navigation to
   `success_url` closes the window; only
   `GET /v1/browser/payment_intents/{id}` decides. The merchant controls
   `success_url`, and a plugin that reports success off a redirect it did not
   authenticate against the API is the exact class of defect
   `CLAUDE.md`'s "failure mode to avoid" names first — a code path that
   returns a plausible success it did not earn.
2. **D2 — one pre-flight session read; stop URLs derived, not configured.**
   `GET /v1/browser/checkout/sessions/{id}` once, before any window opens,
   buys the intent's polling credential, `success_url`/`cancel_url` (so the
   app cannot configure a stop URL its own backend disagrees with), an
   `ui_mode` check (`embedded` is refused before any window opens), and a
   fail-fast on a bad link. `{CHECKOUT_SESSION_ID}` is substituted before
   matching, and matching is scheme+host+port+path only.
3. **D3 — no JavaScript bridge, no native peer added to the page.**
   `addJavascriptInterface`/`WKScriptMessageHandler` are not used. The
   WebView loads third-party rail content by design (Orange's own page,
   top-level, inside it); a bridge reachable from whatever is currently
   loaded is a hole on every platform it would have to be re-proven safe on.
4. **D4 — dismissal polls before it reports.** A payer dismissing the sheet
   is not a cancellation; the plugin runs a short poll first and reports what
   the intent actually says, including a first-class `pending` result so the
   plugin is never forced to choose between lying and throwing.
5. **D5 — platform hosts, non-negotiable settings.** Android
   `VpayCheckoutActivity` is `android:exported="false"`;
   `onReceivedSslError`/`decidePolicyFor` never bypass certificate
   validation; `allowFileAccess`/`allowFileAccessFromFileURLs`/
   `allowUniversalAccessFromFileURLs` are all `false`; no
   `addJavascriptInterface`, no `WKScriptMessageHandler`; a persistent
   WebView data store (D-M5) so page memory (IndexedDB) behaves; `minSdk` 21,
   iOS 12.0 (D-M4, reach over safety margin, stated as a cost knowingly
   bought, not a claim the floor was tested).
6. **D6 — a session credential never reaches a log line.** `toString()` is
   overridden on every type holding a session URL, a session secret or an
   intent secret; no `print`/`debugPrint`/`log` anywhere in `lib/`, asserted
   by a test over the package's own source (the same technique
   `sdks/stripe-js` uses for its `console` rule); a non-`https` base is
   refused unless a named, documented insecure opt-in is passed.
7. **D7 — one package, `vpay_checkout_flutter` (D-M1), not federated.**
   `pigeons/checkout.dart` defines the platform interface; there is no
   third-party implementor to accommodate a federated split for.
8. **D8 — an external-browser mode ships in v1 (D-M2).** Because
   `checked_forward_url` (`vpay-api/src/v1/checkout_sessions.rs:740`) accepts
   only `http(s)://` for `success_url`/`cancel_url`, a custom URL scheme
   return is not available without a bounce page — and a bounce page is
   recommended against here, because a custom scheme is first-come-first-served
   on Android and any installed app may claim it. Because D1 already polls
   for the outcome, tier 0 (nothing configured) degrades correctly with zero
   merchant deployment work; Android App Links / iOS Associated Domains
   (tier 1, domain-verified) are a UX upgrade, never a correctness
   requirement. iOS uses `SFSafariViewController` below 17.4, not
   `ASWebAuthenticationSession` — with no custom scheme to call back on,
   `ASWebAuthenticationSession` buys nothing below 17.4 and costs a
   system-consent alert reading the wrong sentence for a payment.
9. **D9 — store-policy bounds on adoption are a documentation obligation,
   not a code change.** Apple's App Store Review Guidelines 3.1.3(e) requires
   physical-goods/real-world-service merchants to use a method _other than_
   In-App Purchase — which is what this plugin is — while 3.1.1 still flags a
   digital-goods merchant regardless of which window mode is used, and
   3.1.1(a) means the external-browser mode is not a neutral escape hatch for
   one. Both quoted verbatim, not paraphrased, in
   [`docs/flows/mobile-checkout.md`](../flows/mobile-checkout.md) and owed in
   the plugin's own README when Lane A writes it.

**Enforcement landing with this ADR (Lane B of the implementation brief):**
`cargo xtask verify-sdk-parity` reads Dart `test('…')`/`testWidgets('…')`/
`group('…')` test titles, and — the one property this whole gate exists to
get right — **does not** collect one carrying `skip: true` or a string
`skip:` reason, the same way it already drops a Rust `#[ignore]`d test or a
TypeScript `it.skip(…)`. `PARITY_SKIPPED_DIRS` gained `.dart_tool` and
`build`. `just install-flutter`/`analyze-flutter`/`test-flutter` exist,
refuse clearly when `sdks/flutter/vpay_checkout_flutter/` does not exist
(true on this branch), and are **not** in `just ci` (D-M3) — a Flutter SDK in
the CI image and the `vpay-ci` VM is a prerequisite this repository does not
have yet. `flutter-toolchain.toml` pins the version the day the gate lands,
the way `rust-toolchain.toml` pins the compiler.

## Alternatives considered

- **Read the outcome off the redirect URL, like a bare mobile-web
  integration would.** Rejected by D1: the merchant controls that URL, and it
  collapses the one property that makes this plugin worth building instead of
  telling every merchant to open a browser tab themselves.
- **A native SDK per platform (Kotlin, Swift) with the state machine
  duplicated in each.** Rejected: the state machine is the part with the
  correctness properties (D1, D4), and three copies of it is three chances
  for one platform's port to regress a property none of the others do. One
  Dart implementation, tested without a device, is D7's whole reasoning.
- **`ASWebAuthenticationSession` unconditionally on iOS for the
  external-browser mode.** Rejected by D8: with no custom scheme, it cannot
  fire below iOS 17.4, and the consent alert it shows even where it can fire
  is the wrong sentence for a payment.
- **A bounce page (`https://shop/return` → `myapp://vpay/return`) to make
  App-Links-style return detection work everywhere, including pre-17.4 iOS.**
  Rejected by D8: a custom scheme is claimable by any installed app on
  Android — the exact hijack class the in-app WebView already avoids by
  construction — and reintroducing it to save a tap on the return trip was
  judged not worth it.
- **A federated plugin (`vpay_checkout_flutter_platform_interface` +
  per-platform packages).** Rejected by D7: no third-party implementor exists
  to accommodate, and it would put the tests that prove the logic four
  packages away from the logic itself.

## Consequences

A merchant integrates a native payer window with the same server-side call
they already make for the web integration (`POST /v1/checkout/sessions`) and
no new route. The plugin adds a third toolchain (Dart/Flutter) to a repository
that has had exactly two (Rust, TypeScript) since its first commit, and a
payer surface that, like `sdks/stripe-js`, ships no merchant-facing capability
of its own and so shares no row with the merchant SDK parity tables.

**What this does not yet buy**, stated because ADR-0015's own gate has already
measured what an unverified parity claim costs (a whole capability row was
once deleted and passed): no platform host exists in this repository as of
this ADR (Lane C), no Dart source exists at all yet (Lane A), the plugin's own
parity table and its dated ⛔ rows are Lane A's to add, and nothing in this
plugin has ever been driven against a real MTN or Orange endpoint, a real
device, or a store review process — every payment it completes today would
settle against WireMock, exactly like every other payment in this
repository's history.

**Status of that paragraph, 2026-09-14.** It was written before any lane
started and two of its four clauses have since stopped being true, so they
are corrected here rather than left to read as current. The Dart source
exists (`sdks/flutter/vpay_checkout_flutter/lib/`), and so do the Android and
web hosts — both compiled, neither ever opened on a device or in a browser.
The parity table and its dated ⛔ rows exist. What has **not** changed: iOS
and macOS are **compiled by nobody** (no `xcodebuild` on this repository's
host), none of the plugin's tests are in `just ci` (D-M3), and nothing here
has been driven against a real MTN or Orange endpoint, a real device, or a
store review. The decisions above are unchanged; only this paragraph's
account of what exists is.
[`../status/verification/2026-09-14-flutter-review.md`](../status/verification/2026-09-14-flutter-review.md)

**Narrowed again, D8 itself (same day, later pass).** The paragraph above
said `VpayCheckoutMode.externalBrowser` "is designed and unbuilt — the API
throws `UnimplementedError` for it." That is no longer the whole truth:
`pigeons/checkout.dart`'s `ShowCheckoutRequest` now carries a `mode` field,
threaded end to end. Android's host launches a real Custom Tab
(`androidx.browser`) and reports a dismissal the moment the host `Activity`
itself resumes (D8's tier 0 — no custom scheme, D1 makes it
correctness-complete via the poll); that host compiles for real
(`flutter build apk --debug`/`--release` on `example/`). iOS's host wraps
`SFSafariViewController`, not `ASWebAuthenticationSession` — this ADR's
own D8 reasoning explains why — and macOS's opens the payer's default
browser via `NSWorkspace`, the maintainer's own call, recorded in that
file's header. Both are **compiled by nobody**, unchanged. Not built by
this pass: D8's tier 1 (Android App Links / iOS 17.4+ Associated Domains),
left as a dated gap because it needs a merchant-hosted deployment this
repository cannot provide; an emulator run proving Custom Tabs actually
opens (attempted — see `docs/sdks/parity.md`'s dated row for the outcome);
and `just ci` still does not run any of this (D-M3, unchanged).
carries the evidence.

**D5 revised again, 2026-09-16, later the same day: the in-app `WebView` is
gone — on every platform, not only behind an opt-in mode.** Both paragraphs
above described a `WebView`-based window with `externalBrowser` as a second,
selectable surface. That is no longer the design. `VpayCheckoutHostApi.show`
now opens the payer's own browser unconditionally — there is no `mode`
argument left to select anything with. `VpayCheckoutMode` and the wire
`CheckoutWindowMode` are **deleted**, not deprecated: `VpayCheckout.start`
takes a session URL and nothing else, and every platform host reached
through `VpayCheckoutPlatform.show` behaves identically to what D8 above
called `externalBrowser`. `VpayCheckoutViewController.swift` (iOS/macOS) is
deleted outright, and `VpayCheckoutActivity.kt` no longer constructs a
`WebView` of any kind.

**Why, stated as the security reasoning this decision rests on, not just
recorded as a fact.** A `WebView` renders vpay's payment form inside the
merchant app's own process. In that process, `evaluateJavascript`, the
cookie store and a navigation delegate are all reachable to code the
merchant controls — which means a compromised merchant app could read the
payer's PAN and OTP directly out of the page, undetectably, with no signal
to the payer or to vpay that it happened. A separate browser process cannot
be inspected that way from the host app at all, and the payer additionally
gets a real, checkable URL bar — something a `WebView` never offered and a
phished payer has no way to demand. This is a strictly stronger security
property than D3 (no JavaScript bridge) alone ever bought: D3 kept the
*plugin's own* code off the page; this removes the page from the merchant's
process altogether.

**The cost, stated rather than buried.** With the page in a separate
process, no host on any platform can observe a navigation any more —
D2/D2's stop-URL matching, built for a `WebView`'s navigation delegate, has
nothing left to watch. `CheckoutWindowOutcome.stopUrlReached` is redefined:
it now means an **incoming deep link** (an Android App Link or an
iOS/macOS Universal Link) matched one of the session's stop URLs, not an
intercepted navigation. That signal is **unverified on every platform** as
of this revision — a real App Link/Universal Link needs an HTTPS origin
serving `assetlinks.json`/`apple-app-site-association` for the merchant's
own `success_url` host, which this repository can neither deploy nor prove
against. Every checkout today therefore ends as `dismissed` in practice,
never `stopUrlReached` — D1 and D4 are what make that correctness-complete
regardless: the poll decides, never the window. macOS loses even the weak
signal it had: the previous code reported `dismissed` the moment the app
regained focus, which was never real evidence the browser had actually
closed (a payer could simply switch back to check something and switch
away again). That fake signal is removed rather than kept for appearances,
so macOS now has **no dismissal signal at all** unless a Universal Link
arrives or the merchant calls `dismiss()` itself — D4's poll never starts
there otherwise. This is the same trade-off D8's tier 0/tier 1 split above
already named for the external-browser mode; it now applies to the only
mode there is.

**What each platform host does now.** Android launches a **partial**
(bottom-sheet) Custom Tab via `CustomTabsIntent.setInitialActivityHeightPx`
— not a full-screen Custom Tab — continuing the "not a full-screen takeover"
shape the modal-sheet revision above already established for the WebView it
has since replaced. `VpayCheckoutActivity` stays `android:exported="false"`;
a new, narrow, exported `VpayCheckoutAppLinkActivity` exists solely to
forward an incoming App Link into the plugin. The plugin itself ships **no**
`<intent-filter>` of its own: a hostless `https` filter would claim every
`https` URL on the device — harmful on API 21-30, where it can hijack links
meant for other apps, and merely inert on 31+, where Android's own verified
App Links require a host anyway. The merchant instead declares an
`<intent-filter>` for their own verified `success_url`/`cancel_url` host,
merged into the manifest via `tools:node="merge"`. iOS wraps
`SFSafariViewController` with an explicit `.large()` detent (unchanged
choice from D8 above, now unconditional rather than mode-gated). macOS
calls `NSWorkspace.shared.open(url)` and has lost its dismissal signal, as
described above.

**Lost coverage, named rather than left implicit.** The debug-only
JavaScript-injection harness (`evaluateJavascriptForTests`,
`VpayCheckoutActivityTestHarness.kt`) and `checkout_window_test.dart`, the
one suite that drove a full MTN MoMo push through the real hosted checkout
page rendered inside a real `WebView`, are both deleted along with the
`WebView` they depended on. No suite in this repository drives a full MTN
push through the payer's real hosted checkout page end to end on Android
any more.

**Verified, and precisely how far.** For the first time, this plugin was
run — not merely compiled — on a real iOS Simulator (iPhone 17 Pro, iOS
26.5): the `SFSafariViewController` sheet renders with Safari's own address
bar; tapping through to `success_url` leaves the sheet open, since no
navigation interception exists any more; tapping Done reports `dismissed`,
D4 polls, and the checkout resolves `VpayCheckoutSucceeded`. The Dart suite
stays 80 passed / 0 skipped, and `dart analyze --fatal-infos` is clean.
Android and `example/` **compile** (`BUILD SUCCESSFUL`) but were **not**
run on an emulator against this architecture — the Lane E/D8 emulator
evidence earlier in this ADR predates the cutover and does not carry
forward; citing it as current proof would be the exact failure mode
`CLAUDE.md` warns against. macOS was not exercised at all in this pass.
Evidence:
[`../status/verification/2026-09-16-flutter-browser-cutover.md`](../status/verification/2026-09-16-flutter-browser-cutover.md).

**Reversible, at the cost this repository always pays for a reversal: a new
ADR, not an edit to this one.** The clearest candidate is D3 (no native peer
on the checkout page) — the design doc calls it "open to reversal" if a
genuine need for the page to speak to a native host arrives; that would be
its own decision, with its own origin-gating rule, not a quiet extension of
this one.

**2026-09-17 — issue #189 lane 2, the native checkout sheet: two decisions
recorded, not silently dropped.** First, `frame.ts`/`origins.ts`/`csp.ts`/
`entry.ts` — the hosted page's D4/D8 iframe-embedding refusal and the
parent `postMessage` protocol — have **no native analogue** in
`VpayCheckoutSheet` and are not ported: a Flutter widget tree has no iframe
to be embedded in, no parent frame to police an origin against, and no
`postMessage` channel to gate. Those files continue to gate a real security
property for the hosted page this ADR's D3/D4 already cover; the sheet is a
different surface with a different threat model (a merchant's own app
process, not an arbitrary embedding site), and its own boundary is D6
(no session credential in a log, a generated `toString`, or anything that
outlives the sheet) plus D3's continued refusal of a native-to-page bridge
— both already stated above, both unchanged by the sheet's existence.
Second, **cards remain out of scope**, structurally rather than by
convention: `models.dart`'s `RailFieldKind.fromJson` decodes any field type
it does not recognise — `"card"` included — to `RailFieldKindUnknown`, and
`rails.dart`'s `railChoices` (Lane 1, #189) already refuses to render a rail
carrying one (D9). A native PAN field would move the integration from PCI
SAQ-A to SAQ-D; nothing added in Lane 2 changes that boundary, and nothing
in the sheet's own code path can represent a card field even by mistake.
See [`../flows/mobile-checkout.md`](../flows/mobile-checkout.md)'s own
2026-09-17 addition for the walk this rests on.

## See also

- [`docs/plans/2026-09-13-flutter-plugin.md`](../plans/2026-09-13-flutter-plugin.md)
  — the design, with every decision's full reasoning and the risks section
  ("What will not be true when this ships").
- [`docs/plans/2026-09-13-flutter-plugin-brief.md`](../plans/2026-09-13-flutter-plugin-brief.md)
  — the three-lane implementation brief this ADR's enforcement piece (Lane B)
  was built against.
- [`docs/flows/mobile-checkout.md`](../flows/mobile-checkout.md) — the
  process this ADR's decisions describe, including the Apple/Google
  guideline quotes in full and the one open documentation-structure question
  this ADR does not decide (whether this flow supersedes, sits beside, or
  extends `hosted-checkout/page-memory-and-protocols.md`'s three-peer table).
- [ADR-0015](0015-sdk-parity.md) — the parity-matrix rule this plugin's future
  table (Lane A) and this ADR's gate extension (Lane B) both serve.
