# `examples/tauri-checkout`

The smallest real Tauri v2 application that consumes
[`tauri-plugin-vpay-checkout`](../../sdks/tauri/tauri-plugin-vpay-checkout/)
and its guest-JS half, `@vaam-apps/vpay-tauri-checkout`.

One screen: three fields (API base URL, publishable key, checkout session
URL), an "allow insecure base" opt-in, a **Pay** button that calls
`new VpayCheckout({ … }).start(sessionUrl)` and renders the result's `kind`
and ids, and a **Dismiss** button that closes the window. Plain DOM, no
framework, one TypeScript file.

**Its purpose is evidence.** The plugin ships a Rust crate, a Kotlin host
and a Swift host, and none of those three can be compiled by anything inside
`sdks/tauri/`: the Kotlin needs a consuming app's Gradle project, the Swift
needs a consuming app's Xcode project, and the Rust `staticlib`/`cdylib`
needs something to link it. This app is that something — and since
2026-09-22 it is also the only thing that has ever driven the plugin against
a running vpay, on an iOS Simulator and an Android emulator, to two real
`succeeded` results. What was actually run, and what was not, is in
**Status** at the bottom, ending with the addendum that records those runs —
read it before trusting anything here.

## Running it

```bash
pnpm install                       # from the repository root, once
```

### Desktop (macOS, Windows, Linux)

```bash
pnpm --filter @vpay/example-tauri-checkout exec tauri dev
```

The checkout page opens in the payer's **default browser**. The desktop host
has no dismissal signal at all — it learns nothing when the payer closes that
browser tab — so a desktop `start()` resolves only when the app itself calls
`dismiss()`, and then polls. That is a stated property of the plugin, not a
gap in this example; see the plugin's README.

### Android

```bash
export ANDROID_HOME="$HOME/Library/Android/sdk"     # or your SDK location
export NDK_HOME="$ANDROID_HOME/ndk/<version>"
export JAVA_HOME="$(/usr/libexec/java_home -v 21)"  # JDK 17+ (AGP 8.11)
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android

cd examples/tauri-checkout
pnpm exec tauri android init          # regenerates src-tauri/gen/android
pnpm exec tauri android dev           # or: android build --debug --target aarch64 --apk
```

The checkout page opens in a Custom Tab. The plugin asks for a **partial**
one (90 % height, adjustable); measured 2026-09-22 (Lane D2), Chrome on an
API 36 emulator ignored that and presented it **full-height**. The requested
bounds are in the task record; honouring them is the browser's choice.

### iOS

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

cd examples/tauri-checkout
pnpm exec tauri ios init              # regenerates src-tauri/gen/apple; needs CocoaPods
pnpm exec tauri ios dev               # or: ios build --debug --target aarch64-sim
```

The checkout page opens in a detented `SFSafariViewController`.

### A plain browser (no Tauri)

```bash
pnpm --filter @vpay/example-tauri-checkout dev     # http://localhost:1420
```

The same screen, the same code. `defaultHost()` sees it is not inside Tauri
and returns the **web host**, which opens a `window.open` popup and listens
for the origin-and-source-pinned `vpay:complete` message. `stopUrls` and the
insecure opt-in do not apply there.

## Environment variables

All three are optional prefills for the form, read by vite at **build** time
and inlined into the bundle. Put them in a `.env.local` next to this file.

| Variable                    | Fills in             |
| --------------------------- | -------------------- |
| `VITE_VPAY_BASE_URL`        | API base URL         |
| `VITE_VPAY_PUBLISHABLE_KEY` | Publishable key      |
| `VITE_VPAY_SESSION_URL`     | Checkout session URL |

> **Quote `VITE_VPAY_SESSION_URL`. Every time.** The session URL ends in
> `#<session secret>`, and vite's dotenv parser treats an **unquoted `#` as
> the start of a comment** — so an unquoted value is silently truncated at
> the `#` and the app is handed a URL with no fragment. Nothing warns you.
> Found the hard way on 2026-09-22 (Lane D2): the first iOS run against a
> real stack rendered `unresolved … invalid_request_error / invalid_request`
> with **nothing having reached `vpay-server` at all**, because the session
> secret the pre-flight needs had been eaten by a comment marker.
>
> ```bash
> # WRONG — everything from the # onwards is discarded
> VITE_VPAY_SESSION_URL=http://localhost:3080/c/cs_123?key=pk_test_x#cs_123_secret_abc
>
> # RIGHT
> VITE_VPAY_SESSION_URL="http://localhost:3080/c/cs_123?key=pk_test_x#cs_123_secret_abc"
> ```
>
> The other two variables have no `#` in them and are unaffected; quoting
> them anyway costs nothing.

