# `@vaam-apps/vpay-tauri-checkout`

The guest-JS half of `tauri-plugin-vpay-checkout`: vpay's hosted checkout,
opened in the **payer's own browser** from a Tauri v2 app, and answered with a
typed result once the payment intent actually settles.

The same API runs **outside** Tauri. In a plain browser it falls back to a
`window.open` popup speaking the origin-and-source-pinned `vpay:complete`
protocol `@vaam-apps/vpay-stripe-js` already implements. That is what
"android + ios + web" means here: one guest-JS package, one state machine,
three hosts.

**Not on any registry, and whether it ever will be is undecided.** Nothing in
this repository publishes either half of this plugin, and the decision to do
so has not been taken — see
[ADR-0023](../../../docs/adr/0023-tauri-checkout-plugin.md) § "Left to the
maintainer", item 1. The manifests are publish-ready so that a release can
happen without editing them; that is all it means.

So the line below **does not work today** and is written down only so that
the day it does, nothing else has to change:

```bash
pnpm add @vaam-apps/vpay-tauri-checkout   # NOT PUBLISHED — this will 404
```

Until then, a consumer takes the package from a checkout of this repository:
inside the workspace as `"@vaam-apps/vpay-tauri-checkout": "workspace:*"`
(which is what `examples/tauri-checkout` does), and outside it as a
`file:`/`link:` dependency on the built package directory.

Inside this workspace it is a `workspace:*` consumer of
`@vaam-apps/vpay-stripe-js`, and that dependency must be **built** before this
package typechecks: its `exports` resolve to `dist/`. You do not have to do
that by hand — this package's `build`, `typecheck`, `test` and `lint` scripts
each chain a `deps` step that builds `@vaam-apps/vpay-stripe-js` first.

```bash
pnpm --filter @vaam-apps/vpay-tauri-checkout typecheck
```

## Getting started

1. **Your server creates the session.** `POST /v1/checkout/sessions` with
   `ui_mode: "hosted"`, a `success_url` and a `cancel_url`, using your secret
   key. Never from the app: the app holds a publishable key only.
2. **Your server hands the app `session.url`** — `{base}/c/{cs_id}?key={pk}#{cs_secret}`.
3. **The app starts checkout.** One object per deployment, one `start` per
   payment.

   ```ts
   import { VpayCheckout } from "@vaam-apps/vpay-tauri-checkout";

   const checkout = new VpayCheckout({
     baseUrl: "https://api.vpay.example",
     publishableKey: "pk_live_…",
   });

   const result = await checkout.start(sessionUrl);
   ```

4. **`start` never rejects.** Handle all five `kind`s — the union is
   discriminated, so a `switch` is exhaustive at compile time:

   ```ts
   switch (result.kind) {
     case "succeeded":  return showThankYou();
     case "failed":     return showFailure(result.code, result.providerMessage);
     case "canceled":   return showCanceled();
     case "pending":    return showStillProcessing(); // poll your own order
     case "unresolved": return showTryAgain(result.error);
   }
   ```

5. **`pending` and `unresolved` are not failures.** `pending` means the
   polling budget elapsed with the intent still moving; `unresolved` means
   this package could not observe an outcome at all. Neither says the payment
   failed, and neither may be rendered as one.
6. **Fulfil from the webhook, never from this result.** A `succeeded` here is
   a UI fact about what the payer's browser and one API read showed. Your
   server ships the goods on `payment_intent.succeeded` from its
   signature-verified webhook.

`dismiss()` closes the window if one is open, and whatever it produces is an
ordinary dismissal — it never carries an outcome of its own, and the result
still comes from D4's poll, exactly as a payer's own close does.

**Which host reports it differs, and the Platforms table below is the
authority.** On Android, iOS and in a plain browser the close is observed —
the tab closing, Done or a swipe-away, the popup's `closed` — so `dismiss()`
merely triggers what the host would have seen anyway. On **desktop there is
nothing to observe**: `dismiss()` is the only thing that ever produces the
one `dismissed` event, because handing a URL to the default browser leaves
the app no handle on the window. A desktop app that never calls `dismiss()`
waits out the poll budget and answers `pending`.

## Platforms