A `VITE_` variable ends up in the shipped JavaScript, so nothing secret
belongs in one — and this is exactly why the paragraph above is a warning
about **truncation** and not permission to be casual. The session URL's
fragment **is** a payer credential. It is acceptable in this example's
`.env.local` because `.env.local` is gitignored, the session is a
throwaway, and the alternative is typing 67 characters into a simulator by
hand; it is **not** acceptable in anything you ship. A publishable key is
not secret; a secret key, an API token and a session secret all are, and the
session URL is minted **server-side** and handed to the app.

## Two things about the configuration that are not obvious

- **`src-tauri/tauri.conf.json`'s `version` is `0.0.1`, not `0.0.0`.** The
  Android build refuses the default outright: _"You must change the `version`
  in `tauri.conf.json`. The default value `0.0.0` is not allowed for Android
  package and must be at least `0.0.1`."_ Everything else about this example
  is version `0.0.0`, because it is a private example that is never released.
- **`app.security.csp` allows `connect-src` to `https:` and `http:`.** The
  page fetches vpay's API from whatever base URL the payer typed, so it
  cannot know the origin at build time; `http:` is there because the demo
  stack is plain HTTP. Everything else in the policy is `'self'`, and there
  is no `'unsafe-eval'`. A real merchant app should narrow `connect-src` to
  its own API origin.

## `src-tauri/gen/` is not tracked

`cargo tauri android init` and `cargo tauri ios init` write a full Gradle
project and a full Xcode project under `src-tauri/gen/`. Both carry absolute
paths to this machine (`local.properties`, `tauri.settings.gradle`, the
`.xcodeproj`'s file references), both hold build output (`jniLibs/**/*.so`,
`Externals/`), and both are reproduced in their entirety by re-running
`init`. The whole tree is therefore gitignored, and the commands above are
how you get it back. `create-tauri-app`'s own templates track the generated
projects and then ignore a dozen paths inside them; this example ignores the
tree instead, on the grounds that the only thing tracking it would add is a
diff nobody reads.

`src-tauri/Cargo.lock` **is** tracked. It is an application, and the plugin
crate deliberately does not carry one of its own.

## No unit tests

There is no `test` script here, and that is deliberate rather than an
omission. Everything worth asserting about the checkout state machine is
already asserted by the plugin's own 71 vitest cases and 19 Rust unit tests;
what this package adds is a **compile and a link on three platforms**, which
a unit test cannot express. `pnpm -r test` skips a package with no `test`
script, so nothing is silently green.

## Status — what was actually verified, and when

Measured 2026-09-22 on one macOS 26 / Apple Silicon host. Every line below is
a command that was run, with its exit code. Nothing here was inferred.

**Toolchain:** rustc 1.98.0 · Node 24.15.0 · pnpm 11.18.0 · Xcode 26.2
(Swift 6.2.3) · JDK 21.0.2 · Android SDK cmdline-tools with NDK 28.2.13676358
· AGP 8.11.0, Kotlin 1.9.25, Gradle 8.14.3 (all three generated by
`tauri android init`) · `@tauri-apps/cli` 2.11.5 · `tauri` 2.11.6.

| What                                                                           | Result                                                                                                     |
| ------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `pnpm install` (root)                                                          | exit 0                                                                                                     |
| `pnpm --filter @vpay/example-tauri-checkout typecheck`                         | exit 0                                                                                                     |
| `pnpm --filter @vpay/example-tauri-checkout lint`                              | exit 0                                                                                                     |
| `pnpm --filter @vpay/example-tauri-checkout build` (tsc + vite)                | exit 0                                                                                                     |
| `cargo build` in `src-tauri`                                                   | exit 0                                                                                                     |
| `tauri build --debug --no-bundle` (macOS aarch64)                              | exit 0                                                                                                     |
| `tauri ios init` then `tauri ios build --debug --target aarch64-sim`           | exit 0 — **BUILD SUCCEEDED**; the plugin's Swift and the Rust static library both link into the app binary |
| `tauri android init` then `tauri android build --debug --target aarch64 --apk` | exit 0 — the plugin's **Kotlin compiled** and its two Activities merged into the APK manifest              |
| App launched on the iOS Simulator (iPhone 17, iOS 26.3.1)                      | launched, renders the form                                                                                 |
| App launched on an Android emulator (`vpay_tauri`, API 36 arm64)               | launched, renders the form                                                                                 |
| **Pay** pressed on both, against an unreachable base URL                       | both rendered `unresolved … error type: api_connection_error`; see below                                   |

**Superseded in part, later the same day.** The table above was the whole of
this page's evidence for a few hours: builds, launches, and one `Pay` that
could not reach an API. It is left as it was. The addendum at the foot of
this page is what happened next — a real stack, a real hosted page and two
real `succeeded` results — and where the two disagree, the addendum is the
later measurement.

The one interaction that was driven, on both the emulator and the simulator,
was **Pay against a base URL that does not resolve**
(`https://api.vpay.invalid`, with a made-up `cs_test_laned` session URL).
Both rendered, verbatim:

```
unresolved — session cs_test_laned · intent (unknown)
error type: api_connection_error
error code: (none)

No outcome was observed. Not a failure either.
```

That is the **correct** answer and it is worth saying why: the pre-flight read
happens **before** any window opens (D2), so a pre-flight that cannot reach
the API resolves `unresolved` and never asks a host to show anything. No
Custom Tab and no `SFSafariViewController` appeared, and neither should have.
`Dismiss` with nothing open was also pressed on the simulator and answered
"Nothing open to dismiss."

~~**Not verified — no part of this was exercised against a running vpay:**~~
**Struck and replaced 2026-09-22, later the same day (Lane D2).** The list
below is left visible with its corrections rather than deleted, because
three of its six bullets were true when written and are not any more.

- ~~**No payment.** No checkout session was created, no intent confirmed, no
  rail touched, no money of any kind moved.~~ **— closed.** Three real
  sessions were minted against a running vpay and two were paid through to
  `succeeded`. The rail is still WireMock, so no money moved anywhere real,
  and that part stands.
- ~~**No successful pre-flight, and no poll at all.**~~ **— closed.** Both
  reads now happen for real, and it was the poll that produced both
  successes.
- ~~**No browser sheet was ever opened on any platform.**~~ **— closed on
  iOS and Android.** The `SFSafariViewController` and the Custom Tab both
  opened on vpay's real hosted page, and `show`/`dismiss` crossed the IPC
  boundary into the Swift and the Kotlin at runtime. **Still true for
  desktop:** `open::that_detached` has never executed.
- **No deep link, ever.** `stopUrlReached` is unreachable in practice: this
  repository serves no `assetlinks.json` and no
  `apple-app-site-association`, and on iOS the plugin has no Universal-Link
  intake at all (Tauri 2.11.6's Swift `Plugin` base class exposes no
  app-lifecycle hook for it). Every iOS checkout ends `dismissed`.
  **Unchanged, and confirmed by Lane D2:** both successful runs ended with
  the payer closing the sheet by hand and D4's poll deciding.
- **Desktop Windows and Linux were not built.** Only macOS aarch64.
- **No physical device** — a simulator and an emulator only.

## Status, addendum — driven against a running vpay, 2026-09-22 (Lane D2)

Later the same day, and it changes what this example is: it is no longer
only a compile harness. **The plugin was driven end to end on both an iOS
Simulator and an Android emulator against a real, running vpay**, and two
payments reached `succeeded`.

**The stack, and what it was not.** `postgres`, three WireMock containers,
`vpay-server`, `vpay-worker` and `vpay-checkout`, brought up with **zero
image builds** — `ghcr.io/vaam-apps/vpay-server:edge` (21.1 MB) and
`ghcr.io/vaam-apps/vpay-checkout:edge` (348 MB) pull anonymously, and a
scratchpad-only compose override set `image:` on the three so compose could
run without `--build`. **These were `edge` images from GHCR, not a build of
this tree**, which is the single biggest caveat on everything below. API on
`127.0.0.1:8080`, checkout on `127.0.0.1:3080`. `vpay-shop` and the
dashboard were **not** run — there is no published shop image — so sessions
were minted with `curl`, doing exactly what
`examples/shop/src/server/orders.ts` does: a real `private_key_jwt`
`client_credentials` exchange with `.e2e/shop-merchant/oauth-signing-key.pem`,
a real `POST /v1/payment_intents`, and a real form-encoded
`POST /v1/checkout/sessions` with `ui_mode=hosted`. Torn down with
`down -v`; Docker grew 0.2 GiB.

**iOS** (iPhone 17 simulator, iOS 26.3.1, build exit 0). Session
`cs_kde86t758d6191xvezz01p0b` / intent `pi_xpprqgpved3gh58p0yqsvs7v`, with
the insecure-base opt-in ticked because the stack is plain HTTP. **Pay**
opened the `SFSafariViewController` on the real hosted page — title bar
`localhost`, and on the page itself `Vaam Payments`, `Test mode — no money
moves on this deployment.`, `Payment` `FCFA 12,000`, `Reference:
cs_kde86t758d6191xvezz01p0b`, `Choose how you want to pay`, `MTN Mobile
Money` / `Orange Money`. MTN, `237670000000` (the shop README's "pays"
number), `Pay FCFA 12,000` → `Payment received` / `The merchant has been
told you paid FCFA 12,000.` Closing the sheet with its X gave, verbatim:

```
succeeded — session cs_kde86t758d6191xvezz01p0b · intent pi_xpprqgpved3gh58p0yqsvs7v
Fulfil from the webhook, not from this.
```

A second iOS run (`cs_a4exjc3yhx5695957shm4v8z` / `pi_v265kb7sgn48h8bky6f8mtap`)
opened the sheet and **swiped it away without paying**, and answered
`pending … The polling budget elapsed with the intent still moving. Not a
failure either.` — which is D4 doing precisely what it exists for.

**Android** (AVD `vpay_tauri`, API 36 arm64, started `-read-only`; build
exit 0). Host rewritten to `10.0.2.2`, and the checkout container recreated
with `NEXT_PUBLIC_VPAY_API_URL=http://10.0.2.2:8080` because
`frontends/apps/checkout/src/lib/env.ts` reads it at runtime. No cleartext
workaround was needed: Tauri's generated `app/build.gradle.kts` already sets
`manifestPlaceholders["usesCleartextTraffic"] = "true"` for debug. Session
`cs_kn3r3emzq917988xkt4bfn4m` / `pi_93htevhkrn5vn23ys9e2se70`. The Custom
Tab opened on `10.0.2.2:3080` (`XAF 12,000`, `Reference:
cs_kn3r3emzq917988xkt4bfn4m`); MTN → `237670000000` → `Pay XAF 12,000` →
`Check your phone` / `Approve XAF 12,000 on your handset. This page updates
on its own.` → about 25 s later `Payment received`. Closing the tab gave
`succeeded — session cs_kn3r3emzq917988xkt4bfn4m · intent
pi_93htevhkrn5vn23ys9e2se70`. logcat carried no `AndroidRuntime` entry and
no crash — and no vpay log line either, which is the Kotlin behaving as
designed (it has none).

**Two Android findings worth having.** The **first** Pay never reached the
page at all: Chrome's `FirstRunActivity` appeared because its terms had not
been accepted, and force-stopping Chrome there resolved `pending` — correct
behaviour for a window that closed without a payment, and a trap for anyone
reproducing this. It was bypassed with `/data/local/tmp/chrome-command-line`
(`--disable-fre --no-first-run --no-default-browser-check`, honoured on the
userdebug image, written inside the emulator only and discarded by
`-read-only`). And the Custom Tab rendered **full-height, not partial** —
Chrome offered "Minimize tab to return to it later", and although
`VpayCheckoutActivity` is in the task stack as the transparent trampoline
with the plugin's requested bounds recorded
(`lastNonFullscreenBounds=Rect(215, 420 - 865, 1500)`), Chrome did not
honour a partial presentation on this device. That is a **gap**, recorded in
`docs/sdks/parity.md`.

**The cross-check, and its limit.** Each intent was read back under a **real
merchant token** with `GET /v1/payment_intents/{id}`:
`pi_xpprqgpved3gh58p0yqsvs7v` → `succeeded`, 12000 `xaf`,
`last_payment_error` null; `pi_93htevhkrn5vn23ys9e2se70` → `succeeded`; and
the dismissed-unpaid `pi_v265kb7sgn48h8bky6f8mtap` → `requires_payment_method`,
which is exactly what the UI's `pending` claimed. The MTN WireMock journal
shows the real `POST /collection/token/`,
`POST /collection/v1_0/requesttopay` and
`GET /collection/v1_0/requesttopay/{ref}`. **This is a weaker merchant-side
check than the Flutter e2e suite's**, which reads the shop's own order row
written by a signature-verified webhook: no shop ran here, the webhook
receiver's journal was empty, and **no merchant webhook was verified.**

**Still not verified after all of this:** no `stopUrlReached` on any
platform (both successes were a manual close plus D4's poll);
dismiss-from-the-app while a sheet is open could not be driven on iOS, since
the `.large()` sheet covers the app's own UI and dragging the grabber
dismisses outright; Orange Money, the failure MSISDNs, desktop, physical
devices and `just demo-up` itself were all untouched; the rail is WireMock,
so no real MTN endpoint was called; and the stack was `edge` images rather
than a build of this tree.

**Side effects.** Two images pulled; the app uninstalled from the simulator
and the emulator and both shut down. Free disk went from 33 GiB to 11 GiB
over the run — this worktree's `target/` is 7.3 GiB, DerivedData 11 GiB,
`src-tauri/gen/android` build output 532 MB, plus another session's AVD.
Screenshots were kept in a scratchpad directory and are **not** committed.