| Platform                      | Window                                                                                                          | Dismissal signal              |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------- | ----------------------------- |
| Android                       | partial Custom Tab requested (90 % height, adjustable, close at START) — the browser may decline it; see Status | yes — the tab closing         |
| iOS                           | `SFSafariViewController`, `.pageSheet` with a `.large()` detent                                                 | yes — Done **and** swipe-away |
| Desktop (macOS/Windows/Linux) | the payer's **default browser**                                                                                 | **none** — see below          |
| Plain browser (no Tauri)      | `window.open` popup, address bar visible                                                                        | yes — the popup's `closed`    |

The desktop row is the honest one: handing a URL to the default browser gives
the app no handle on the window, so nothing can tell it the payer closed the
tab. The desktop host sends `dismissed` only when `dismiss()` is called while
a show is in flight. It does not invent a focus-based signal.

`stopUrls` and `allowInsecureUrl` are **inapplicable** in the plain-browser
host and it says so rather than pretending: a cross-origin popup's location
cannot be read from the opener at all, and the browser's own address bar
already shows the payer whatever scheme was navigated to.

No surface here reads an outcome off a URL, off a message payload or off an
event. A host reports one of two things — `dismissed` or `stopUrlReached` —
and the result comes from polling `GET /v1/browser/payment_intents/{id}`
afterwards. That is decision D1, and it is why a reached `success_url` with a
still-processing intent answers `pending` rather than `succeeded`.

## Tauri setup (what a merchant does)

1. Add the crate to `src-tauri/Cargo.toml`. **Not by version** — the crate is
   `publish = false` and is on no registry, so
   `tauri-plugin-vpay-checkout = "0.4"` can never resolve; whether it is ever
   published is an open question
   ([ADR-0023](../../../docs/adr/0023-tauri-checkout-plugin.md) § "Left to
   the maintainer", item 1). Depend on the repository instead:

   ```toml
   [dependencies]
   tauri-plugin-vpay-checkout = { git = "https://github.com/vaam-apps/vpay", branch = "master" }
   ```

   Cargo finds the crate by walking the repository for a manifest with that
   `name`, so no subdirectory key is needed; pin with `tag = "vX.Y.Z"` or
   `rev = "<sha>"` rather than `branch` for anything you ship, because a
   payment SDK that moves under you between builds is not a dependency, it is
   a surprise.

   An in-repository consumer uses a path instead — this is what
   `examples/tauri-checkout/src-tauri/Cargo.toml` does:

   ```toml
   [dependencies]
   tauri-plugin-vpay-checkout = { path = "../../../sdks/tauri/tauri-plugin-vpay-checkout" }
   ```

2. Register it in `src-tauri/src/lib.rs`:

   ```rust
   tauri::Builder::default()
       .plugin(tauri_plugin_vpay_checkout::init())
   ```

3. Grant the permission in `src-tauri/capabilities/default.json`:

   ```json
   { "permissions": ["vpay-checkout:default"] }
   ```

Without step 3 the two commands are denied by Tauri's capability system and
`start` answers `unresolved` with `platform_window_failed` — which is the
correct answer, and a confusing one if the capability is what is missing.

## App Store and Play

Opening a payment page in the payer's own browser rather than an in-app
WebView is deliberate, and the store-policy reasoning behind it —
`SFSafariViewController` and Custom Tabs versus a `WKWebView`, and what each
store's rules actually say — is in
[`docs/flows/mobile-checkout.md`](../../../docs/flows/mobile-checkout.md).
Read it before changing which window this plugin opens.

## Errors and credentials

Two rules, and both are gated by tests in this package rather than by review:

- **No `console.*` in shipping source.** `guest-js/no-logging.test.ts` reads
  every non-test module in `guest-js/` and fails on one.
- **No error message interpolates a URL, a secret or a thrown value.** Every
  message is a fixed string; the only value any of them carries is a public
  `cs_…` session id and an HTTP status. A host rejection becomes the fixed
  `platform_window_failed` error, never the thrown value's own message — a
  host's message can quote the URL, and that URL's fragment _is_ the session
  secret.

`redacted(value)` is exported for the merchant's own logging: it renders
`[N chars redacted]`, and `null` for an absent value.

## Status

Written 2026-09-22, and **amended later the same day**, because what this
section could honestly claim changed twice in one day. Both versions are
below; the amendment is the later measurement.

### Amended 2026-09-22 — the plugin has been driven end to end, on two simulators, against a running vpay

**What was run**, by the lane that built `examples/tauri-checkout`, on one
macOS host:

- **iOS** (iPhone 17 simulator, iOS 26.3.1): **Pay** opened the
  `SFSafariViewController` on vpay's **real hosted checkout page**, an MTN
  Mobile Money push was driven through the page to `Payment received`, the
  payer closed the sheet, and `start()` resolved **`succeeded`** with the
  session and intent ids it was given. A second run opened the sheet and
  **swiped it away without paying**, and resolved **`pending`** — D4's poll
  answering exactly as designed, never a fabricated `canceled`.
- **Android** (API 36 arm64 emulator): the **Custom Tab** opened on the same
  real page, the same MTN push was driven through `Check your phone` to
  `Payment received` about 25 s later, the tab was closed, and `start()`
  resolved **`succeeded`**.
- **Cross-checked under a real merchant token.**
  `GET /v1/payment_intents/{id}` reports `succeeded` for both paid intents
  (12000 `xaf`, `last_payment_error` null) and `requires_payment_method` for
  the dismissed-unpaid one — which is what the `pending` the UI showed
  actually meant.

So, and this is the sentence the four bullets below no longer support:
`show` and `dismiss` **have** crossed the IPC boundary into the Kotlin and
the Swift at runtime, and the two reads have run against a real
`vpay-server`.

**The scope of that, stated as narrowly as it deserves.** The stack was
`ghcr.io/vaam-apps/vpay-*:edge` images, **not a build of this tree**. Its
rail is a WireMock container, so **no real MTN endpoint was called and no
money moved**. No merchant webhook was verified — no shop ran, and the
merchant-side evidence is the token-authenticated intent read rather than an
order row written by a signature-verified delivery. **No `stopUrlReached`
fired on either platform**: both successes were a manual close plus D4's
poll, which is the only way this plugin ends a checkout today. Nothing ran
on a **physical device**, on **desktop**, or against **Orange Money** or the
failure MSISDNs.

**One behavioural finding for the Platforms table above.** On that emulator
Chrome presented the Custom Tab **full-height**, not as the partial sheet
the plugin asks for — the requested bounds reach the task record and Chrome
declined them. Honouring `setInitialActivityHeightPx` is the browser's
choice, and the table's "partial" describes the request, not a guarantee.

Full evidence, with ids, verbatim screen text and every caveat:
[`docs/status/verification/2026-09-22-tauri-plugin.md`](../../../docs/status/verification/2026-09-22-tauri-plugin.md)
§ "Lane D2".

### As written earlier the same day — this package's own suite

What the lane that wrote **this package** ran, on this machine:

| Command                                                  | Result                                |
| -------------------------------------------------------- | ------------------------------------- |
| `pnpm --filter @vaam-apps/vpay-tauri-checkout typecheck` | exit 0                                |
| `pnpm --filter @vaam-apps/vpay-tauri-checkout lint`      | exit 0, `--max-warnings 0`            |
| `pnpm --filter @vaam-apps/vpay-tauri-checkout test`      | exit 0 — 71 tests, 8 files, 0 skipped |
| `pnpm --filter @vaam-apps/vpay-tauri-checkout build`     | exit 0                                |

Those are **unit tests on Node**, with `fetch`, the clock, the host and the
`Window` all injected — and that is still all they are. Specifically:

- ~~Nothing here has run against a real vpay deployment.~~ **Closed by the
  amendment above.** Still true of _this package's suite_: its two reads go
  through `@vaam-apps/vpay-stripe-js` over a scripted `fetch`, and no test
  file here opens a socket.
- ~~`tauriHost` has **not** been exercised against a real Tauri IPC.~~
  **Closed by the amendment above.** Its _tests_ still inject a fake
  `invoke`/`Channel` pair and assert the command names and the four argument
  keys; what changed is that a real app has now made those calls for real.
- `webHost` has **not** been run in a real browser. Its tests drive a
  hand-rolled `Window`; a real `window.open` popup, a real cross-origin
  `message` and a real close have **still** not been observed anywhere.
- The Rust, Android and iOS halves are other lanes' work. What they do is
  described by `docs/plans/2026-09-22-tauri-plugin-brief.md` and by the
  status pages, not by this file.
- ~~**No device or simulator run of any kind was performed.**~~ **Closed by
  the amendment above** for an iOS Simulator and an Android emulator. **No
  physical device**, still.
